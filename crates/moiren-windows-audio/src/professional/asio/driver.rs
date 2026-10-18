//! Apartment and IASIO reference ownership stay on the native owner thread.

use std::{ffi::c_void, marker::PhantomData, ptr, ptr::NonNull, rc::Rc};
use windows::{
    Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    core::{GUID, HRESULT},
};

use super::{BackendError, abi};

// Unlike the generic windows wrapper, IASIO uses its driver CLSID as its IID.
// The raw declaration avoids inventing one global Interface::IID for all
// drivers. Only this activation call and IUnknown's Release cross COM proper.
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoCreateInstance(
        class: *const GUID,
        outer: *mut c_void,
        context: u32,
        interface: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT;
}

pub(super) struct Apartment(PhantomData<Rc<()>>);

impl Apartment {
    pub fn new() -> Result<Self, BackendError> {
        // SAFETY: exactly one matching CoUninitialize follows S_OK or S_FALSE;
        // a failed initialization (including changed apartment mode) owns none.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|error| {
                BackendError::Failed(format!("ASIO owner COM initialization failed: {error}"))
            })?;
        Ok(Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub(super) struct Driver {
    raw: NonNull<abi::Driver>,
    _owner_thread: PhantomData<Rc<()>>,
}

impl Driver {
    pub fn open(clsid: &GUID) -> Result<Self, BackendError> {
        let mut raw = ptr::null_mut();
        // SAFETY: class and interface are the same registered ASIO CLSID, as
        // specified by Steinberg's host/pc/asiolist.cpp. CLSCTX_INPROC_SERVER=1.
        unsafe { CoCreateInstance(clsid, ptr::null_mut(), 1, clsid, &mut raw) }
            .ok()
            .map_err(|error| {
                BackendError::ResourceUnavailable(format!(
                    "cannot activate ASIO driver {{{clsid:?}}}: {error}; \
                     install a matching 64-bit driver and check that its device is connected"
                ))
            })?;
        let raw = NonNull::new(raw.cast()).ok_or_else(|| {
            BackendError::Failed("ASIO CoCreateInstance returned a null interface".into())
        })?;
        Ok(Self {
            raw,
            _owner_thread: PhantomData,
        })
    }

    pub fn raw(&self) -> *mut abi::Driver {
        self.raw.as_ptr()
    }

    pub fn vtable(&self) -> &abi::VTable {
        // SAFETY: one successful activation reference is retained through
        // Release, and no interface escapes to another owner thread.
        unsafe { &*self.raw.as_ref().vtable }
    }

    pub fn initialize(&self) -> Result<(), BackendError> {
        // ASIO accepts a null system handle. Drivers requiring a window handle
        // fail explicitly; this headless backend does not invent an HWND.
        let initialized = unsafe { (self.vtable().init)(self.raw(), ptr::null_mut()) };
        if initialized == 0 {
            Err(BackendError::ResourceUnavailable(format!(
                "ASIO driver init failed: {}",
                self.message()
            )))
        } else {
            Ok(())
        }
    }

    fn message(&self) -> String {
        let mut bytes = [0u8; 124]; // ASIO's required error-message buffer.
        unsafe { (self.vtable().get_error_message)(self.raw(), bytes.as_mut_ptr()) };
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }

    pub fn check(&self, operation: &str, code: i32) -> Result<(), BackendError> {
        if code == abi::OK {
            return Ok(());
        }
        let message = format!("ASIO {operation} failed ({code}): {}", self.message());
        Err(match code {
            -1000 => BackendError::ResourceUnavailable(message),
            -998 | -997 | -995 => BackendError::Unsupported(message),
            _ => BackendError::Failed(message),
        })
    }

    #[cfg(test)]
    pub unsafe fn from_test_raw(raw: *mut abi::Driver) -> Self {
        Self {
            raw: NonNull::new(raw).unwrap(),
            _owner_thread: PhantomData,
        }
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        // The stream retires callbacks, stops and disposes before final Release.
        // No SDK ASIOExit wrapper or cross-thread COM Release is involved.
        unsafe { (self.vtable().release)(self.raw()) };
    }
}
