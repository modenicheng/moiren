//! One exclusive owning file object, with bounded overlapped control waits.

use super::{CREATE_SIZE, ClientError, DeviceInterface, EndpointKey, MAX_ENDPOINTS, ProtocolError};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
    time::Instant,
};
use windows::{
    Win32::{
        Foundation::{
            ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE,
            WAIT_TIMEOUT,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE,
            OPEN_EXISTING,
        },
        System::{
            IO::{
                CancelIoEx, DeviceIoControl, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
            },
            Threading::CreateEventW,
        },
    },
    core::{Error, HRESULT},
};

struct Request {
    input: [u8; CREATE_SIZE],
    output: [u8; 128],
    overlapped: OVERLAPPED,
    returned: u32,
    // The event must remain open until the kernel has finished the request.
    _event: OwnedHandle,
}

pub(super) struct ControlHandle {
    owner: Arc<SessionHandle>,
    pending: Option<Box<Request>>,
    poisoned: bool,
}

// SAFETY: the handle and overlapped request are owned by one ControlHandle at
// a time. The Windows file object is thread agnostic, and callers serialize
// each handle's request state through the owning ControlClient. The raw event
// pointer is only consumed by the issuing thread while its Box<Request> stays
// live until terminal completion.
unsafe impl Send for ControlHandle {}

pub(super) fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

struct SessionHandle {
    handle: OwnedHandle,
    pcm_endpoints: Mutex<[Option<EndpointKey>; MAX_ENDPOINTS as usize]>,
    pcm_sequence: AtomicU64,
}

/// Holds the original owning handle open as well as a same-file duplicate.
/// A second helper for the same endpoint would exceed the four-slot contract.
pub(super) struct PcmLease {
    pub(super) handle: OwnedHandle,
    owner: Arc<SessionHandle>,
    key: EndpointKey,
}

impl PcmLease {
    pub(super) fn next_sequence(&self) -> Result<u64, ClientError> {
        next_pcm_sequence(&self.owner.pcm_sequence)
    }
}

fn next_pcm_sequence(sequence: &AtomicU64) -> Result<u64, ClientError> {
    // Sequence belongs to the owning file, not a four-slot helper. Rebuilding
    // PCM after a clock/carrier reset can leave the driver's gate unchanged.
    // A session-wide monotonic counter is stronger than per-endpoint ordering
    // and prevents such a helper from restarting at one under the same epoch.
    sequence
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map(|previous| previous + 1)
        .map_err(|_| ClientError::SequenceExhausted)
}
impl Drop for PcmLease {
    fn drop(&mut self) {
        let mut endpoints = self
            .owner
            .pcm_endpoints
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = endpoints.iter_mut().find(|slot| **slot == Some(self.key)) {
            *slot = None;
        }
    }
}

