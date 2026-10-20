use super::{Channels, SampleFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmFormatError {
    InvalidChannels,
    InputLength { expected: usize, actual: usize },
    OutputLength { expected: usize, actual: usize },
}

fn validate_channels(channels: Channels) -> usize {
    match channels {
        Channels::Mono => 1,
        Channels::Stereo => 2,
    }
}

pub fn decode_interleaved(
    input: &[u8],
    output: &mut [f32],
    format: SampleFormat,
    channels: Channels,
) -> Result<(), PcmFormatError> {
    let channels = validate_channels(channels);
    let bytes_per_sample = format.bytes_per_sample();
    if channels == 0 {
        return Err(PcmFormatError::InvalidChannels);
    }
    if !output.len().is_multiple_of(2) {
        return Err(PcmFormatError::OutputLength {
            expected: output.len() + 1,
            actual: output.len(),
        });
    }
    let frames = output.len() / 2;
    let expected_samples = frames
        .checked_mul(channels)
        .ok_or(PcmFormatError::InputLength {
            expected: usize::MAX,
            actual: input.len(),
        })?;
    let expected_bytes =
        expected_samples
            .checked_mul(bytes_per_sample)
            .ok_or(PcmFormatError::InputLength {
                expected: usize::MAX,
                actual: input.len(),
            })?;
    if input.len() != expected_bytes {
        return Err(PcmFormatError::InputLength {
            expected: expected_bytes,
            actual: input.len(),
        });
    }
    for frame in 0..frames {
        for channel in 0..2 {
            output[frame * 2 + channel] = if channel < channels {
                let sample = frame * channels + channel;
                match format {
                    SampleFormat::Pcm16 => {
                        let offset = sample * 2;
                        i16::from_le_bytes([input[offset], input[offset + 1]]) as f32 / 32_768.0
                    }
                    SampleFormat::Float32 => {
                        let offset = sample * 4;
                        f32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
                    }
                }
            } else {
                0.0
            };
        }
    }
    Ok(())
}

pub fn encode_interleaved(
    input: &[f32],
    output: &mut [u8],
    format: SampleFormat,
    channels: Channels,
) -> Result<(), PcmFormatError> {
    let channels = validate_channels(channels);
    if channels == 0 {
        return Err(PcmFormatError::InvalidChannels);
    }
    let frames = input.len() / 2;
    if input.len() != frames * 2 {
        return Err(PcmFormatError::InputLength {
            expected: frames * 2,
            actual: input.len(),
        });
    }
    let expected_samples = frames
        .checked_mul(channels)
        .ok_or(PcmFormatError::OutputLength {
            expected: usize::MAX,
            actual: output.len(),
        })?;
    let expected_bytes = expected_samples
        .checked_mul(format.bytes_per_sample())
        .ok_or(PcmFormatError::OutputLength {
            expected: usize::MAX,
            actual: output.len(),
        })?;
    if output.len() != expected_bytes {
        return Err(PcmFormatError::OutputLength {
            expected: expected_bytes,
            actual: output.len(),
        });
    }
    for frame in 0..frames {
        for channel in 0..channels {
            let sample = input[frame * 2 + channel];
            let offset = (frame * channels + channel) * format.bytes_per_sample();
            match format {
                SampleFormat::Pcm16 => {
                    let scaled = if sample <= -1.0 {
                        i16::MIN
                    } else {
                        (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16
                    };
                    output[offset..offset + 2].copy_from_slice(&scaled.to_le_bytes());
                }
                SampleFormat::Float32 => {
                    output[offset..offset + 4].copy_from_slice(&sample.to_le_bytes());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_pcm16_mono_uses_slot_zero_and_clears_slot_one() {
        let input = [0x00, 0x40, 0x00, 0xc0];
        let mut output = [f32::NAN; 4];

        decode_interleaved(&input, &mut output, SampleFormat::Pcm16, Channels::Mono).unwrap();

        assert_eq!(output[0], 0.5);
        assert_eq!(output[1], 0.0);
        assert_eq!(output[2], -0.5);
        assert_eq!(output[3], 0.0);
    }

    #[test]
    fn decode_float32_stereo_preserves_interleaved_channels() {
        let input = [
            1.0f32.to_le_bytes(),
            (-2.0f32).to_le_bytes(),
            0.25f32.to_le_bytes(),
            0.5f32.to_le_bytes(),
        ]
        .concat();
        let mut output = [0.0; 4];

        decode_interleaved(&input, &mut output, SampleFormat::Float32, Channels::Stereo).unwrap();

        assert_eq!(output, [1.0, -2.0, 0.25, 0.5]);
    }

    #[test]
    fn encode_pcm16_mono_ignores_slot_one_and_clamps_samples() {
        let input = [-2.0, 8.0, 0.5, -0.5];
        let mut output = [0; 4];

        encode_interleaved(&input, &mut output, SampleFormat::Pcm16, Channels::Mono).unwrap();

        assert_eq!(output, [0x00, 0x80, 0x00, 0x40]);
    }

    #[test]
    fn conversion_rejects_wrong_lengths_before_touching_output() {
        let mut output = [127.0_f32; 4];
        assert_eq!(
            decode_interleaved(&[0; 2], &mut output, SampleFormat::Pcm16, Channels::Stereo),
            Err(PcmFormatError::InputLength {
                expected: 8,
                actual: 2,
            })
        );
        assert_eq!(output, [127.0_f32; 4]);
        assert_eq!(
            encode_interleaved(
                &[0.0, 0.0, 0.0],
                &mut [0; 8],
                SampleFormat::Float32,
                Channels::Stereo,
            ),
            Err(PcmFormatError::InputLength {
                expected: 2,
                actual: 3,
            })
        );
    }

    #[test]
    fn decode_rejects_an_incomplete_engine_frame_before_writing_output() {
        let mut output = [127.0_f32; 3];

        assert_eq!(
            decode_interleaved(&[0; 2], &mut output, SampleFormat::Pcm16, Channels::Mono),
            Err(PcmFormatError::OutputLength {
                expected: 4,
                actual: 3,
            })
        );
        assert_eq!(output, [127.0_f32; 3]);
    }
}
