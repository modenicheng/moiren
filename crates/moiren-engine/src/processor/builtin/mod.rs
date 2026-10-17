//! Built-in realtime DSP processors.
mod channel_mapper;
mod compressor;
mod delay;
mod eq;
mod gain;
mod gate;
mod limiter;
mod pan;
mod passthrough;
mod strip;
mod sum;

pub use compressor::{Compressor, CompressorSettings};
pub use delay::{Delay, DelaySettings};
pub use eq::{Eq, EqSettings};
pub use gain::Gain;
pub use gate::{Gate, GateSettings};
pub use limiter::{Limiter, LimiterSettings};
pub use pan::Pan;
pub(crate) use passthrough::PassThrough;
pub(crate) use strip::ChannelStrip;
pub use strip::{ChannelMode, StripConfig};
/// Bus and Sum share the same prepared mixing kernel. Dynamic logical input
/// editing stays in core; runtime inputs are fixed by the prepared IO bindings.
pub use sum::Sum as Bus;
pub use sum::Sum;

/// Built-in state is fixed-size so `process` never allocates for a new graph
/// channel count. Larger layouts need a processor-specific prepared contract.
pub(crate) const MAX_CHANNELS: usize = 256;
pub use channel_mapper::{ChannelMapper, ChannelMapperSettings};
