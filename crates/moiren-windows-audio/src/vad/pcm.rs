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
        ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NOT_ENOUGH_QUOTA, ERROR_OPERATION_ABORTED,
        ERROR_SEM_TIMEOUT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
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

    fn finish_error(
        &mut self,
        book: &mut SlotBook,
        ticket: PcmTicket,
        error: windows::core::Error,
        waiting: bool,
        writing: bool,
        cancel: impl FnOnce(),
    ) -> Result<Option<PcmCompletion<'_>>, ClientError> {
        if !waiting && error.code() == ERROR_IO_INCOMPLETE.to_hresult() {
            return Ok(None);
        }
        if error.code() == HRESULT::from_win32(WAIT_TIMEOUT.0)
            || error.code() == ERROR_IO_INCOMPLETE.to_hresult()
        {
            cancel();
            return Err(ClientError::DeadlineExceeded);
        }
        self.pending = false;
        book.finish(ticket).map_err(slot_error)?;
        if error.code() == ERROR_OPERATION_ABORTED.to_hresult() {
            // Terminal cancellation releases this ticket only. It is neither
            // PCM delivery nor permission to forgive a running owner's abort;
            // the transport must still check its stop/admission/epoch state.
            log::debug!(
                target: "moiren::audio::vad",
                "event=pcm_terminal_cancel ticket={ticket:?} writing={writing} remaining_pending={}",
                book.pending(),
            );
            return Err(ClientError::Cancelled);
        }
        if writing_completion_timeout(error.code(), writing) {
            return Ok(Some(PcmCompletion::CaptureDropped { stamp: self.stamp }));
        }
        if kernel_quota_rejection(error.code()) {
            return Err(ClientError::KernelPcmQuotaFull);
        }
        Err(ClientError::windows("GetOverlappedResult(VAD PCM)", error))
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
    shutdown_faults: ShutdownFaults,
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
            shutdown_faults: ShutdownFaults::default(),
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
            if kernel_quota_rejection(error.code()) {
                return Err(ClientError::KernelPcmQuotaFull);
            }
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
            let handle = raw(&self.lease.handle);
            // SAFETY: the boxed request stays live through error collection.
            let overlapped = unsafe { addr_of!((*storage).overlapped) };
            return slot.finish_error(
                &mut self.book,
                ticket,
                error,
                deadline.is_some(),
                writing,
                || {
                    // SAFETY: targets this live request; cancellation is not a drain.
                    let _ = unsafe { CancelIoEx(handle, Some(overlapped)) };
                },
            );
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
    /// `Duration::ZERO` is a pure non-blocking probe: it still cancels, then
    /// collects only requests that already reached terminal status, so parked
    /// residuals release as soon as the driver completes them. It must never
    /// short-circuit before cancelling — a parked slot that cannot be probed
    /// turns the retire park back into a blocking drain (2026-10-24 audit).
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), ClientError> {
        let deadline = if timeout.is_zero() {
            None
        } else {
            Some(deadline_after(timeout)?)
        };
        self.cancel_all();
        let handle = raw(&self.lease.handle);
        shutdown_slots(
            &mut self.slots,
            &mut self.book,
            &mut self.shutdown_faults,
            |slot| {
                let storage = slot.storage.get();
                let mut returned = 0;
                // SAFETY: no storage is reclaimed until terminal completion.
                unsafe {
                    if let Some(deadline) = deadline {
                        let milliseconds = wait_milliseconds(deadline, Instant::now()).unwrap_or(0);
                        GetOverlappedResultEx(
                            handle,
                            addr_of!((*storage).overlapped),
                            &mut returned,
                            milliseconds,
                            false,
                        )
                    } else {
                        GetOverlappedResult(
                            handle,
                            addr_of!((*storage).overlapped),
                            &mut returned,
                            false,
                        )
                    }
                }
            },
        )
    }

    /// Cancel every outstanding request and collect what the driver completes
    /// within `quick`. Returns true when everything drained without faults.
    /// Pending requests or retained terminal faults move the whole owner into
    /// `residual` for the worker to collect and report. Moving is allocation
    /// free; formatting terminal faults may allocate on this non-RT path.
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

