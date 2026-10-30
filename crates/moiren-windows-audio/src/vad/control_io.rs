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
            ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_REVISION_MISMATCH, GENERIC_READ,
            GENERIC_WRITE, HANDLE, WAIT_TIMEOUT,
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

fn operation_name(code: u32) -> &'static str {
    match code {
        crate::vad::IOCTL_CAPABILITIES => "CAPABILITIES",
        crate::vad::IOCTL_CREATE => "CREATE",
        crate::vad::IOCTL_QUERY => "QUERY",
        crate::vad::IOCTL_REMOVE => "REMOVE",
        crate::vad::IOCTL_SET_GATE => "SET_GATE",
        crate::vad::IOCTL_READ_PCM => "READ_PCM",
        crate::vad::IOCTL_WRITE_PCM => "WRITE_PCM",
        _ => "UNKNOWN",
    }
}

fn operation_label(operation: &'static str, phase: &'static str) -> &'static str {
    match (operation, phase) {
        ("CAPABILITIES", "DeviceIoControl(VAD)") => "CAPABILITIES DeviceIoControl(VAD)",
        ("CREATE", "DeviceIoControl(VAD)") => "CREATE DeviceIoControl(VAD)",
        ("QUERY", "DeviceIoControl(VAD)") => "QUERY DeviceIoControl(VAD)",
        ("REMOVE", "DeviceIoControl(VAD)") => "REMOVE DeviceIoControl(VAD)",
        ("SET_GATE", "DeviceIoControl(VAD)") => "SET_GATE DeviceIoControl(VAD)",
        ("READ_PCM", "DeviceIoControl(VAD)") => "READ_PCM DeviceIoControl(VAD)",
        ("WRITE_PCM", "DeviceIoControl(VAD)") => "WRITE_PCM DeviceIoControl(VAD)",
        ("CAPABILITIES", "GetOverlappedResultEx(VAD)") => "CAPABILITIES GetOverlappedResultEx(VAD)",
        ("CREATE", "GetOverlappedResultEx(VAD)") => "CREATE GetOverlappedResultEx(VAD)",
        ("QUERY", "GetOverlappedResultEx(VAD)") => "QUERY GetOverlappedResultEx(VAD)",
        ("REMOVE", "GetOverlappedResultEx(VAD)") => "REMOVE GetOverlappedResultEx(VAD)",
        ("SET_GATE", "GetOverlappedResultEx(VAD)") => "SET_GATE GetOverlappedResultEx(VAD)",
        ("READ_PCM", "GetOverlappedResultEx(VAD)") => "READ_PCM GetOverlappedResultEx(VAD)",
        ("WRITE_PCM", "GetOverlappedResultEx(VAD)") => "WRITE_PCM GetOverlappedResultEx(VAD)",
        (_, _) => phase,
    }
}

fn control_error(code: u32, phase: &'static str, error: Error) -> ClientError {
    ClientError::windows(operation_label(operation_name(code), phase), error)
}

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
    /// Granted PCM leases by registry slot: endpoint key plus the lease's
    /// unique grant. Releasing clears only an exact (key, grant) match, so a
    /// residual lease dropping after its slot was reused can never clear a
    /// replacement lease's registration.
    pcm_endpoints: Mutex<[Option<(EndpointKey, u64)>; MAX_ENDPOINTS as usize]>,
    pcm_sequence: AtomicU64,
    lease_grants: AtomicU64,
}

/// Holds the original owning handle open as well as a same-file duplicate.
/// A second helper for the same endpoint would exceed the four-slot contract.
pub(super) struct PcmLease {
    pub(super) handle: OwnedHandle,
    owner: Arc<SessionHandle>,
    key: EndpointKey,
    slot: usize,
    grant: u64,
}

impl PcmLease {
    pub(super) fn next_sequence(&self) -> Result<u64, ClientError> {
        next_pcm_sequence(&self.owner.pcm_sequence)
    }

