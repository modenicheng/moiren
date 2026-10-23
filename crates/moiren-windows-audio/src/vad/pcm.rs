//! Direct IO belongs to a PCM owner thread, never Engine render.

use super::{
    Capabilities, ClientError, EndpointSnapshot, IOCTL_READ_PCM, IOCTL_WRITE_PCM, Intent,
    MAX_PCM_BYTES, PCM_HEADER_SIZE, PcmPacketStamp, PcmShape, PcmTicket, ProtocolError,
    REQUEST_SLOTS,
    client::deadline_after,
    control_io::{PcmLease, raw, wait_milliseconds},
    pcm_slots::{SlotBook, SlotError},
};
use std::{
    cell::UnsafeCell,
    os::windows::io::{FromRawHandle, OwnedHandle},
    ptr::{addr_of, addr_of_mut},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{
        ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_SEM_TIMEOUT, WAIT_FAILED, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    System::{
        IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
        Performance::QueryPerformanceCounter,
        Threading::{CreateEventW, ResetEvent, WaitForSingleObject},
    },
};
use windows::core::HRESULT;

struct Storage {
    header: [u8; PCM_HEADER_SIZE],
    direct: [u8; PCM_HEADER_SIZE + MAX_PCM_BYTES],
    overlapped: OVERLAPPED,
    returned: u32,
}

struct Slot {
    // The kernel can write while Rust operates on other slots or control.
    // All request memory has a stable address, and is only read after completion.
    storage: Box<UnsafeCell<Storage>>,
    event: OwnedHandle,
    pending: bool,
    stamp: PcmPacketStamp,
}

// SAFETY: moving a Slot preserves its boxed request address. Access is exclusive
// to its owner thread (&mut PcmOwner); pending kernel mutation uses UnsafeCell.
// Slot/PcmOwner are deliberately not Sync.
unsafe impl Send for Slot {}

impl Slot {
    fn new() -> Result<Self, ClientError> {
        // SAFETY: unnamed manual-reset event, no inheritable security attributes.
        let event = unsafe { CreateEventW(None, true, false, None) }
            .map_err(|e| ClientError::windows("CreateEventW(VAD PCM)", e))?;
        // SAFETY: transfer this successful event handle exactly once.
        let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
        Ok(Self {
            storage: Box::new(UnsafeCell::new(Storage {
                header: [0; PCM_HEADER_SIZE],
                direct: [0; PCM_HEADER_SIZE + MAX_PCM_BYTES],
                overlapped: OVERLAPPED {
                    hEvent: raw(&event),
                    ..Default::default()
                },
                returned: 0,
            })),
            event,
            pending: false,
            stamp: PcmPacketStamp {
                sequence: 0,
                qpc: 0,
            },
        })
    }
}

/// A completed render packet borrows reusable storage only until the next
/// mutable operation on the owner. Capture completion reports PCM byte count.
#[derive(Debug)]
pub enum PcmCompletion<'a> {
    Render {
        stamp: PcmPacketStamp,
        pcm: &'a [u8],
    },
    Capture {
        stamp: PcmPacketStamp,
        bytes: usize,
    },
    /// The driver intentionally discarded a late microphone packet. This is
    /// bounded backpressure, not a dead device or an uncertain request.
    CaptureDropped {
        stamp: PcmPacketStamp,
    },
}

/// Four preallocated overlapped requests for one endpoint and native shape.
///
/// Construct on control, move to a dedicated IO owner if desired. No steady
/// submission/completion allocation, Engine callback, PCM ring or DSP is added.
/// The lease retains the original control handle and a non-inheritable same-file
/// duplicate. Drop cancels all requests then drains before reclaiming any memory;
/// that final safety drain can block if a driver fails to complete cancellation.
pub struct PcmOwner {
    lease: PcmLease,
    slots: [Slot; REQUEST_SLOTS],
    book: SlotBook,
    capabilities: Capabilities,
    shape: PcmShape,
    poisoned: bool,
}

impl PcmOwner {
    pub(super) fn new(
        lease: PcmLease,
        capabilities: Capabilities,
        shape: PcmShape,
    ) -> Result<Self, ClientError> {
        Ok(Self {
            lease,
            slots: [Slot::new()?, Slot::new()?, Slot::new()?, Slot::new()?],
            book: SlotBook::new().map_err(slot_error)?,
            capabilities,
            shape,
            poisoned: false,
        })
    }

