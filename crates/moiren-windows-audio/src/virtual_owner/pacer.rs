use crate::StopSignal;
use moiren_engine::host::BackendError;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0},
    System::{
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Threading::{CreateWaitableTimerW, INFINITE, SetWaitableTimer, WaitForMultipleObjects},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockWake {
    Timer,
    Stopped,
}

/// Monotonic ticks and an interruptible wait. This never owns another Engine.
pub trait VirtualRealtimeClock: Send {
    fn frequency(&self) -> u64;
    fn now_ticks(&mut self) -> Result<u64, BackendError>;
    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError>;
}

pub trait VirtualClockFactory: Send + Sync {
    fn prepare(&self, stop: StopSignal) -> Result<Box<dyn VirtualRealtimeClock>, BackendError>;
}

#[derive(Default)]
pub struct QpcClockFactory;

impl VirtualClockFactory for QpcClockFactory {
    fn prepare(&self, stop: StopSignal) -> Result<Box<dyn VirtualRealtimeClock>, BackendError> {
        let mut frequency = 0;
        // SAFETY: writable local scalar, no asynchronous reference is retained.
        unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(api)?;
        if frequency <= 0 {
            return Err(BackendError::Failed("invalid QPC frequency".into()));
        }
        // SAFETY: unnamed non-inheritable synchronization timer; unique owner.
        let timer = unsafe { CreateWaitableTimerW(None, false, None) }.map_err(api)?;
        Ok(Box::new(QpcClock {
            // SAFETY: ownership is transferred exactly once after a successful open.
            timer: unsafe { OwnedHandle::from_raw_handle(timer.0) },
            stop,
            frequency: frequency as u64,
        }))
    }
}

struct QpcClock {
    timer: OwnedHandle,
    stop: StopSignal,
    frequency: u64,
}

fn api(error: windows::core::Error) -> BackendError {
    BackendError::ResourceUnavailable(format!("virtual owner clock: {error}"))
}

impl VirtualRealtimeClock for QpcClock {
    fn frequency(&self) -> u64 {
        self.frequency
    }

    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        let mut ticks = 0;
        // SAFETY: this call only writes the local scalar.
        unsafe { QueryPerformanceCounter(&mut ticks) }.map_err(api)?;
        u64::try_from(ticks).map_err(|_| BackendError::Failed("negative QPC counter".into()))
    }

    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
        let intervals = (u128::from(ticks) * 10_000_000)
            .div_ceil(u128::from(self.frequency))
            .max(1);
        let due = -i64::try_from(intervals)
            .map_err(|_| BackendError::Failed("virtual clock wait overflow".into()))?;
        // SAFETY: negative relative 100ns due time, one shot, no callback/APC.
        // The timer and stop handle stay owned across the wait.
        unsafe {
            SetWaitableTimer(
                HANDLE(self.timer.as_raw_handle()),
                &due,
                0,
                None,
                None,
                false,
            )
        }
        .map_err(api)?;
        let handles = [
            HANDLE(self.stop.event.as_raw_handle()),
            HANDLE(self.timer.as_raw_handle()),
        ];
        // Stop has the lower index, so it wins if both objects are signalled.
        match unsafe { WaitForMultipleObjects(&handles, false, INFINITE) } {
            value if value == WAIT_OBJECT_0 => Ok(ClockWake::Stopped),
            value if value.0 == WAIT_OBJECT_0.0 + 1 => Ok(ClockWake::Timer),
            _ => Err(api(windows::core::Error::from_thread())),
        }
    }
}