impl ControlHandle {
    pub(super) fn open(interface: &DeviceInterface) -> Result<Self, ClientError> {
        // SAFETY: discovery owns a validated NUL-terminated wide path. A null
        // SECURITY_ATTRIBUTES pointer makes the handle non-inheritable; sharing
        // zero enforces exclusive read/write ownership. No arbitrary named opens.
        let handle = unsafe {
            CreateFileW(
                interface.wide_path(),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(|error| ClientError::windows("CreateFileW(VAD control)", error))?;
        Ok(Self {
            // SAFETY: CreateFileW succeeded and ownership is transferred once.
            owner: Arc::new(SessionHandle {
                handle: unsafe { OwnedHandle::from_raw_handle(handle.0) },
                pcm_endpoints: Mutex::new([None; MAX_ENDPOINTS as usize]),
                pcm_sequence: AtomicU64::new(0),
            }),
            pending: None,
            poisoned: false,
        })
    }

    pub(super) fn pcm_lease(&self, key: EndpointKey) -> Result<PcmLease, ClientError> {
        if self.poisoned {
            return Err(ClientError::SessionUncertain);
        }
        let mut endpoints = self
            .owner
            .pcm_endpoints
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if endpoints.contains(&Some(key)) {
            return Err(ClientError::PcmOwnerExists);
        }
        let slot = endpoints
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ClientError::PcmOwnerExists)?;
        // OwnedHandle::try_clone creates a non-inheritable same-file handle.
        let handle = self
            .owner
            .handle
            .try_clone()
            .map_err(|source| ClientError::Io {
                operation: "clone VAD PCM handle",
                source,
            })?;
        *slot = Some(key);
        Ok(PcmLease {
            handle,
            owner: Arc::clone(&self.owner),
            key,
        })
    }

    /// Create another control view of the same owning file object. The root
    /// session stays alive through the shared Arc, while each view retains its
    /// own serialized control request state.
    pub(super) fn duplicate(&self) -> Self {
        Self {
            owner: Arc::clone(&self.owner),
            pending: None,
            poisoned: false,
        }
    }

    pub(super) fn exchange<const N: usize>(
        &mut self,
        code: u32,
        input: &[u8],
        deadline: Instant,
    ) -> Result<[u8; N], ClientError> {
        if input.len() > CREATE_SIZE || N > 128 {
            return Err(ProtocolError::Inconsistent("control buffer exceeds ABI bounds").into());
        }
        if self.poisoned {
            return Err(ClientError::SessionUncertain);
        }
        if wait_milliseconds(deadline, Instant::now()).is_none() {
            return Err(ClientError::DeadlineExceeded);
        }
        // SAFETY: unnamed, non-inheritable manual-reset event; transferred once.
        let event = unsafe { CreateEventW(None, true, false, None) }
            .map_err(|error| ClientError::windows("CreateEventW(VAD IOCTL)", error))?;
        let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
        let mut request = Box::new(Request {
            input: [0; CREATE_SIZE],
            output: [0; 128],
            overlapped: OVERLAPPED {
                hEvent: raw(&event),
                ..Default::default()
            },
            returned: 0,
            _event: event,
        });
        request.input[..input.len()].copy_from_slice(input);
        // SAFETY: all request storage is pinned by this Box until completion.
        // The mutable client borrow permits only one outstanding control IOCTL.
        let submitted = unsafe {
            DeviceIoControl(
                raw(&self.owner.handle),
                code,
                Some(request.input.as_ptr().cast()),
                input.len() as u32,
                Some(request.output.as_mut_ptr().cast()),
                N as u32,
                Some(&mut request.returned),
                Some(&mut request.overlapped),
            )
        };
        if let Err(error) = submitted
            && error.code() != ERROR_IO_PENDING.to_hresult()
        {
            return Err(ClientError::windows("DeviceIoControl(VAD)", error));
        }
        // Event creation, allocation, submission, or scheduling may have consumed
        // the budget. Refresh it here, after submission. Expiry uses exactly the
        // same cancellation/retention path as a timeout of the completion wait.
        let completed = match wait_milliseconds(deadline, Instant::now()) {
            Some(milliseconds) => {
                // SAFETY: file/event/OVERLAPPED and both buffers remain live.
                // Immediate completion is also collected for its exact byte count.
                unsafe {
                    GetOverlappedResultEx(
                        raw(&self.owner.handle),
                        &request.overlapped,
                        &mut request.returned,
                        milliseconds,
                        false,
                    )
                }
            }
            None => Err(Error::from_hresult(HRESULT::from_win32(WAIT_TIMEOUT.0))),
        };
        if let Err(error) = completed {
            self.poisoned = true;
            // SAFETY: cancellation targets exactly this live request. Completion
            // may have raced, so a cancellation error alone does not free buffers.
            let _ = unsafe { CancelIoEx(raw(&self.owner.handle), Some(&request.overlapped)) };
            // SAFETY: a nonblocking result check distinguishes pending storage
            // from a completed request (including completed IOCTL failures).
            let drained = unsafe {
                GetOverlappedResult(
                    raw(&self.owner.handle),
                    &request.overlapped,
                    &mut request.returned,
                    false,
                )
            };
            if drained
                .as_ref()
                .is_err_and(|error| error.code() == ERROR_IO_INCOMPLETE.to_hresult())
            {
                // A timed-out request can still be owned by the kernel. Keep its
                // storage and event until Drop finishes cancellation on control.
                self.pending = Some(request);
            }
            return Err(if error.code() == HRESULT::from_win32(WAIT_TIMEOUT.0) {
                ClientError::DeadlineExceeded
            } else {
                ClientError::windows("GetOverlappedResultEx(VAD)", error)
            });
        }
        if request.returned as usize != N {
            return Err(ProtocolError::BufferSize {
                expected: N,
                actual: request.returned as usize,
            }
            .into());
        }
        Ok(request.output[..N].try_into().expect("bounded output"))
    }
}

pub(super) fn wait_milliseconds(deadline: Instant, now: Instant) -> Option<u32> {
    let remaining = deadline.checked_duration_since(now)?;
    if remaining.is_zero() {
        return None;
    }
    // Round up sub-millisecond budgets, but never accidentally request INFINITE.
    Some(
        remaining
            .as_nanos()
            .div_ceil(1_000_000)
            .min(u128::from(u32::MAX - 1)) as u32,
    )
}

impl Drop for ControlHandle {
    fn drop(&mut self) {
        if let Some(mut request) = self.pending.take() {
            // SAFETY: the kernel must release the OVERLAPPED and buffers before
            // Rust reclaims them. This cancellation drain can block for a broken
            // driver; the entire client, including Drop, is control-thread only.
            unsafe {
                let _ = CancelIoEx(raw(&self.owner.handle), Some(&request.overlapped));
                let _ = GetOverlappedResult(
                    raw(&self.owner.handle),
                    &request.overlapped,
                    &mut request.returned,
                    true,
                );
            }
        }
        // PCM leases retain the original handle until their own cancellation
        // drains finish. Only the last same-file handle triggers owner cleanup.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pcm_helper_replacement_and_other_endpoints_cannot_rewind_session_sequence() {
        let sequence = AtomicU64::new(0);
        let helper = || next_pcm_sequence(&sequence).unwrap();
        assert_eq!(helper(), 1);
        assert_eq!(helper(), 2);
        // A replaced helper and another endpoint share the original counter.
        let replacement = || next_pcm_sequence(&sequence).unwrap();
        assert_eq!(replacement(), 3);
        assert_eq!(helper(), 4);
        sequence.store(u64::MAX, Ordering::Relaxed);
        assert!(matches!(
            next_pcm_sequence(&sequence),
            Err(ClientError::SequenceExhausted)
        ));
        assert_eq!(sequence.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn expired_budget_does_not_start_a_completion_wait() {
        let now = Instant::now();
        assert_eq!(wait_milliseconds(now, now), None);
        assert_eq!(wait_milliseconds(now - Duration::from_secs(1), now), None);
    }

    #[test]
    fn remaining_budget_is_refreshed_after_submission() {
        let before_submission = Instant::now();
        let deadline = before_submission + Duration::from_millis(100);
        assert_eq!(wait_milliseconds(deadline, before_submission), Some(100));
        assert_eq!(
            wait_milliseconds(deadline, before_submission + Duration::from_millis(80)),
            Some(20)
        );
        assert_eq!(
            wait_milliseconds(deadline, deadline + Duration::from_nanos(1)),
            None
        );
    }

    #[test]
    fn finite_wait_never_becomes_infinite() {
        let now = Instant::now();
        let long_deadline = now + Duration::from_secs(u64::from(u32::MAX));
        assert_eq!(wait_milliseconds(long_deadline, now), Some(u32::MAX - 1));
        assert_eq!(
            wait_milliseconds(now + Duration::from_nanos(1), now),
            Some(1)
        );
    }
}
