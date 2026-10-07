use std::ops::{AddAssign, MulAssign};

pub trait ProcessingSample:
    Copy + Send + Sync + AddAssign + MulAssign + 'static
{
    const ZERO: Self;
}

impl ProcessingSample for f32 {
    const ZERO: Self = 0.0;
}

impl ProcessingSample for f64 {
    const ZERO: Self = 0.0;
}
