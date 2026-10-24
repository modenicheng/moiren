//! Bounded little-endian PCM/IEEE conversion, including left-aligned valid bits.
#[cfg(any(windows, test))]
use moiren_engine::audio_config::SampleFormat;
#[cfg(any(windows, test))]
use thiserror::Error;

#[cfg(any(windows, test))]
use super::native::StereoFrame;

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CodecError {
    #[error("unsupported PCM layout")]
    InvalidLayout,
    #[error("sample data does not contain whole native frames")]
    PartialFrame,
    #[error("destination is smaller than the requested frame count")]
    ShortBuffer,
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy)]
pub struct SampleCodec {
    format: SampleFormat,
    channels: usize,
    container_bytes: usize,
    valid_bits: u16,
}

#[cfg(any(windows, test))]
impl SampleCodec {
    /// Convenience constructor for the canonical layouts used by the test
    /// harnesses; production builds negotiate via [`SampleCodec::with_layout`]
    /// from the native format descriptor.
    #[cfg(test)]
    pub fn new(format: SampleFormat, channels: usize) -> Result<Self, CodecError> {
        let bits = match format {
            SampleFormat::Pcm16 => 16,
            SampleFormat::Pcm24 => 24,
            SampleFormat::Pcm32 | SampleFormat::Float32 => 32,
            SampleFormat::Float64 => 64,
            SampleFormat::Native => return Err(CodecError::InvalidLayout),
        };
        Self::with_layout(format, channels, bits, bits)
    }

    pub fn with_layout(
        format: SampleFormat,
        channels: usize,
        container_bits: u16,
        valid_bits: u16,
    ) -> Result<Self, CodecError> {
        let valid = match format {
            SampleFormat::Pcm16 => container_bits == 16 && valid_bits == 16,
            SampleFormat::Pcm24 => [24, 32].contains(&container_bits) && valid_bits == 24,
            SampleFormat::Pcm32 => container_bits == 32 && (1..=32).contains(&valid_bits),
            SampleFormat::Float32 => container_bits == 32 && valid_bits == 32,
            SampleFormat::Float64 => container_bits == 64 && valid_bits == 64,
            SampleFormat::Native => false,
        };
        if !valid || !(1..=32).contains(&channels) {
            return Err(CodecError::InvalidLayout);
        }
        Ok(Self {
            format,
            channels,
            container_bytes: usize::from(container_bits / 8),
            valid_bits,
        })
    }

    pub fn frame_bytes(self) -> usize {
        self.channels * self.container_bytes
    }

    pub fn decode(self, input: &[u8], output: &mut [StereoFrame]) -> Result<usize, CodecError> {
        if !input.len().is_multiple_of(self.frame_bytes()) {
            return Err(CodecError::PartialFrame);
        }
        let frames = input.len() / self.frame_bytes();
        if output.len() < frames {
            return Err(CodecError::ShortBuffer);
        }
        for (native, stereo) in input.chunks_exact(self.frame_bytes()).zip(output) {
            stereo[0] = self.decode_sample(&native[..self.container_bytes]);
            stereo[1] = if self.channels == 1 {
                stereo[0]
            } else {
                self.decode_sample(&native[self.container_bytes..2 * self.container_bytes])
            };
        }
        Ok(frames)
    }

    pub fn encode(self, input: &[StereoFrame], output: &mut [u8]) -> Result<usize, CodecError> {
        let bytes = input
            .len()
            .checked_mul(self.frame_bytes())
            .ok_or(CodecError::ShortBuffer)?;
        if output.len() < bytes {
            return Err(CodecError::ShortBuffer);
        }
        for (stereo, native) in input
            .iter()
            .zip(output[..bytes].chunks_exact_mut(self.frame_bytes()))
        {
            native.fill(0);
            let left = if self.channels == 1 {
                finite(stereo[0]) * 0.5 + finite(stereo[1]) * 0.5
            } else {
                stereo[0]
            };
            self.encode_sample(left, &mut native[..self.container_bytes]);
            if self.channels > 1 {
                self.encode_sample(
                    stereo[1],
                    &mut native[self.container_bytes..2 * self.container_bytes],
                );
            }
        }
        Ok(input.len())
    }

    fn decode_sample(self, bytes: &[u8]) -> f64 {
        match self.format {
            SampleFormat::Float32 => finite(f32::from_le_bytes(bytes.try_into().unwrap()) as f64),
            SampleFormat::Float64 => finite(f64::from_le_bytes(bytes.try_into().unwrap())),
            _ => {
                let value = match self.container_bytes {
                    2 => i16::from_le_bytes(bytes.try_into().unwrap()) as i32,
                    3 => {
                        ((u32::from(bytes[0])
                            | (u32::from(bytes[1]) << 8)
                            | (u32::from(bytes[2]) << 16))
                            << 8) as i32
                            >> 8
                    }
                    4 => i32::from_le_bytes(bytes.try_into().unwrap()),
                    _ => unreachable!("validated PCM container"),
                };
                let shift = self.container_bytes * 8 - usize::from(self.valid_bits);
                f64::from(value >> shift) / (1u64 << (self.valid_bits - 1)) as f64
            }
        }
    }

