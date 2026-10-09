//! The agile completion handler owns borrowed activation data through timeout.
use crate::capture::{CaptureError, wasapi::api};
use std::{
    mem::ManuallyDrop,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::E_UNEXPECTED,
        Media::Audio::{
            AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
            AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
            ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
            IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioClient,
            PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        },
        System::{
            Com::{
                BLOB, IAgileObject, IAgileObject_Impl,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
            },
            Variant::VT_BLOB,
        },
    },
    core::{Interface, Ref, Result, implement},
};

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Completion {
    completed: Arc<AtomicBool>,
    // Keep activation data alive even if the owner times out before completion.
    _params: Arc<AUDIOCLIENT_ACTIVATION_PARAMS>,
}
impl IAgileObject_Impl for Completion_Impl {}
impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> Result<()> {
        // No panic, lock, COM interface transfer, or last-reference release in the callback.
        self.completed.store(true, Ordering::Release);
        Ok(())
    }
}

fn activation_params(pid: u32) -> Arc<AUDIOCLIENT_ACTIVATION_PARAMS> {
    Arc::new(AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    })
}

fn borrowed_activation_variant(
    params: &Arc<AUDIOCLIENT_ACTIVATION_PARAMS>,
) -> ManuallyDrop<PROPVARIANT> {
    // windows 0.62 PROPVARIANT implements Drop (PropVariantClear). Suppress the
    // OUTER destructor too: this BLOB borrows Rust memory, not CoTaskMem memory.
    ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: Arc::as_ptr(params).cast_mut().cast(),
                    },
                },
                ..Default::default()
            }),
        },
    })
}

fn wait_for_completion(
    completed: &AtomicBool,
    mut abort: impl FnMut() -> std::result::Result<(), CaptureError>,
    timeout: Duration,
) -> std::result::Result<(), CaptureError> {
    let start = Instant::now();
    loop {
        // Cancellation/exit wins even if completion arrives at the same time.
        abort()?;
        if completed.load(Ordering::Acquire) {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(CaptureError::ActivationTimeout);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub(crate) fn activate(
    pid: u32,
    abort: impl FnMut() -> std::result::Result<(), CaptureError>,
) -> std::result::Result<IAudioClient, CaptureError> {
    let params = activation_params(pid);
    let completed = Arc::new(AtomicBool::new(false));
    let handler: IActivateAudioInterfaceCompletionHandler = Completion {
        completed: Arc::clone(&completed),
        _params: Arc::clone(&params),
    }
    .into();
    let variant = borrowed_activation_variant(&params);
    // The variant borrows Arc-owned data: never PropVariantClear this borrowed BLOB.
    let operation = api("ActivateAudioInterfaceAsync(loopback)", unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*variant),
            &handler,
        )
    })?;
    // Windows retains the agile handler until completion. Dropping our local
    // operation on cancellation does not invalidate its Arc-owned BLOB. A late
    // callback only updates its flag and cannot attach a client to a new owner.
    wait_for_completion(&completed, abort, Duration::from_secs(10))?;
    let mut status = E_UNEXPECTED;
    let mut interface = None;
    api("GetActivateResult(loopback)", unsafe {
        operation.GetActivateResult(&mut status, &mut interface)
    })?;
    api("Activation status(loopback)", status.ok())?;
    let interface = api(
        "Activation interface(loopback)",
        interface.ok_or_else(|| windows::core::Error::from_hresult(E_UNEXPECTED)),
    )?;
    api("IAudioClient(loopback)", interface.cast())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_activation_rechecks_abort_between_poll_ticks() {
        let mut checks = 0;
        let result = wait_for_completion(
            &AtomicBool::new(false),
            || {
                checks += 1;
                if checks == 2 {
                    Err(CaptureError::Cancelled)
                } else {
                    Ok(())
                }
            },
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(CaptureError::Cancelled));
        assert_eq!(checks, 2);
    }

    #[test]
    fn cancel_and_exit_take_precedence_over_completion_and_timeout() {
        for completed in [false, true] {
            for error in [CaptureError::Cancelled, CaptureError::TargetExited] {
                assert_eq!(
                    wait_for_completion(&AtomicBool::new(completed), || Err(error), Duration::ZERO),
                    Err(error)
                );
            }
        }
        assert_eq!(
            wait_for_completion(&AtomicBool::new(false), || Ok(()), Duration::ZERO),
            Err(CaptureError::ActivationTimeout)
        );
        assert!(wait_for_completion(&AtomicBool::new(true), || Ok(()), Duration::ZERO).is_ok());
    }
    #[test]
    fn completion_after_cancellation_only_updates_retained_flag() {
        let params = activation_params(123);
        let weak = Arc::downgrade(&params);
        let completed = Arc::new(AtomicBool::new(false));
        let handler: IActivateAudioInterfaceCompletionHandler = Completion {
            completed: Arc::clone(&completed),
            _params: Arc::clone(&params),
        }
        .into();
        assert_eq!(
            wait_for_completion(&completed, || Err(CaptureError::Cancelled), Duration::ZERO),
            Err(CaptureError::Cancelled)
        );
        drop(params);
        assert!(weak.upgrade().is_some());
        unsafe {
            handler
                .ActivateCompleted(None::<&IActivateAudioInterfaceAsyncOperation>)
                .unwrap();
        }
        assert!(completed.load(Ordering::Acquire));
        drop(handler);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn borrowed_blob_drop_does_not_free_rust_activation_data() {
        let params = activation_params(123);
        {
            let variant = borrowed_activation_variant(&params);
            let blob = unsafe { &variant.Anonymous.Anonymous.Anonymous.blob };
            assert_eq!(blob.pBlobData, Arc::as_ptr(&params).cast_mut().cast());
            assert_eq!(
                blob.cbSize as usize,
                size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()
            );
        }
        assert_eq!(
            unsafe { params.Anonymous.ProcessLoopbackParams.TargetProcessId },
            123
        );
        assert_eq!(Arc::strong_count(&params), 1);
    }

    #[test]
    fn agile_completion_keeps_parameters_alive_until_last_reference() {
        let params = activation_params(123);
        let completed = Arc::new(AtomicBool::new(false));
        let handler: IActivateAudioInterfaceCompletionHandler = Completion {
            completed: Arc::clone(&completed),
            _params: Arc::clone(&params),
        }
        .into();
        let agile = handler.cast::<IAgileObject>().unwrap();
        assert_eq!(Arc::strong_count(&params), 2);
        unsafe {
            handler
                .ActivateCompleted(None::<&IActivateAudioInterfaceAsyncOperation>)
                .unwrap();
        }
        assert!(completed.load(Ordering::Acquire));
        drop(handler);
        assert_eq!(Arc::strong_count(&params), 2);
        drop(agile);
        assert_eq!(Arc::strong_count(&params), 1);
    }
}