    pub const fn shape(&self) -> PcmShape {
        self.shape
    }
    pub fn pending(&self) -> usize {
        self.book.pending()
    }

    /// Same endpoint only, after outstanding IO has completed or been drained.
    /// Refresh from QUERY after native reopen/resume or gate close/open.
    pub fn rebind(&mut self, snapshot: EndpointSnapshot) -> Result<(), ClientError> {
        let previous = self.shape.snapshot();
        snapshot.require_progress_after(previous)?;
        let shape = snapshot.pcm_shape(self.capabilities)?;
        self.book.rebind().map_err(slot_error)?;
        self.shape = shape;
        self.poisoned = false;
        Ok(())
    }

    /// Playback endpoint: render PCM moves kernel -> user via OUT_DIRECT.
    pub fn submit_read(&mut self) -> Result<PcmTicket, ClientError> {
        self.submit(None)
    }

    /// Microphone endpoint: one exact native packet moves user -> kernel via
    /// IN_DIRECT. The 80-byte buffered header is separate from this PCM buffer.
    /// QPC is sampled here, immediately before IO submission, not by Engine.
    pub fn submit_write(&mut self, pcm: &[u8]) -> Result<PcmTicket, ClientError> {
        self.submit(Some(pcm))
    }

    fn submit(&mut self, pcm: Option<&[u8]>) -> Result<PcmTicket, ClientError> {
        if self.poisoned {
            return Err(ClientError::SessionUncertain);
        }
        let writing = pcm.is_some();
        if writing != (self.shape.snapshot().intent() == Intent::Microphone) {
            return Err(
                ProtocolError::Inconsistent("PCM IO direction does not match endpoint").into(),
            );
        }
        if let Some(pcm) = pcm
            && pcm.len() != self.shape.pcm_bytes()
        {
            return Err(ProtocolError::BufferSize {
                expected: self.shape.pcm_bytes(),
                actual: pcm.len(),
            }
            .into());
        }
        let ticket = self.book.reserve().map_err(slot_error)?;
        let result = self.submit_slot(ticket, pcm);
        if result.is_err() {
            // Only synchronous submission failures reach here, no kernel owner.
            self.book.finish(ticket).expect("reserved ticket");
        }
        result.map(|()| ticket)
    }

    fn submit_slot(&mut self, ticket: PcmTicket, pcm: Option<&[u8]>) -> Result<(), ClientError> {
        let slot = &mut self.slots[ticket.slot];
        // SAFETY: this slot is free, no request can own its event/storage.
        unsafe { ResetEvent(raw(&slot.event)) }
            .map_err(|e| ClientError::windows("ResetEvent(VAD PCM)", e))?;
        let sequence = if pcm.is_some() {
            self.lease.next_sequence()?
        } else {
            0
        };
        let qpc = if pcm.is_some() { current_qpc()? } else { 0 };
        slot.stamp = PcmPacketStamp { sequence, qpc };
        let storage = slot.storage.get();
        // SAFETY: the book reserved a previously free slot. Nothing can observe
        // this memory asynchronously until DeviceIoControl is submitted below.
        unsafe {
            (*storage).header = self.shape.header(sequence, qpc);
            (*storage).overlapped = OVERLAPPED {
                hEvent: raw(&slot.event),
                ..Default::default()
            };
            (*storage).returned = 0;
            if let Some(pcm) = pcm {
                (&mut (*storage).direct)[..pcm.len()].copy_from_slice(pcm);
            }
        }
        let output_bytes = if pcm.is_some() {
            self.shape.pcm_bytes()
        } else {
            self.shape.read_bytes()
        };
        // SAFETY: both pointers designate fixed boxed storage. Direct IO uses
        // lpOutBuffer even for IN_DIRECT. No Rust references to the direct bytes
        // are exposed until GetOverlappedResult has established terminal status.
        let submitted = unsafe {
            DeviceIoControl(
                raw(&self.lease.handle),
                if pcm.is_some() {
                    IOCTL_WRITE_PCM
                } else {
                    IOCTL_READ_PCM
                },
                Some(addr_of!((*storage).header).cast()),
                PCM_HEADER_SIZE as u32,
                Some(addr_of_mut!((*storage).direct).cast()),
                output_bytes as u32,
                Some(addr_of_mut!((*storage).returned)),
                Some(addr_of_mut!((*storage).overlapped)),
            )
        };
        if let Err(error) = submitted
            && error.code() != ERROR_IO_PENDING.to_hresult()
        {
            return Err(ClientError::windows("DeviceIoControl(VAD PCM)", error));
        }
        // Immediate completion also needs collection before slot reuse.
        slot.pending = true;
        Ok(())
    }

