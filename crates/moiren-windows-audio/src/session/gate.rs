//! One manual-reset event releases all prepared owners; stop wins every race.
use super::StopSignal;
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use thiserror::Error;
use windows::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects},
};

const PREPARED: u8 = 0;
const ACTIVATED: u8 = 1;
const CANCELLED: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    Start,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GateError {
    #[error("{stage} failed (HRESULT 0x{hresult:08X})")]
    Api { stage: &'static str, hresult: i32 },
    #[error("activation gate is already resolved")]
    AlreadyResolved,
}
fn api<T>(stage: &'static str, result: windows::core::Result<T>) -> Result<T, GateError> {
    result.map_err(|error| GateError::Api {
        stage,
        hresult: error.code().0,
    })
}

#[derive(Clone)]
pub struct ActivationGate {
    event: Arc<OwnedHandle>,
    state: Arc<AtomicU8>,
    stop: StopSignal,
}
impl ActivationGate {
    /// The gate and running owner must observe the same stop event, so stopping
    /// also wakes an owner still waiting for activation.
    pub(crate) fn validate_stop(&self, stop: &StopSignal) -> Result<(), GateError> {
        if Arc::ptr_eq(&self.stop.event, &stop.event) {
            Ok(())
        } else {
            Err(GateError::Api {
                stage: "ActivationGate stop identity",
                hresult: 0x80070057u32 as i32, // E_INVALIDARG
            })
        }
    }
    pub fn new(stop: StopSignal) -> Result<Self, GateError> {
        let raw = api("CreateEvent(activation gate)", unsafe {
            CreateEventW(None, true, false, None)
        })?;
        Ok(Self {
            // SAFETY: CreateEvent returned a unique valid kernel handle.
            event: Arc::new(unsafe { OwnedHandle::from_raw_handle(raw.0) }),
            state: Arc::new(AtomicU8::new(PREPARED)),
            stop,
        })
    }
    fn resolve(&self, target: u8) -> Result<(), GateError> {
        self.state
            .compare_exchange(PREPARED, target, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| GateError::AlreadyResolved)?;
        let result = api("SetEvent(activation gate)", unsafe {
            SetEvent(HANDLE(self.event.as_raw_handle()))
        });
        if result.is_err() {
            let _ = self.stop.request_stop();
        }
        result
    }
    pub fn activate(&self) -> Result<(), GateError> {
        self.resolve(ACTIVATED)
    }
    pub fn cancel(&self) -> Result<(), GateError> {
        // Even a losing cancel must wake owners already running after activation.
        let stopped = api("SetEvent(gate stop)", self.stop.request_stop());
        let resolved = self.resolve(CANCELLED);
        stopped?;
        resolved
    }
    pub fn wait(&self) -> Result<Activation, GateError> {
        self.wait_with_target(None)
    }
    /// A selected process may exit while a prepared capture waits for its peer.
    pub(crate) fn wait_with_target(&self, target: Option<HANDLE>) -> Result<Activation, GateError> {
        let handles = [
            HANDLE(self.stop.event.as_raw_handle()),
            target.unwrap_or(HANDLE(self.event.as_raw_handle())),
            HANDLE(self.event.as_raw_handle()),
        ];
        let handles = if target.is_some() {
            &handles[..]
        } else {
            &handles[..2]
        };
        match unsafe { WaitForMultipleObjects(handles, false, INFINITE) } {
            event if event == WAIT_OBJECT_0 => Ok(Activation::Cancelled),
            event if event.0 == WAIT_OBJECT_0.0 + 1 && target.is_some() => {
                Ok(Activation::Cancelled)
            }
            event if event.0 == WAIT_OBJECT_0.0 + handles.len() as u32 - 1 => {
                Ok(if self.state.load(Ordering::Acquire) == ACTIVATED {
                    Activation::Start
                } else {
                    Activation::Cancelled
                })
            }
            _ => Err(GateError::Api {
                stage: "WaitForMultipleObjects(activation gate)",
                hresult: windows::core::Error::from_thread().code().0,
            }),
        }
    }
}
