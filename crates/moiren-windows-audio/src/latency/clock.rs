use super::{TelemetryMode, qpc_ticks_to_100ns};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub(crate) struct QpcClock {
    frequency: u64,
}
impl QpcClock {
    pub fn prepare(mode: TelemetryMode) -> Option<Self> {
        if mode == TelemetryMode::Disabled {
            return None;
        }
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.ok()?;
        let frequency = u64::try_from(frequency).ok().filter(|value| *value > 0)?;
        Some(Self { frequency })
    }
    pub fn now(&self) -> Option<u64> {
        let mut ticks = 0;
        unsafe { QueryPerformanceCounter(&mut ticks) }.ok()?;
        qpc_ticks_to_100ns(u64::try_from(ticks).ok()?, self.frequency)
    }
}
