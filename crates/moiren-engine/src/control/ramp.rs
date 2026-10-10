#[derive(Debug, Clone, Copy)]
pub struct FloatRamp {
    from: f64,
    target: f64,
    elapsed: u32,
    duration: u32,
}
impl FloatRamp {
    pub fn constant(value: f64) -> Self {
        Self {
            from: value,
            target: value,
            elapsed: 0,
            duration: 0,
        }
    }
    fn at_progress(self, progress: u64) -> f64 {
        if self.duration == 0 || progress >= u64::from(self.duration) {
            return self.target;
        }
        let t = progress as f64 / f64::from(self.duration);
        self.from * (1.0 - t) + self.target * t
    }
    /// The first sample advances by one ramp step; sample N reaches the target.
    pub fn sample(self, offset: usize) -> f64 {
        self.at_progress(
            u64::from(self.elapsed)
                .saturating_add(offset as u64)
                .saturating_add(1),
        )
    }
    pub(super) fn retarget(&mut self, target: f64, duration: u32) {
        self.from = self.at_progress(u64::from(self.elapsed));
        self.target = target;
        self.duration = duration;
        self.elapsed = 0;
    }
    pub(super) fn advance(&mut self, frames: usize) {
        self.elapsed = (u64::from(self.elapsed).saturating_add(frames as u64))
            .min(u64::from(self.duration)) as u32;
    }
}