    pub fn try_complete(
        &mut self,
        ticket: PcmTicket,
    ) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        self.collect(ticket, None)
    }

    /// Wait for activity without collecting or cancelling the request. A normal
    /// timeout only lets the IO worker recheck cancellation and reset epochs.
    /// Unlike `complete`, this never turns a polling timeout into cancellation.
    pub fn wait_activity(
        &mut self,
        ticket: PcmTicket,
        timeout: Duration,
    ) -> Result<(), ClientError> {
        self.book.check(ticket).map_err(slot_error)?;
        wait_event(&self.slots[ticket.slot].event, timeout)
    }

    /// Timeout cancels this request, retains its storage, and leaves the ticket
    /// outstanding until completion is collected or shutdown drains it.
    pub fn complete(
        &mut self,
        ticket: PcmTicket,
        timeout: Duration,
    ) -> Result<PcmCompletion<'_>, ClientError> {
        let deadline = deadline_after(timeout)?;
        self.collect(ticket, Some(deadline))?
            .ok_or(ClientError::DeadlineExceeded)
    }

    fn collect(
        &mut self,
        ticket: PcmTicket,
        deadline: Option<Instant>,
    ) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        self.book.check(ticket).map_err(slot_error)?;
        let slot = &mut self.slots[ticket.slot];
        let writing = self.shape.snapshot().intent() == Intent::Microphone;
        let storage = slot.storage.get();
        let mut returned = 0;
        // SAFETY: pending storage stays boxed and its OVERLAPPED/event remain
        // live. This call observes completion; direct storage is not read yet.
        let result = unsafe {
            if let Some(deadline) = deadline {
                let milliseconds = wait_milliseconds(deadline, Instant::now()).unwrap_or(0);
                GetOverlappedResultEx(
                    raw(&self.lease.handle),
                    addr_of!((*storage).overlapped),
                    &mut returned,
                    milliseconds,
                    false,
                )
            } else {
                GetOverlappedResult(
                    raw(&self.lease.handle),
                    addr_of!((*storage).overlapped),
                    &mut returned,
                    false,
                )
            }
        };
        if let Err(error) = result {
            if deadline.is_none() && error.code() == ERROR_IO_INCOMPLETE.to_hresult() {
                return Ok(None);
            }
            if error.code() == HRESULT::from_win32(WAIT_TIMEOUT.0)
                || error.code() == ERROR_IO_INCOMPLETE.to_hresult()
            {
                // SAFETY: targets this live request; cancellation is not a drain.
                let _ = unsafe {
                    CancelIoEx(
                        raw(&self.lease.handle),
                        Some(addr_of!((*storage).overlapped)),
                    )
                };
                return Err(ClientError::DeadlineExceeded);
            }
            slot.pending = false;
            self.book.finish(ticket).map_err(slot_error)?;
            if writing_completion_timeout(error.code(), writing) {
                return Ok(Some(PcmCompletion::CaptureDropped { stamp: slot.stamp }));
            }
            return Err(ClientError::windows("GetOverlappedResult(VAD PCM)", error));
        }
        slot.pending = false;
        self.book.finish(ticket).map_err(slot_error)?;
        let expected = if writing {
            self.shape.pcm_bytes()
        } else {
            self.shape.read_bytes()
        };
        if returned as usize != expected {
            self.poisoned = true;
            return Err(ProtocolError::BufferSize {
                expected,
                actual: returned as usize,
            }
            .into());
        }
        if writing {
            return Ok(Some(PcmCompletion::Capture {
                stamp: slot.stamp,
                bytes: expected,
            }));
        }
        // SAFETY: successful completion relinquished kernel ownership. Borrowing
        // the owner through this result prevents resubmission while PCM is read.
        let direct = unsafe { &(*storage).direct };
        let stamp = match PcmPacketStamp::decode_read(&direct[..PCM_HEADER_SIZE], self.shape) {
            Ok(stamp) => stamp,
            Err(error) => {
                self.poisoned = true;
                return Err(error.into());
            }
        };
        Ok(Some(PcmCompletion::Render {
            stamp,
            pcm: &direct[PCM_HEADER_SIZE..expected],
        }))
    }

    /// Cancel all four slots first, then collect within one overall deadline.
    /// A timeout retains pending memory. Drop always performs the final drain.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), ClientError> {
        let deadline = deadline_after(timeout)?;
        self.cancel_all();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if !slot.pending {
                continue;
            }
            let milliseconds =
                wait_milliseconds(deadline, Instant::now()).ok_or(ClientError::DeadlineExceeded)?;
            let storage = slot.storage.get();
            let mut returned = 0;
            // SAFETY: no storage is reclaimed until terminal completion.
            let result = unsafe {
                GetOverlappedResultEx(
                    raw(&self.lease.handle),
                    addr_of!((*storage).overlapped),
                    &mut returned,
                    milliseconds,
                    false,
                )
            };
            if let Err(error) = result
                && (error.code() == ERROR_IO_INCOMPLETE.to_hresult()
                    || error.code() == HRESULT::from_win32(WAIT_TIMEOUT.0))
            {
                return Err(ClientError::DeadlineExceeded);
            }
            slot.pending = false;
            self.book.retire_slot(index);
        }
        // All collected tickets are retired without resetting write sequence.
        // Preserve serials too: stale tickets cannot alias a fresh request.
        Ok(())
    }

    /// Cancel every outstanding request and collect what the driver completes
    /// within `quick`. Returns true when everything drained. Requests the
    /// driver still holds move the whole owner into `residual` — a pure move,
    /// no allocation — for the worker to drain opportunistically between pump
    /// iterations.
    ///
    /// A driver may hold cancelled overlapped requests through its own gate
    /// cycle: on the 2026-10-23 night run both transport workers froze for
    /// the full `PCM_SHUTDOWN_TIMEOUT` this way during a gate-epoch recovery,
    /// so a pump thread must never wait for the final drain. Retiring also
    /// releases the session's single lease slot immediately, letting the
    /// worker prepare a replacement owner while the residual drains.
    pub fn retire(mut self, quick: Duration, residual: &mut Option<ResidualPcm>) -> bool {
        if self.shutdown(quick).is_ok() {
            // Nothing is pending; the Drop drain is instant.
            return true;
        }
        self.lease.release_endpoint_slot();
        *residual = Some(ResidualPcm { owner: self });
        false
    }

    fn cancel_all(&mut self) {
        self.poisoned = true;
        for slot in &mut self.slots {
            if slot.pending {
                // SAFETY: each OVERLAPPED remains live until the following drain.
                let _ = unsafe {
                    CancelIoEx(
                        raw(&self.lease.handle),
                        Some(addr_of!((*slot.storage.get()).overlapped)),
                    )
                };
            }
        }
    }
}