    /// Free this lease's registry slot before the lease itself drops.
    /// Idempotent and identity-checked: a later Drop finds the slot either
    /// already cleared or reused by a different grant, and touches neither.
    /// Retiring a PCM owner calls this so a replacement owner can be prepared
    /// while the retired owner's residual drain still owns the duplicate
    /// handle.
    pub(super) fn release_endpoint_slot(&self) {
        let mut endpoints = self
            .owner
            .pcm_endpoints
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if endpoints[self.slot] == Some((self.key, self.grant)) {
            endpoints[self.slot] = None;
        }
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
        self.release_endpoint_slot();
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
                lease_grants: AtomicU64::new(1),
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
        if endpoints
            .iter()
            .any(|slot| matches!(slot, Some((held, _)) if *held == key))
        {
            return Err(ClientError::PcmOwnerExists);
        }
        let slot = endpoints
            .iter()
            .position(Option::is_none)
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
        let grant = self
            .owner
            .lease_grants
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        endpoints[slot] = Some((key, grant));
        Ok(PcmLease {
            handle,
            owner: Arc::clone(&self.owner),
            key,
            slot,
            grant,
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
            return Err(control_error(code, "DeviceIoControl(VAD)", error));
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
            let handle = raw(&self.owner.handle);
            return Err(self.finish_failed_exchange_named(
                request,
                error,
                operation_name(code),
                |request| {
                    // SAFETY: the request still owns its OVERLAPPED and buffers.
                    unsafe {
                        GetOverlappedResult(
                            handle,
                            &request.overlapped,
                            &mut request.returned,
                            false,
                        )
                    }
                },
                |request| {
                    // SAFETY: cancellation targets exactly this live request.
                    let _ = unsafe { CancelIoEx(handle, Some(&request.overlapped)) };
                },
            ));
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

    #[cfg(test)]
    fn finish_failed_exchange(
        &mut self,
        request: Box<Request>,
        error: Error,
        probe: impl FnMut(&mut Request) -> windows::core::Result<()>,
        cancel: impl FnOnce(&Request),
    ) -> ClientError {
        self.finish_failed_exchange_named(request, error, "", probe, cancel)
    }

    fn finish_failed_exchange_named(
        &mut self,
        mut request: Box<Request>,
        error: Error,
        operation: &'static str,
        mut probe: impl FnMut(&mut Request) -> windows::core::Result<()>,
        cancel: impl FnOnce(&Request),
    ) -> ClientError {
        // A peer close can return a terminal revision refusal. Confirm that
        // exact status before allowing a subsequent QUERY on the same view.
        if error.code() == ERROR_REVISION_MISMATCH.to_hresult()
            && probe(&mut request).is_err_and(|terminal| terminal.code() == error.code())
        {
            return ClientError::windows(
                operation_label(operation, "GetOverlappedResultEx(VAD)"),
                error,
            );
        }
        self.poisoned = true;
        cancel(&request);
        let drained = probe(&mut request);
        if drained
            .as_ref()
            .is_err_and(|error| error.code() == ERROR_IO_INCOMPLETE.to_hresult())
        {
            // Cancellation alone is not a drain. Retain the buffers and event
            // until Drop collects terminal completion on the control thread.
            self.pending = Some(request);
        }
        if error.code() == HRESULT::from_win32(WAIT_TIMEOUT.0) {
            ClientError::DeadlineExceeded
        } else {
            ClientError::windows(
                operation_label(operation, "GetOverlappedResultEx(VAD)"),
                error,
            )
        }
    }

    fn drain_pending(
        &mut self,
        cancel: impl FnOnce(&Request),
        complete: impl FnOnce(&mut Request) -> windows::core::Result<()>,
    ) {
        if let Some(mut request) = self.pending.take() {
            cancel(&request);
            let _ = complete(&mut request);
        }
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
        let handle = raw(&self.owner.handle);
        self.drain_pending(
            |request| {
                // SAFETY: cancellation targets exactly this retained request.
                let _ = unsafe { CancelIoEx(handle, Some(&request.overlapped)) };
            },
            |request| {
                // SAFETY: block until the kernel releases this request's storage.
                // The entire client, including Drop, is control-thread only.
                unsafe {
                    GetOverlappedResult(handle, &request.overlapped, &mut request.returned, true)
                }
            },
        );
        // PCM leases retain the original handle until their own cancellation
        // drains finish. Only the last same-file handle triggers owner cleanup.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, time::Duration};
    use windows::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_OPERATION_ABORTED};

    #[test]
    fn every_current_control_ioctl_has_operation_and_phase_labels() {
        let operations = [
            (crate::vad::IOCTL_CAPABILITIES, "CAPABILITIES"),
            (crate::vad::IOCTL_CREATE, "CREATE"),
            (crate::vad::IOCTL_QUERY, "QUERY"),
            (crate::vad::IOCTL_REMOVE, "REMOVE"),
            (crate::vad::IOCTL_SET_GATE, "SET_GATE"),
            (crate::vad::IOCTL_READ_PCM, "READ_PCM"),
            (crate::vad::IOCTL_WRITE_PCM, "WRITE_PCM"),
        ];
        for (code, operation) in operations {
            assert_eq!(operation_name(code), operation);
            for phase in ["DeviceIoControl(VAD)", "GetOverlappedResultEx(VAD)"] {
                assert_eq!(
                    operation_label(operation_name(code), phase),
                    format!("{operation} {phase}")
                );
                let hresult = HRESULT(0xE000_0225_u32 as i32);
                let error = control_error(code, phase, Error::from_hresult(hresult));
                let ClientError::Windows {
                    operation: actual,
                    source,
                } = error
                else {
                    panic!("preserve native error");
                };
                assert_eq!(actual, format!("{operation} {phase}"));
                assert_eq!(source.code(), hresult);
            }
        }
        // SET_POLICY is not an IOCTL in the checked-in Rust ABI or driver header.
        // Do not invent its numeric code or alias any reserved code to it.
        for code in [0, 0x0022_e014, u32::MAX] {
            assert_eq!(operation_name(code), "UNKNOWN");
            for phase in [
                "DeviceIoControl(VAD)",
                "GetOverlappedResultEx(VAD)",
                "future phase",
            ] {
                assert_eq!(operation_label(operation_name(code), phase), phase);
                let hresult = HRESULT(0x8889_0004_u32 as i32);
                assert!(
                    matches!(control_error(code, phase, Error::from_hresult(hresult)), ClientError::Windows { operation, source } if operation == phase && source.code() == hresult)
                );
            }
        }
    }

    #[test]
    fn named_wait_failure_retains_original_hresult_and_cancellation_behavior() {
        for hresult in [
            ERROR_REVISION_MISMATCH.to_hresult(),
            HRESULT(0xE000_0225_u32 as i32),
        ] {
            let mut control = test_control();
            let cancelled = Cell::new(false);
            let error = control.finish_failed_exchange_named(
                test_request(),
                Error::from_hresult(hresult),
                operation_name(crate::vad::IOCTL_QUERY),
                |_| Err(Error::from_hresult(hresult)),
                |_| cancelled.set(true),
            );
            assert!(
                matches!(error, ClientError::Windows { operation: "QUERY GetOverlappedResultEx(VAD)", source } if source.code() == hresult)
            );
            assert_eq!(
                cancelled.get(),
                hresult != ERROR_REVISION_MISMATCH.to_hresult()
            );
            assert_eq!(control.poisoned, cancelled.get());
            assert!(control.pending.is_none());
        }
    }

    fn test_event() -> OwnedHandle {
        // SAFETY: an unnamed event is owned only by this test, never a driver.
        let event = unsafe { CreateEventW(None, true, false, None) }.unwrap();
        unsafe { OwnedHandle::from_raw_handle(event.0) }
    }

    fn test_control() -> ControlHandle {
        ControlHandle {
            owner: Arc::new(SessionHandle {
                // The expired exchange below returns before any native IO.
                handle: test_event(),
                pcm_endpoints: Mutex::new([None; MAX_ENDPOINTS as usize]),
                pcm_sequence: AtomicU64::new(0),
                lease_grants: AtomicU64::new(1),
            }),
            pending: None,
            poisoned: false,
        }
    }

    fn test_request() -> Box<Request> {
        let event = test_event();
        Box::new(Request {
            input: [0; CREATE_SIZE],
            output: [0; 128],
            overlapped: OVERLAPPED {
                hEvent: raw(&event),
                ..Default::default()
            },
            returned: 0,
            _event: event,
        })
    }

    fn next_expired_exchange(control: &mut ControlHandle) -> ClientError {
        control
            .exchange::<0>(crate::vad::IOCTL_QUERY, &[], Instant::now())
            .unwrap_err()
    }

    fn assert_native_error(error: ClientError, expected: HRESULT) {
        assert!(matches!(
            error,
            ClientError::Windows { operation: "GetOverlappedResultEx(VAD)", source }
                if source.code() == expected
        ));
    }

    fn finish_retained_request(control: &mut ControlHandle, expected: *const Request) {
        assert!(std::ptr::eq(control.pending.as_deref().unwrap(), expected,));
        let steps = Cell::new(0);
        control.drain_pending(
            |request| {
                assert!(std::ptr::eq(request, expected));
                assert_eq!(steps.replace(1), 0);
            },
            |request| {
                assert!(std::ptr::eq(request, expected));
                assert_eq!(steps.replace(2), 1);
                Err(Error::from_hresult(ERROR_OPERATION_ABORTED.to_hresult()))
            },
        );
        assert_eq!(steps.get(), 2);
        assert!(control.pending.is_none());
        assert!(matches!(
            next_expired_exchange(control),
            ClientError::SessionUncertain
        ));
    }

    #[test]
    fn terminal_revision_mismatch_allows_the_next_exchange_without_cancellation() {
        let mut control = test_control();
        let mismatch = ERROR_REVISION_MISMATCH.to_hresult();
        let probes = Cell::new(0);
        let error = control.finish_failed_exchange(
            test_request(),
            Error::from_hresult(mismatch),
            |_| {
                probes.set(probes.get() + 1);
                Err(Error::from_hresult(mismatch))
            },
            |_| panic!("a confirmed terminal refusal needs no cancellation"),
        );
        assert_native_error(error, mismatch);
        assert_eq!(probes.get(), 1);
        assert!(control.pending.is_none());
        assert!(matches!(
            next_expired_exchange(&mut control),
            ClientError::DeadlineExceeded
        ));
    }

    #[test]
    fn unconfirmed_revision_mismatch_retains_pending_storage_until_terminal_drain() {
        for after_cancel in [
            ERROR_IO_INCOMPLETE.to_hresult(),
            ERROR_REVISION_MISMATCH.to_hresult(),
        ] {
            let mut control = test_control();
            let request = test_request();
            let address = &*request as *const Request;
            let mismatch = ERROR_REVISION_MISMATCH.to_hresult();
            let probes = Cell::new(0);
            let cancelled = Cell::new(false);
            let error = control.finish_failed_exchange(
                request,
                Error::from_hresult(mismatch),
                |_| {
                    let attempt = probes.get();
                    assert_eq!(cancelled.get(), attempt != 0);
                    probes.set(attempt + 1);
                    Err(Error::from_hresult(if attempt == 0 {
                        ERROR_IO_INCOMPLETE.to_hresult()
                    } else {
                        after_cancel
                    }))
                },
                |_| assert!(!cancelled.replace(true)),
            );
            assert_native_error(error, mismatch);
            assert_eq!(probes.get(), 2);
            assert!(cancelled.get());
            assert!(matches!(
                next_expired_exchange(&mut control),
                ClientError::SessionUncertain
            ));
            if after_cancel == ERROR_IO_INCOMPLETE.to_hresult() {
                finish_retained_request(&mut control, address);
            } else {
                assert!(control.pending.is_none());
            }
        }
    }

    #[test]
    fn timeout_and_incomplete_waits_keep_cancellation_poison_and_pending_storage() {
        for code in [
            HRESULT::from_win32(WAIT_TIMEOUT.0),
            ERROR_IO_INCOMPLETE.to_hresult(),
        ] {
            let mut control = test_control();
            let request = test_request();
            let address = &*request as *const Request;
            let cancelled = Cell::new(false);
            let probes = Cell::new(0);
            let error = control.finish_failed_exchange(
                request,
                Error::from_hresult(code),
                |_| {
                    assert!(cancelled.get());
                    probes.set(probes.get() + 1);
                    Err(Error::from_hresult(ERROR_IO_INCOMPLETE.to_hresult()))
                },
                |_| assert!(!cancelled.replace(true)),
            );
            if code == HRESULT::from_win32(WAIT_TIMEOUT.0) {
                assert!(matches!(error, ClientError::DeadlineExceeded));
            } else {
                assert_native_error(error, code);
            }
            assert_eq!(probes.get(), 1);
            assert!(matches!(
                next_expired_exchange(&mut control),
                ClientError::SessionUncertain
            ));
            finish_retained_request(&mut control, address);
        }
    }

    #[test]
    fn different_terminal_probe_results_never_confirm_revision_mismatch() {
        for probe_code in [None, Some(ERROR_DEVICE_NOT_CONNECTED.to_hresult())] {
            let mut control = test_control();
            let mismatch = ERROR_REVISION_MISMATCH.to_hresult();
            let cancelled = Cell::new(false);
            let probes = Cell::new(0);
            let error = control.finish_failed_exchange(
                test_request(),
                Error::from_hresult(mismatch),
                |_| {
                    let attempt = probes.get();
                    assert_eq!(cancelled.get(), attempt != 0);
                    probes.set(attempt + 1);
                    probe_code.map_or(Ok(()), |code| Err(Error::from_hresult(code)))
                },
                |_| assert!(!cancelled.replace(true)),
            );
            assert_native_error(error, mismatch);
            assert_eq!(probes.get(), 2);
            assert!(control.pending.is_none());
            assert!(matches!(
                next_expired_exchange(&mut control),
                ClientError::SessionUncertain
            ));
        }
    }

    #[test]
    fn other_wait_errors_racing_a_terminal_revision_mismatch_remain_poisoned() {
        for code in [
            HRESULT::from_win32(WAIT_TIMEOUT.0),
            ERROR_DEVICE_NOT_CONNECTED.to_hresult(),
        ] {
            let mut control = test_control();
            let cancelled = Cell::new(false);
            let error = control.finish_failed_exchange(
                test_request(),
                Error::from_hresult(code),
                |_| {
                    assert!(cancelled.get());
                    Err(Error::from_hresult(ERROR_REVISION_MISMATCH.to_hresult()))
                },
                |_| assert!(!cancelled.replace(true)),
            );
            if code == HRESULT::from_win32(WAIT_TIMEOUT.0) {
                assert!(matches!(error, ClientError::DeadlineExceeded));
            } else {
                assert_native_error(error, code);
            }
            assert!(control.pending.is_none());
            assert!(matches!(
                next_expired_exchange(&mut control),
                ClientError::SessionUncertain
            ));
        }
    }

    #[test]
    fn terminal_revision_mismatch_never_clears_existing_poison() {
        let mut control = test_control();
        control.poisoned = true;
        let mismatch = ERROR_REVISION_MISMATCH.to_hresult();
        let error = control.finish_failed_exchange(
            test_request(),
            Error::from_hresult(mismatch),
            |_| Err(Error::from_hresult(mismatch)),
            |_| panic!("terminal request"),
        );
        assert_native_error(error, mismatch);
        assert!(control.pending.is_none());
        assert!(matches!(
            next_expired_exchange(&mut control),
            ClientError::SessionUncertain
        ));
    }

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
    fn retired_lease_dropping_after_replacement_never_clears_the_new_grant() {
        let control = test_control();
        let key = EndpointKey::new(crate::vad::RuntimePdoToken::new(1).unwrap(), 2, 3).unwrap();
        let residual = control.pcm_lease(key).unwrap();
        // An owner retire frees the registry slot before the residual lease
        // drops; the replacement may be granted in that same slot.
        residual.release_endpoint_slot();
        let replacement = control.pcm_lease(key).unwrap();
        // The residual lease outlives the grant: its Drop must not clear the
        // replacement's registration.
        drop(residual);
        assert!(
            matches!(control.pcm_lease(key), Err(ClientError::PcmOwnerExists)),
            "a late residual drop must not clear the replacement lease's grant"
        );
        drop(replacement);
        // With the replacement gone the endpoint is leaseable again.
        assert!(control.pcm_lease(key).is_ok());
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
