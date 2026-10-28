//! Built-in realtime DSP processors.

/// Dispatch one linked DSP kernel over both transports. Expands to exactly
/// the hand-written match: Separate frames wrap the producer's slot plus a
/// fresh one, in-place frames wrap the storage they replace, and read-only IO
/// is unreachable for a kernel with an output. Monomorphized like any
/// [`crate::buffer::FrameIo`] use — zero dynamic dispatch on the render path.
/// `$name` only labels the validated-input panics.
macro_rules! linked_io {
    ($io:expr, $kernel:expr, $name:literal $(, $arg:expr)* $(,)?) => {
        match $io {
            crate::buffer::ProcessIo::Separate { inputs, mut outputs } => {
                $kernel.process_linked(
                    &mut crate::buffer::SeparateFrames::new(
                        inputs.get(0).expect(concat!("validated ", $name, " input")),
                        outputs.get_mut(0).expect(concat!("validated ", $name, " output")),
                    ),
                    $($arg,)*
                )
            }
            crate::buffer::ProcessIo::InPlace { mut pairs, .. } => $kernel.process_linked(
                &mut crate::buffer::InPlaceFrames::new(
                    pairs.get_mut(0, 0).expect(concat!("validated ", $name, " pair")),
                ),
                $($arg,)*
            ),
            crate::buffer::ProcessIo::ReadOnly { .. } => {
                unreachable!(concat!($name, " requires an output"))
            }
        }
    };
}

mod channel_mapper;
mod compressor;
mod delay;
mod dynamics;
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
/// The prepared mixing kernel. Dynamic logical input editing stays in core;
/// runtime inputs are fixed by the prepared IO bindings.
pub use sum::Sum as Bus;

/// Built-in state is fixed-size so `process` never allocates for a new graph
/// channel count. Larger layouts need a processor-specific prepared contract.
pub(crate) const MAX_CHANNELS: usize = 256;
pub use channel_mapper::{ChannelMapper, ChannelMapperSettings};