fn wait_event(event: &OwnedHandle, timeout: Duration) -> Result<(), ClientError> {
    let milliseconds = timeout.as_nanos().div_ceil(1_000_000);
    // Never turn a finite worker wait into Windows' INFINITE sentinel.
    let milliseconds = u32::try_from(milliseconds)
        .ok()
        .filter(|&value| value < u32::MAX)
        .ok_or(ClientError::InvalidDeadline)?;
    // SAFETY: the pending slot retains this stable event for the entire wait.
    // No request storage is accessed, collected, cancelled, or reclaimed here.
    let result = unsafe { WaitForSingleObject(raw(event), milliseconds) };
    match result {
        WAIT_OBJECT_0 | WAIT_TIMEOUT => Ok(()),
        WAIT_FAILED => Err(ClientError::windows(
            "WaitForSingleObject(VAD PCM)",
            windows::core::Error::from_thread(),
        )),
        _ => Err(ProtocolError::Inconsistent("unexpected PCM event wait result").into()),
    }
}

fn writing_completion_timeout(code: HRESULT, writing: bool) -> bool {
    // GetOverlappedResult reports GetLastError through HRESULT_FROM_WIN32,
    // after Windows translates the driver's intentional STATUS_IO_TIMEOUT.
    writing && code == ERROR_SEM_TIMEOUT.to_hresult()
}

