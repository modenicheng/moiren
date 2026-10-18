//! Sample conversion runs on the owner thread, outside the ASIO callback.

use moiren_engine::{audio_config::SampleFormat, host::BackendError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer(u32),
    Float32,
    Float64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Encoding {
    kind: Kind,
    pub bytes: usize,
    big_endian: bool,
}

/// Two native channel samples, with unused bytes zeroed. A callback copies only
/// the driver's actual sample width; it never converts floats or allocates.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg(all(windows, target_pointer_width = "64"))]
pub(super) struct Frame {
    pub samples: [[u8; 8]; 2],
}

impl Encoding {
    pub fn from_asio(sample_type: i32) -> Result<Self, BackendError> {
        let (kind, bytes) = match sample_type {
            0 | 16 => (Kind::Integer(16), 2),
            1 | 17 => (Kind::Integer(24), 3),
            2 | 18 => (Kind::Integer(32), 4),
            3 | 19 => (Kind::Float32, 4),
            4 | 20 => (Kind::Float64, 8),
            // ASIO's reduced-resolution 32-bit containers are right-aligned.
            8 | 24 => (Kind::Integer(16), 4),
            11 | 27 => (Kind::Integer(24), 4),
            _ => {
                return Err(BackendError::Unsupported(format!(
                    "ASIO sample type {sample_type} cannot be represented by the supported \
                     PCM16/24/32 or float32/64 formats (DSD and 18/20-bit PCM are unsupported)"
                )));
            }
        };
        Ok(Self {
            kind,
            bytes,
            big_endian: sample_type < 16,
        })
    }

    pub fn format(self) -> SampleFormat {
        match self.kind {
            Kind::Integer(16) => SampleFormat::Pcm16,
            Kind::Integer(24) => SampleFormat::Pcm24,
            Kind::Integer(_) => SampleFormat::Pcm32,
            Kind::Float32 => SampleFormat::Float32,
            Kind::Float64 => SampleFormat::Float64,
        }
    }

    pub fn decode(self, bytes: &[u8; 8]) -> f64 {
        let mut word = 0u64;
        for &byte in &bytes[..self.bytes] {
            if self.big_endian {
                word = (word << 8) | u64::from(byte);
            }
        }
        if !self.big_endian {
            word = u64::from_le_bytes(*bytes);
        }
        let sample = match self.kind {
            Kind::Integer(bits) => {
                // Sign extend the effective width, including right-aligned
                // PCM16/24 in 32-bit containers and packed three-byte PCM24.
                let signed = ((word << (64 - bits)) as i64) >> (64 - bits);
                signed as f64 / (1u64 << (bits - 1)) as f64
            }
            Kind::Float32 => f64::from(f32::from_bits(word as u32)),
            Kind::Float64 => f64::from_bits(word),
        };
        if sample.is_finite() { sample } else { 0.0 }
    }

    pub fn encode(self, sample: f64) -> [u8; 8] {
        let sample = if sample.is_finite() { sample } else { 0.0 };
        let word = match self.kind {
            Kind::Integer(bits) => {
                let scale = (1u64 << (bits - 1)) as f64;
                // +1 cannot be represented in signed PCM; saturate rather
                // than overflowing into the negative full-scale code.
                let signed = (sample.clamp(-1.0, 1.0) * scale)
                    .round()
                    .clamp(-scale, scale - 1.0) as i64;
                if self.bytes == 4 && bits < 32 {
                    // ASIOSTInt32*16/*24 are sign-extended right-aligned
                    // 32-bit words, including their unused most-significant
                    // bits. Packed Int24 remains three bytes wide.
                    (signed as i32 as u32) as u64
                } else {
                    signed as u64
                }
            }
            Kind::Float32 => u64::from(
                (sample.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32).to_bits(),
            ),
            Kind::Float64 => sample.to_bits(),
        };
        let mut result = [0; 8];
        if self.big_endian {
            let source = word.to_be_bytes();
            result[..self.bytes].copy_from_slice(&source[8 - self.bytes..]);
        } else {
            result[..self.bytes].copy_from_slice(&word.to_le_bytes()[..self.bytes]);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_pcm_has_correct_byte_order_sign_extension_and_saturation() {
        for (code, negative_half, positive_max) in [
            (0, vec![0xc0, 0], vec![0x7f, 0xff]),
            (16, vec![0, 0xc0], vec![0xff, 0x7f]),
            (1, vec![0xc0, 0, 0], vec![0x7f, 0xff, 0xff]),
            (17, vec![0, 0, 0xc0], vec![0xff, 0xff, 0x7f]),
            (2, vec![0xc0, 0, 0, 0], vec![0x7f, 0xff, 0xff, 0xff]),
            (18, vec![0, 0, 0, 0xc0], vec![0xff, 0xff, 0xff, 0x7f]),
        ] {
            let encoding = Encoding::from_asio(code).unwrap();
            let bytes = encoding.encode(-0.5);
            assert_eq!(&bytes[..encoding.bytes], negative_half);
            assert_eq!(encoding.decode(&bytes), -0.5);
            assert_eq!(&encoding.encode(2.0)[..encoding.bytes], positive_max);
            assert_eq!(encoding.decode(&encoding.encode(-2.0)), -1.0);
        }
    }

    #[test]
    fn reduced_resolution_int32_containers_are_sign_extended_right_aligned() {
        for (code, expected, format) in [
            (8, [0xff, 0xff, 0xc0, 0], SampleFormat::Pcm16),
            (24, [0, 0xc0, 0xff, 0xff], SampleFormat::Pcm16),
            (11, [0xff, 0xc0, 0, 0], SampleFormat::Pcm24),
            (27, [0, 0, 0xc0, 0xff], SampleFormat::Pcm24),
        ] {
            let encoding = Encoding::from_asio(code).unwrap();
            assert_eq!(encoding.format(), format);
            assert_eq!(&encoding.encode(-0.5)[..4], &expected);
            assert_eq!(encoding.decode(&encoding.encode(-0.5)), -0.5);
        }
    }

    #[test]
    fn ieee_samples_honor_width_and_endianness_and_silence_non_finite_values() {
        for (code, bytes) in [
            (3, 0.25f32.to_be_bytes().to_vec()),
            (19, 0.25f32.to_le_bytes().to_vec()),
            (4, 0.25f64.to_be_bytes().to_vec()),
            (20, 0.25f64.to_le_bytes().to_vec()),
        ] {
            let encoding = Encoding::from_asio(code).unwrap();
            assert_eq!(&encoding.encode(0.25)[..encoding.bytes], bytes);
            assert_eq!(encoding.decode(&encoding.encode(0.25)), 0.25);
            assert_eq!(encoding.decode(&encoding.encode(f64::NAN)), 0.0);
            let mut invalid = [0; 8];
            let word = if encoding.bytes == 4 {
                u64::from(f32::INFINITY.to_bits())
            } else {
                f64::NAN.to_bits()
            };
            if encoding.big_endian {
                invalid[..encoding.bytes]
                    .copy_from_slice(&word.to_be_bytes()[8 - encoding.bytes..]);
            } else {
                invalid[..encoding.bytes].copy_from_slice(&word.to_le_bytes()[..encoding.bytes]);
            }
            assert_eq!(encoding.decode(&invalid), 0.0);
        }
    }

    #[test]
    fn native_types_round_trip_with_quantization_bound_and_no_stale_padding() {
        for code in [0, 1, 2, 3, 4, 8, 11, 16, 17, 18, 19, 20, 24, 27] {
            let encoding = Encoding::from_asio(code).unwrap();
            for sample in [-1.0, -0.812345, -0.00001, 0.0, 0.00001, 0.812345, 1.0] {
                let tolerance = match encoding.kind {
                    Kind::Integer(bits) => 1.0 / (1u64 << (bits - 1)) as f64,
                    Kind::Float32 => f64::from(f32::EPSILON),
                    Kind::Float64 => f64::EPSILON,
                };
                let bytes = encoding.encode(sample);
                assert!((encoding.decode(&bytes) - sample).abs() <= tolerance);
                assert!(bytes[encoding.bytes..].iter().all(|&byte| byte == 0));
            }
        }
        for code in [-1, 5, 9, 10, 25, 26, 32, 33, 40, 999] {
            assert!(Encoding::from_asio(code).is_err());
        }
    }

    #[test]
    fn float64_keeps_precision_and_ieee_output_retains_finite_headroom() {
        for code in [4, 20] {
            let encoding = Encoding::from_asio(code).unwrap();
            let sample = 1.0 + f64::EPSILON;
            assert_eq!(encoding.decode(&encoding.encode(sample)), sample);
        }
        for code in [3, 19] {
            let encoding = Encoding::from_asio(code).unwrap();
            assert_eq!(encoding.decode(&encoding.encode(1.25)), 1.25);
            assert_eq!(
                encoding.decode(&encoding.encode(f64::MAX)),
                f64::from(f32::MAX)
            );
        }
    }
}
