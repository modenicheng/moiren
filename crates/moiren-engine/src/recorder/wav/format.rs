use super::MAX_CHANNELS;

/// Encoding performed only by the control-side WAV writer. PCM uses symmetric
/// power-of-two scaling, nearest rounding (ties away from zero), and saturation
/// to the signed integer range. Every non-finite PCM input becomes silence.
/// No dither or noise shaping is applied. Float32 preserves the input bits,
/// including non-finite values, for compatibility with the original writer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WavFormat {
    #[default]
    Float32,
    Pcm24,
    Pcm16,
}

impl WavFormat {
    pub const fn bytes_per_sample(self) -> u16 {
        match self {
            Self::Float32 => 4,
            Self::Pcm24 => 3,
            Self::Pcm16 => 2,
        }
    }

    /// Bytes preceding audio data: WAVEFORMATEX/fact for float, extensible PCM otherwise.
    pub const fn header_bytes(self) -> u32 {
        match self {
            Self::Float32 => 58,
            Self::Pcm24 | Self::Pcm16 => 68,
        }
    }

    /// Maximum complete frames including RIFF's headers and final word padding.
    /// Shared with capacity estimates so integer sample widths cannot overstate
    /// the available duration, including odd-width mono PCM24 recordings.
    pub fn max_frames(self, channels: usize) -> Option<u64> {
        if !(1..=MAX_CHANNELS).contains(&channels) {
            return None;
        }
        Some(self.max_data_bytes() / (channels as u64 * u64::from(self.bytes_per_sample())))
    }

    pub(super) fn max_data_bytes(self) -> u64 {
        // RIFF's size is even after padding; u32::MAX itself cannot be used.
        (u64::from(u32::MAX) - (u64::from(self.header_bytes()) - 8)) & !1
    }

    pub(super) fn encode(self, sample: f32, destination: &mut [u8]) {
        match self {
            Self::Float32 => destination.copy_from_slice(&sample.to_le_bytes()),
            Self::Pcm24 | Self::Pcm16 => {
                let scale = 1i32 << (self.bytes_per_sample() * 8 - 1);
                let value = if sample.is_finite() {
                    (f64::from(sample) * f64::from(scale))
                        .round()
                        .clamp(f64::from(-scale), f64::from(scale - 1)) as i32
                } else {
                    0
                };
                destination
                    .copy_from_slice(&value.to_le_bytes()[..self.bytes_per_sample() as usize]);
            }
        }
    }
}
