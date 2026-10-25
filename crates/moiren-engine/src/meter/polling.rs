//! Control-side timer precision for demand-gated meter delivery.

/// Keeps short control-thread waits usable on Windows while meters have demand.
/// Windows condition-variable/parking timeouts can otherwise round a 1 ms
/// interval up to the default system timer tick. This request is never made on
/// the audio thread and is released as soon as demand ends or the owner drops.
#[derive(Default)]
pub struct MeterPollingTimer {
    #[cfg(windows)]
    active: bool,
}

impl MeterPollingTimer {
    pub fn set_demand(&mut self, demanded: bool) {
        #[cfg(windows)]
        {
            if demanded == self.active {
                return;
            }
            if demanded {
                // SAFETY: winmm takes a scalar millisecond period, retains no
                // pointers, and each successful request has one matching end.
                self.active = unsafe { timeBeginPeriod(1) } == 0;
            } else {
                // SAFETY: active records a successful, still-owned request.
                unsafe { timeEndPeriod(1) };
                self.active = false;
            }
        }
        #[cfg(not(windows))]
        let _ = demanded;
    }
}

impl Drop for MeterPollingTimer {
    fn drop(&mut self) {
        self.set_demand(false);
    }
}

#[cfg(windows)]
#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}
