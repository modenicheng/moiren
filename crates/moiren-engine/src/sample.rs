/// Processing precision is independent of device PCM packing.
pub trait ProcessingSample: Copy + Send + Sync + 'static {
    const ZERO: Self;
    fn from_f64(value: f64) -> Self;
    fn to_f64(self) -> f64;
}
impl ProcessingSample for f32 {
    const ZERO: Self = 0.0;
    fn from_f64(value: f64) -> Self {
        value as Self
    }
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ProcessingSample for f64 {
    const ZERO: Self = 0.0;
    fn from_f64(value: f64) -> Self {
        value
    }
    fn to_f64(self) -> f64 {
        self
    }
}
