use super::TelemetryMode;
#[cfg(windows)]
use super::qpc_ticks_to_100ns;
#[cfg(windows)]
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

#[derive(Debug, Clone, Copy)]
pub(crate) struct QpcClock {
    #[cfg(windows)]
    frequency: u64,
}
impl QpcClock {
    pub fn prepare(mode: TelemetryMode) -> Option<Self> {
        if mode == TelemetryMode::Disabled {
            return None;
        }
        #[cfg(not(windows))]
        {
            let _ = mode;
            return None;
        }
        #[cfg(windows)]
        {
            let mut frequency = 0;
            unsafe { QueryPerformanceFrequency(&mut frequency) }.ok()?;
            let frequency = u64::try_from(frequency).ok().filter(|value| *value > 0)?;
            Some(Self { frequency })
        }
    }
    pub fn now(&self) -> Option<u64> {
        #[cfg(not(windows))]
        {
            let _ = self;
            return None;
        }
        #[cfg(windows)]
        {
            let mut ticks = 0;
            unsafe { QueryPerformanceCounter(&mut ticks) }.ok()?;
            qpc_ticks_to_100ns(u64::try_from(ticks).ok()?, self.frequency)
        }
    }
}
