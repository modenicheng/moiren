#[cfg(windows)]
use super::{TelemetryMode, qpc_ticks_to_100ns};
#[cfg(windows)]
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

#[derive(Debug, Clone, Copy)]
pub struct QpcClock {
    #[cfg(windows)]
    frequency: u64,
}
impl QpcClock {
    /// Always-on timing source, independent of the telemetry policy.
    #[cfg(windows)]
    pub fn new() -> Option<Self> {
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.ok()?;
        let frequency = u64::try_from(frequency).ok().filter(|value| *value > 0)?;
        Some(Self { frequency })
    }
    #[cfg(not(windows))]
    pub fn new() -> Option<Self> {
        None
    }
    #[cfg(windows)]
    pub fn prepare(mode: TelemetryMode) -> Option<Self> {
        (mode != TelemetryMode::Disabled).then(Self::new).flatten()
    }
    pub fn now(&self) -> Option<u64> {
        #[cfg(not(windows))]
        {
            None
        }
        #[cfg(windows)]
        {
            let mut ticks = 0;
            unsafe { QueryPerformanceCounter(&mut ticks) }.ok()?;
            qpc_ticks_to_100ns(u64::try_from(ticks).ok()?, self.frequency)
        }
    }
}