impl Drop for PcmOwner {
    fn drop(&mut self) {
        self.cancel_all();
        for slot in &mut self.slots {
            if !slot.pending {
                continue;
            }
            let mut returned = 0;
            // SAFETY: final blocking control-side drain before any slot, event,
            // direct buffer, duplicate or retained original handle can be freed.
            let _ = unsafe {
                GetOverlappedResult(
                    raw(&self.lease.handle),
                    addr_of!((*slot.storage.get()).overlapped),
                    &mut returned,
                    true,
                )
            };
            slot.pending = false;
        }
    }
}

/// A retired [`PcmOwner`] whose cancelled requests the driver had not
/// completed by the retire bound. The session lease slot is already released;
/// only the duplicate handle and the still-pending slots remain. Ownership
/// stays with the pumping worker, parked in preallocated slots and collected
/// by non-blocking probes between iterations — never a second thread, lock,
/// or heap allocation.
pub struct ResidualPcm {
    owner: PcmOwner,
}

impl ResidualPcm {
    /// Requests the driver still holds. Diagnostics only.
    pub fn pending(&self) -> usize {
        self.owner.pending()
    }

    /// Collect completions until `timeout` elapses; `Duration::ZERO` is a pure
    /// non-blocking probe. Returns whether every cancelled request reached
    /// terminal status. A false return leaves the residual valid for a later
    /// retry; Drop performs the final drain.
    pub fn drain(&mut self, timeout: Duration) -> bool {
        self.owner.shutdown(timeout).is_ok()
    }
}

fn current_qpc() -> Result<u64, ClientError> {
    let mut qpc = 0;
    // SAFETY: valid writable i64; no IO or Engine thread involvement.
    unsafe { QueryPerformanceCounter(&mut qpc) }
        .map_err(|e| ClientError::windows("QueryPerformanceCounter(VAD PCM)", e))?;
    if qpc <= 0 {
        return Err(ProtocolError::Inconsistent("nonpositive QPC").into());
    }
    Ok(qpc as u64)
}
fn slot_error(error: SlotError) -> ClientError {
    match error {
        SlotError::Full => ClientError::NoFreePcmSlot,
        SlotError::Stale => ClientError::InvalidPcmTicket,
        SlotError::Exhausted => ClientError::SequenceExhausted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{
        ERROR_OPERATION_ABORTED, RtlNtStatusToDosError, STATUS_IO_TIMEOUT,
    };
    use windows::Win32::System::Threading::SetEvent;

    #[test]
    fn expired_capture_packet_uses_the_windows_completion_error_domain() {
        // SAFETY: pure OS status translation; no device or request is opened.
        let translated = unsafe { RtlNtStatusToDosError(STATUS_IO_TIMEOUT) };
        assert_eq!(translated, ERROR_SEM_TIMEOUT.0);
        let completion = HRESULT::from_win32(translated);
        assert!(writing_completion_timeout(completion, true));
        assert!(!writing_completion_timeout(completion, false));
        assert!(!writing_completion_timeout(
            ERROR_OPERATION_ABORTED.to_hresult(),
            true
        ));
        assert!(!writing_completion_timeout(
            HRESULT::from_win32(WAIT_TIMEOUT.0),
            true
        ));
    }

    #[test]
    fn activity_wait_timeout_does_not_cancel_or_modify_request_storage() {
        let slot = Slot::new().unwrap();
        wait_event(&slot.event, Duration::ZERO).unwrap();
        // SAFETY: this test owns the event; no kernel IO uses this slot.
        unsafe { SetEvent(raw(&slot.event)) }.unwrap();
        wait_event(&slot.event, Duration::from_millis(10)).unwrap();
        wait_event(&slot.event, Duration::ZERO).unwrap();
        assert!(!slot.pending);
        assert_eq!(slot.stamp.sequence, 0);
    }

    #[test]
    fn activity_wait_rejects_infinite_sentinel_timeout() {
        let slot = Slot::new().unwrap();
        assert!(matches!(
            wait_event(&slot.event, Duration::from_millis(u64::from(u32::MAX))),
            Err(ClientError::InvalidDeadline)
        ));
    }
}
