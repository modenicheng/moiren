//! Creation owns callback storage until SwDeviceClose has joined callbacks.
use super::{ENUMERATOR, unexpected};
use crate::owner::OwnedHandle;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        Devices::Enumeration::Pnp::{
            HSWDEVICE, SW_DEVICE_CREATE_INFO, SWDeviceCapabilitiesDriverRequired,
            SWDeviceCapabilitiesNoDisplayInUI, SWDeviceCapabilitiesRemovable,
            SWDeviceCapabilitiesSilentInstall, SwDeviceClose, SwDeviceCreate,
        },
        Foundation::WAIT_OBJECT_0,
        System::Threading::{SetEvent, WaitForSingleObject},
    },
    core::{HRESULT, HSTRING, PCWSTR, Result, w},
};

#[derive(Clone)]
pub(super) struct Completion {
    pub(super) result: HRESULT,
    pub(super) id: Option<String>,
}
struct Context {
    event: OwnedHandle,
    completion: Mutex<Option<Completion>>,
    panicked: AtomicBool,
}
unsafe extern "system" fn completed(
    _: HSWDEVICE,
    result: HRESULT,
    context: *const c_void,
    id: PCWSTR,
) {
    if context.is_null() {
        return;
    }
    // The box stays alive through SwDeviceClose, which joins late callbacks.
    let context = unsafe { &*context.cast::<Context>() };
    if catch_unwind(AssertUnwindSafe(|| {
        let id = if id.is_null() {
            None
        } else {
            unsafe { id.to_string().ok() }
        };
        *context.completion.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Completion { result, id });
    }))
    .is_err()
    {
        context.panicked.store(true, Ordering::Release);
    }
    let _ = unsafe { SetEvent(context.event.0) };
}

struct SwLease(Option<HSWDEVICE>);
impl SwLease {
    fn close(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe { SwDeviceClose(handle) };
        }
    }
}
impl Drop for SwLease {
    fn drop(&mut self) {
        self.close();
    }
}
pub(super) struct Created {
    // Struct fields drop in declaration order: close/join before freeing context.
    lease: SwLease,
    context: Box<Context>,
}
impl Created {
    pub(super) fn handle(&self) -> Result<HSWDEVICE> {
        self.lease.0.ok_or_else(unexpected)
    }

    pub(super) fn close(&mut self) {
        self.lease.close();
    }

    pub(super) fn completion(&self) -> Option<Completion> {
        // SwDeviceClose joins callbacks before the owner reads late completion.
        self.context
            .completion
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(super) fn wait(&self) -> Result<Completion> {
        if unsafe { WaitForSingleObject(self.context.event.0, 5_000) } != WAIT_OBJECT_0 {
            return Err(windows::core::Error::from_hresult(HRESULT::from_win32(
                1460,
            )));
        }
        if self.context.panicked.load(Ordering::Acquire) {
            return Err(unexpected());
        }
        self.completion().ok_or_else(unexpected)
    }
}

pub(super) fn create(instance: &str, driver_required: bool) -> Result<Created> {
    let context = Box::new(Context {
        event: OwnedHandle::event()?,
        completion: Mutex::new(None),
        panicked: AtomicBool::new(false),
    });
    let instance = HSTRING::from(instance);
    let info = SW_DEVICE_CREATE_INFO {
        cbSize: size_of::<SW_DEVICE_CREATE_INFO>() as u32,
        pszInstanceId: PCWSTR(instance.as_ptr()),
        // No installed audio package matches this W00-only hardware identity.
        pszzHardwareIds: w!("Moiren\\W00NoDriver\0"),
        pszzCompatibleIds: w!("Moiren\\W00Probe\0"),
        CapabilityFlags: (SWDeviceCapabilitiesRemovable.0
            | SWDeviceCapabilitiesSilentInstall.0
            | SWDeviceCapabilitiesNoDisplayInUI.0
            | if driver_required {
                SWDeviceCapabilitiesDriverRequired.0
            } else {
                0
            }) as u32,
        pszDeviceDescription: w!("Moiren W00 temporary lifecycle probe - no audio driver"),
        ..Default::default()
    };
    let handle = unsafe {
        SwDeviceCreate(
            &HSTRING::from(ENUMERATOR),
            w!("HTREE\\ROOT\\0"),
            &info,
            None,
            Some(completed),
            Some((&*context as *const Context).cast()),
        )?
    };
    Ok(Created {
        lease: SwLease(Some(handle)),
        context,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_records_identity_before_create_returns_without_real_device() {
        let context = Context {
            event: OwnedHandle::event().unwrap(),
            completion: Mutex::new(None),
            panicked: AtomicBool::new(false),
        };
        unsafe {
            completed(
                HSWDEVICE::default(),
                HRESULT(0),
                (&context as *const Context).cast(),
                w!("SWD\\MOIRENW00\\W00-abc"),
            );
        }
        assert_eq!(
            unsafe { WaitForSingleObject(context.event.0, 100) },
            WAIT_OBJECT_0
        );
        assert_eq!(
            context
                .completion
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .id
                .as_deref(),
            Some("SWD\\MOIRENW00\\W00-abc")
        );
    }
}