    fn encode_sample(self, sample: f64, bytes: &mut [u8]) {
        let sample = finite(sample);
        match self.format {
            SampleFormat::Float32 => bytes.copy_from_slice(
                &(sample.clamp(-(f32::MAX as f64), f32::MAX as f64) as f32).to_le_bytes(),
            ),
            SampleFormat::Float64 => bytes.copy_from_slice(&sample.to_le_bytes()),
            _ => {
                let scale = (1u64 << (self.valid_bits - 1)) as f64;
                let quantized = (sample.clamp(-1.0, 1.0) * scale)
                    .round()
                    .clamp(-scale, scale - 1.0) as i32;
                let shift = self.container_bytes * 8 - usize::from(self.valid_bits);
                let packed = quantized.wrapping_shl(shift as u32).to_le_bytes();
                bytes.copy_from_slice(&packed[..self.container_bytes]);
            }
        }
    }
}

pub(super) fn finite(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codecs_round_trip_endpoints_and_stereo() {
        for format in [
            SampleFormat::Pcm16,
            SampleFormat::Pcm24,
            SampleFormat::Pcm32,
            SampleFormat::Float32,
            SampleFormat::Float64,
        ] {
            let codec = SampleCodec::new(format, 2).unwrap();
            let input = [[-1.0, 1.0], [0.0, -0.25], [0.5, 0.125]];
            let mut bytes = [0u8; 48];
            let len = input.len() * codec.frame_bytes();
            codec.encode(&input, &mut bytes[..len]).unwrap();
            let mut output = [[0.0; 2]; 3];
            codec.decode(&bytes[..len], &mut output).unwrap();
            for (expected, actual) in input
                .into_iter()
                .flatten()
                .zip(output.into_iter().flatten())
            {
                assert!((expected - actual).abs() <= 1.0 / 32768.0);
            }
        }
    }

    #[test]
    fn pcm24_sign_extension_and_left_aligned_32_bit_container() {
        let packed = SampleCodec::new(SampleFormat::Pcm24, 1).unwrap();
        let mut out = [[0.0; 2]; 2];
        packed
            .decode(&[0, 0, 0x80, 0xff, 0xff, 0x7f], &mut out)
            .unwrap();
        assert_eq!(out[0], [-1.0; 2]);
        assert_eq!(out[1], [1.0 - 1.0 / 8388608.0; 2]);
        let padded = SampleCodec::with_layout(SampleFormat::Pcm24, 1, 32, 24).unwrap();
        let mut bytes = [0xff; 8];
        padded.encode(&out, &mut bytes).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 0, 0x80]);
        assert_eq!(&bytes[4..], &[0, 0xff, 0xff, 0x7f]);
        let original = out;
        padded.decode(&bytes, &mut out).unwrap();
        assert_eq!(out, original);
    }

    #[test]
    fn malformed_buffers_do_not_write_and_float64_keeps_precision() {
        let codec = SampleCodec::new(SampleFormat::Float64, 2).unwrap();
        let value = 1.0 + f64::EPSILON;
        let mut bytes = [0; 16];
        codec.encode(&[[value, f64::NAN]], &mut bytes).unwrap();
        let mut output = [[9.0; 2]];
        assert_eq!(
            codec.decode(&bytes[..15], &mut output),
            Err(CodecError::PartialFrame)
        );
        assert_eq!(output, [[9.0; 2]]);
        codec.decode(&bytes, &mut output).unwrap();
        assert_eq!(output, [[value, 0.0]]);
        assert!(SampleCodec::new(SampleFormat::Native, 2).is_err());
    }

    #[test]
    fn extra_native_channels_are_silent_and_mono_downmixes() {
        let codec = SampleCodec::new(SampleFormat::Float32, 6).unwrap();
        let mut bytes = [0xff; 24];
        codec.encode(&[[0.25, -0.5]], &mut bytes).unwrap();
        assert_eq!(&bytes[8..], &[0; 16]);
        let mono = SampleCodec::new(SampleFormat::Pcm16, 1).unwrap();
        mono.encode(&[[0.75, -0.25]], &mut bytes).unwrap();
        assert_eq!(&bytes[..2], &8192i16.to_le_bytes());
    }
}