#[derive(Default)]
struct ShutdownFaults {
    codes: [Option<HRESULT>; REQUEST_SLOTS],
}

impl ShutdownFaults {
    fn take_error(&mut self) -> Option<ClientError> {
        std::mem::take(self).error()
    }

    fn error(&self) -> Option<ClientError> {
        use std::fmt::Write;
        let mut message = String::new();
        for (index, code) in self.codes.iter().enumerate() {
            if let Some(code) = code {
                if !message.is_empty() {
                    message.push_str("; ");
                }
                write!(message, "slot={index} HRESULT=0x{:08X}", code.0 as u32)
                    .expect("writing into String");
            }
        }
        (!message.is_empty()).then(|| ClientError::Io {
            operation: "VAD PCM shutdown terminal failures",
            source: std::io::Error::other(message),
        })
    }
}

fn shutdown_slots(
    slots: &mut [Slot; REQUEST_SLOTS],
    book: &mut SlotBook,
    faults: &mut ShutdownFaults,
    mut collect: impl FnMut(&mut Slot) -> windows::core::Result<()>,
) -> Result<(), ClientError> {
    for (index, slot) in slots.iter_mut().enumerate() {
        if !slot.pending {
            continue;
        }
        if let Err(error) = collect(slot) {
            if error.code() == ERROR_IO_INCOMPLETE.to_hresult()
                || error.code() == HRESULT::from_win32(WAIT_TIMEOUT.0)
            {
                // One incomplete request must not prevent nonblocking probes
                // of later slots. The shared deadline still bounds all waits.
                continue;
            }
            if error.code() != ERROR_OPERATION_ABORTED.to_hresult() {
                // Four immutable terminal outcomes, retained across repeated
                // residual probes. Cancellation is not an IO fault; all other
                // terminal errors remain cleanup failures even after drain.
                faults.codes[index] = Some(error.code());
                log::warn!(
                    target: "moiren::audio::vad",
                    "event=pcm_shutdown_terminal_fault slot={index} error={error:?}",
                );
            }
        }
        slot.pending = false;
        book.retire_slot(index);
    }
    // A failed terminal request relinquishes its storage, but not its failure
    // evidence. Report it even when another slot remains pending for Drop.
    if let Some(error) = faults.error() {
        Err(error)
    } else if book.pending() != 0 {
        Err(ClientError::DeadlineExceeded)
    } else {
        Ok(())
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

/// The driver reserves one of the endpoint's four kernel request slots at
/// submission; while a retired owner's cancelled requests still occupy them,
/// a replacement owner's submissions complete with the quota rejection. The
/// kernel audit (2026-10-24) confirmed this is a clean transient rejection,
/// not queue corruption — the pump must defer, never poison, on it.
fn kernel_quota_rejection(code: HRESULT) -> bool {
    code == ERROR_NOT_ENOUGH_QUOTA.to_hresult()
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

/// A retired [`PcmOwner`] with pending requests or terminal cleanup faults.
/// The session lease slot is already released. Ownership stays with the
/// pumping worker in preallocated slots, collected by non-blocking probes
/// between iterations. Fault formatting may allocate on this non-RT path.
pub struct ResidualPcm {
    owner: PcmOwner,
}

impl ResidualPcm {
    pub(crate) fn take_shutdown_failure(&mut self) -> Option<ClientError> {
        self.owner.shutdown_faults.take_error()
    }

    /// Requests the driver still holds. Diagnostics only.
    pub fn pending(&self) -> usize {
        self.owner.pending()
    }

    /// Collect completions until `timeout` elapses; `Duration::ZERO` is a pure
    /// non-blocking probe. Returns whether shutdown succeeded without faults.
    /// A false result does not imply pending IO: check [`Self::pending`] and
    /// retain terminal faults before releasing an already-drained residual.
    /// Drop performs the final drain for requests still pending.
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
        ERROR_SEM_TIMEOUT, RtlNtStatusToDosError, STATUS_IO_TIMEOUT, STATUS_QUOTA_EXCEEDED,
    };
    use windows::Win32::System::Threading::SetEvent;

    #[test]
    fn pcm_cancel_shutdown_fault_transfer_is_once_and_keeps_later_distinct_faults() {
        use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_DEVICE_NOT_CONNECTED};
        let mut faults = ShutdownFaults::default();
        faults.codes[0] = Some(ERROR_DEVICE_NOT_CONNECTED.to_hresult());
        let first = faults.take_error().expect("first fault must transfer");
        assert!(first.to_string().contains("slot=0 HRESULT=0x8007048F"));
        assert!(
            faults.take_error().is_none(),
            "a residual probe must not repeat transferred faults"
        );
        faults.codes[2] = Some(ERROR_ACCESS_DENIED.to_hresult());
        let later = faults
            .take_error()
            .expect("a later terminal fault must transfer");
        let text = later.to_string();
        assert!(text.contains("slot=2 HRESULT=0x80070005"), "{text}");
        assert!(
            !text.contains("slot=0"),
            "old faults must not be allocated and reported again"
        );
        assert!(faults.take_error().is_none());
    }

    #[test]
    fn pcm_cancel_shutdown_drains_every_ticket_and_retains_distinct_terminal_faults() {
        use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_DEVICE_NOT_CONNECTED};
        let mut slots: [Slot; REQUEST_SLOTS] = std::array::from_fn(|_| Slot::new().unwrap());
        let mut book = SlotBook::new().unwrap();
        let tickets: [_; REQUEST_SLOTS] = std::array::from_fn(|_| book.reserve().unwrap());
        for slot in &mut slots {
            slot.pending = true;
        }
        let mut faults = ShutdownFaults::default();
        let codes = [
            ERROR_DEVICE_NOT_CONNECTED.to_hresult(),
            ERROR_OPERATION_ABORTED.to_hresult(),
            ERROR_ACCESS_DENIED.to_hresult(),
        ];
        let mut calls = 0;
        let result = shutdown_slots(&mut slots, &mut book, &mut faults, |_| {
            let index = calls;
            calls += 1;
            if index < codes.len() {
                Err(windows::core::Error::from_hresult(codes[index]))
            } else {
                Ok(())
            }
        });
        assert_eq!(
            calls, REQUEST_SLOTS,
            "terminal faults must not short-circuit peer drain"
        );
        assert_eq!(book.pending(), 0);
        assert!(slots.iter().all(|slot| !slot.pending));
        assert!(tickets.iter().all(|&ticket| book.check(ticket).is_err()));
        let error = result.expect_err("non-cancel terminal faults must reach cleanup result");
        let text = error.to_string();
        assert!(
            text.contains("slot=0") && text.contains("0x8007048F"),
            "{text}"
        );
        assert!(
            text.contains("slot=2") && text.contains("0x80070005"),
            "{text}"
        );
        assert!(
            !text.contains("0x800703E3"),
            "expected cancelled completion is not a fault"
        );
        assert!(
            faults.error().is_some(),
            "fault evidence survives a later drain probe"
        );
        let result = shutdown_slots(&mut slots, &mut book, &mut faults, |_| {
            panic!("already drained")
        });
        assert!(
            result.is_err(),
            "a second probe must not erase a terminal cleanup failure"
        );
    }

    #[test]
    fn pcm_cancel_shutdown_pending_peer_does_not_block_terminal_peers_or_lose_errors() {
        use windows::Win32::Foundation::ERROR_DEVICE_NOT_CONNECTED;
        let mut slots: [Slot; REQUEST_SLOTS] = std::array::from_fn(|_| Slot::new().unwrap());
        let mut book = SlotBook::new().unwrap();
        let tickets: [_; REQUEST_SLOTS] = std::array::from_fn(|_| book.reserve().unwrap());
        for slot in &mut slots {
            slot.pending = true;
        }
        let mut faults = ShutdownFaults::default();
        let mut calls = 0;
        let result = shutdown_slots(&mut slots, &mut book, &mut faults, |_| {
            let code = match calls {
                0 => ERROR_IO_INCOMPLETE.to_hresult(),
                1 => ERROR_DEVICE_NOT_CONNECTED.to_hresult(),
                _ => ERROR_OPERATION_ABORTED.to_hresult(),
            };
            calls += 1;
            Err(windows::core::Error::from_hresult(code))
        });
        assert!(result.is_err());
        assert_eq!(calls, REQUEST_SLOTS);
        assert_eq!(book.pending(), 1);
        assert!(slots[0].pending && book.check(tickets[0]).is_ok());
        assert!(slots[1..].iter().all(|slot| !slot.pending));
        assert!(faults.error().is_some());
        let result = shutdown_slots(&mut slots, &mut book, &mut faults, |_| {
            Err(windows::core::Error::from_hresult(
                ERROR_OPERATION_ABORTED.to_hresult(),
            ))
        });
        assert_eq!(book.pending(), 0);
        assert!(
            result.is_err(),
            "later cancellation cannot erase the distinct earlier fault"
        );
    }

    #[test]
    fn pcm_cancel_terminal_abort_retires_only_the_completed_ticket_and_stays_typed() {
        for writing in [false, true] {
            let mut book = SlotBook::new().unwrap();
            let ticket = book.reserve().unwrap();
            let other = book.reserve().unwrap();
            let mut slot = Slot::new().unwrap();
            slot.pending = true;
            let result = slot.finish_error(
                &mut book,
                ticket,
                windows::core::Error::from_hresult(ERROR_OPERATION_ABORTED.to_hresult()),
                false,
                writing,
                || panic!("terminal completion must not request another cancellation"),
            );
            assert!(matches!(result, Err(ClientError::Cancelled)), "{result:?}");
            assert!(!slot.pending);
            assert!(book.check(ticket).is_err());
            assert!(book.check(other).is_ok());
            assert_eq!(
                book.pending(),
                1,
                "peer requests still need their own drain"
            );
        }
    }

    #[test]
    fn pcm_cancel_timeout_and_incomplete_never_release_pending_storage() {
        for (code, waiting) in [
            (ERROR_IO_INCOMPLETE.to_hresult(), false),
            (ERROR_IO_INCOMPLETE.to_hresult(), true),
            (HRESULT::from_win32(WAIT_TIMEOUT.0), true),
        ] {
            let mut book = SlotBook::new().unwrap();
            let ticket = book.reserve().unwrap();
            let mut slot = Slot::new().unwrap();
            slot.pending = true;
            let cancelled = std::cell::Cell::new(false);
            let result = slot.finish_error(
                &mut book,
                ticket,
                windows::core::Error::from_hresult(code),
                waiting,
                false,
                || cancelled.set(true),
            );
            if waiting {
                assert!(matches!(result, Err(ClientError::DeadlineExceeded)));
                assert!(cancelled.get());
            } else {
                assert!(matches!(result, Ok(None)));
                assert!(!cancelled.get());
            }
            assert!(slot.pending && book.check(ticket).is_ok());
            assert_eq!(book.pending(), 1);
        }
    }

    #[test]
    fn pcm_cancel_does_not_convert_other_terminal_faults_to_cancellation() {
        use windows::Win32::Foundation::ERROR_DEVICE_NOT_CONNECTED;
        let mut book = SlotBook::new().unwrap();
        let ticket = book.reserve().unwrap();
        let mut slot = Slot::new().unwrap();
        slot.pending = true;
        let result = slot.finish_error(
            &mut book,
            ticket,
            windows::core::Error::from_hresult(ERROR_DEVICE_NOT_CONNECTED.to_hresult()),
            false,
            false,
            || panic!("already terminal"),
        );
        assert!(matches!(result, Err(ClientError::Windows { source, .. })
            if source.code() == ERROR_DEVICE_NOT_CONNECTED.to_hresult()));
        assert!(!slot.pending);
        assert_eq!(book.pending(), 0);
    }

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
    fn kernel_quota_rejection_uses_the_windows_completion_error_domain() {
        // SAFETY: pure OS status translation; no device or request is opened.
        let translated = unsafe { RtlNtStatusToDosError(STATUS_QUOTA_EXCEEDED) };
        assert_eq!(translated, ERROR_NOT_ENOUGH_QUOTA.0);
        assert!(kernel_quota_rejection(HRESULT::from_win32(translated)));
        assert!(!kernel_quota_rejection(
            ERROR_OPERATION_ABORTED.to_hresult()
        ));
        assert!(!kernel_quota_rejection(HRESULT::from_win32(WAIT_TIMEOUT.0)));
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
