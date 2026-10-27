//! Read-only evidence for finalized capture artifacts, independent of UI state.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Encoding {
    Float32,
    Pcm24,
    Pcm16,
}

impl Encoding {
    fn bits(self) -> u16 {
        match self {
            Self::Float32 => 32,
            Self::Pcm24 => 24,
            Self::Pcm16 => 16,
        }
    }
}

#[derive(serde::Serialize)]
pub(super) struct WavEvidence {
    pub bytes: u64,
    pub data_offset: usize,
    pub data_bytes: usize,
    pub format_tag: u16,
    pub fmt_chunk_bytes: usize,
    pub bits_per_sample: u16,
    pub valid_bits_per_sample: u16,
    pub channel_mask: Option<u32>,
    pub pcm_subtype_verified: bool,
    pub block_align: u16,
    pub byte_rate: u32,
    pub frames: u64,
    pub channels: usize,
    pub sample_rate: u32,
    pub samples: u64,
    pub nonzero_samples: u64,
    pub sample_min: f64,
    pub sample_max: f64,
    pub all_samples_finite: bool,
}

fn u16_at(bytes: &[u8], offset: usize) -> anyhow::Result<u16> {
    let field = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| anyhow::anyhow!("truncated WAV u16 field"))?;
    Ok(u16::from_le_bytes(field.try_into()?))
}

fn u32_at(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    let field = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("truncated WAV u32 field"))?;
    Ok(u32::from_le_bytes(field.try_into()?))
}

/// Walk chunk sizes and RIFF word pads rather than assuming PCM data starts at
/// byte 44. The intended PCM contract is WAVEFORMATEXTENSIBLE with a PCM GUID.
pub(super) fn verify_bytes(
    bytes: &[u8],
    encoding: Encoding,
    frames: u64,
    rate: u32,
    channels: usize,
) -> anyhow::Result<WavEvidence> {
    anyhow::ensure!(
        frames > 0 && rate > 0 && (1..=256).contains(&channels),
        "missing confirmed WAV frame/rate/channel observations"
    );
    anyhow::ensure!(
        bytes.get(..4) == Some(b"RIFF")
            && bytes.get(8..12) == Some(b"WAVE")
            && u64::from(u32_at(bytes, 4)?) + 8 == bytes.len() as u64,
        "WAV RIFF signature or finalized file length is invalid"
    );
    let mut cursor = 12usize;
    let mut fmt = None;
    let mut fact = None;
    let mut data = None;
    while cursor < bytes.len() {
        let header_end = cursor
            .checked_add(8)
            .ok_or_else(|| anyhow::anyhow!("WAV chunk overflow"))?;
        anyhow::ensure!(header_end <= bytes.len(), "truncated WAV chunk header");
        let length = usize::try_from(u32_at(bytes, cursor + 4)?)?;
        let end = header_end
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("WAV chunk overflow"))?;
        let padded_end = end
            .checked_add(length & 1)
            .ok_or_else(|| anyhow::anyhow!("WAV padding overflow"))?;
        anyhow::ensure!(
            padded_end <= bytes.len(),
            "WAV chunk/pad exceeds file length"
        );
        let payload = &bytes[header_end..end];
        match &bytes[cursor..cursor + 4] {
            b"fmt " => {
                anyhow::ensure!(fmt.replace(payload).is_none(), "duplicate WAV fmt chunk");
            }
            b"fact" => {
                anyhow::ensure!(
                    fact.replace(u32_at(payload, 0)?).is_none(),
                    "duplicate WAV fact chunk"
                );
            }
            b"data" => {
                anyhow::ensure!(
                    data.replace((header_end, payload)).is_none(),
                    "duplicate WAV data chunk"
                );
            }
            _ => {}
        }
        cursor = padded_end;
    }
    let fmt = fmt.ok_or_else(|| anyhow::anyhow!("missing WAV fmt chunk"))?;
    let (data_offset, payload) = data.ok_or_else(|| anyhow::anyhow!("missing WAV data chunk"))?;
    let tag = u16_at(fmt, 0)?;
    let bits = u16_at(fmt, 14)?;
    let sample_bytes = usize::from(encoding.bits() / 8);
    let frame_bytes = channels
        .checked_mul(sample_bytes)
        .ok_or_else(|| anyhow::anyhow!("WAV frame overflow"))?;
    let expected_bytes = frames
        .checked_mul(frame_bytes as u64)
        .ok_or_else(|| anyhow::anyhow!("WAV data overflow"))?;
    let byte_rate = u32_at(fmt, 8)?;
    let block_align = u16_at(fmt, 12)?;
    anyhow::ensure!(
        usize::from(u16_at(fmt, 2)?) == channels
            && u32_at(fmt, 4)? == rate
            && bits == encoding.bits()
            && usize::from(block_align) == frame_bytes
            && u64::from(byte_rate) == u64::from(rate) * frame_bytes as u64
            && payload.len() as u64 == expected_bytes,
        "WAV format, block alignment, rate or data frames disagree with the service"
    );
    let (valid_bits, channel_mask) = if encoding == Encoding::Float32 {
        anyhow::ensure!(
            tag == 3
                && fmt.len() == 18
                && u16_at(fmt, 16)? == 0
                && fact.map(u64::from) == Some(frames),
            "invalid legacy float32 fmt/fact chunks"
        );
        (32, None)
    } else {
        const PCM_GUID: [u8; 16] = [1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113];
        anyhow::ensure!(
            tag == 0xfffe
                && fmt.len() == 40
                && u16_at(fmt, 16)? == 22
                && u16_at(fmt, 18)? == bits
                && u32_at(fmt, 20)? == 0
                && fmt.get(24..40) == Some(PCM_GUID.as_slice())
                && fact.is_none_or(|value| u64::from(value) == frames),
            "invalid extensible PCM tag, valid bits, channel mask or subtype GUID"
        );
        (u16_at(fmt, 18)?, Some(u32_at(fmt, 20)?))
    };
    let mut evidence = WavEvidence {
        bytes: bytes.len() as u64,
        data_offset,
        data_bytes: payload.len(),
        format_tag: tag,
        fmt_chunk_bytes: fmt.len(),
        bits_per_sample: bits,
        valid_bits_per_sample: valid_bits,
        channel_mask,
        pcm_subtype_verified: encoding != Encoding::Float32,
        block_align,
        byte_rate,
        frames,
        channels,
        sample_rate: rate,
        samples: 0,
        nonzero_samples: 0,
        sample_min: f64::INFINITY,
        sample_max: f64::NEG_INFINITY,
        all_samples_finite: true,
    };
    let mut channel_nonzero = vec![false; channels];
    for (index, sample) in payload.chunks_exact(sample_bytes).enumerate() {
        let value = match encoding {
            Encoding::Float32 => f64::from(f32::from_le_bytes(sample.try_into()?)),
            Encoding::Pcm16 => f64::from(i16::from_le_bytes(sample.try_into()?)) / 32768.,
            Encoding::Pcm24 => {
                let signed = i32::from_le_bytes([
                    sample[0],
                    sample[1],
                    sample[2],
                    if sample[2] & 0x80 == 0 { 0 } else { 0xff },
                ]);
                f64::from(signed) / 8_388_608.
            }
        };
        anyhow::ensure!(
            value.is_finite(),
            "capture WAV contains a non-finite sample"
        );
        evidence.samples += 1;
        evidence.nonzero_samples += u64::from(value != 0.);
        channel_nonzero[index % channels] |= value != 0.;
        evidence.sample_min = evidence.sample_min.min(value);
        evidence.sample_max = evidence.sample_max.max(value);
    }
    anyhow::ensure!(
        evidence.samples == frames * channels as u64 && channel_nonzero.iter().all(|value| *value),
        "capture WAV is empty, silent on a channel, or incomplete"
    );
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::{Encoding, verify_bytes};

    fn chunk(bytes: &mut Vec<u8>, id: &[u8; 4], payload: &[u8]) {
        bytes.extend_from_slice(id);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        if !payload.len().is_multiple_of(2) {
            bytes.push(0);
        }
    }

    /// Hand-built parser fixtures are deliberately separate from the encoder:
    /// a writer/header bug must not validate itself through a shared helper.
    fn wav(encoding: Encoding, channels: u16, extra_chunk: bool) -> Vec<u8> {
        let bits = encoding.bits();
        let align = channels * (bits / 8);
        let mut fmt = Vec::new();
        fmt.extend_from_slice(
            &(if encoding == Encoding::Float32 {
                3u16
            } else {
                0xfffe
            })
            .to_le_bytes(),
        );
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&48_000u32.to_le_bytes());
        fmt.extend_from_slice(&(48_000 * u32::from(align)).to_le_bytes());
        fmt.extend_from_slice(&align.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        if encoding == Encoding::Float32 {
            fmt.extend_from_slice(&0u16.to_le_bytes());
        } else {
            fmt.extend_from_slice(&22u16.to_le_bytes());
            fmt.extend_from_slice(&bits.to_le_bytes());
            fmt.extend_from_slice(&0u32.to_le_bytes());
            fmt.extend_from_slice(&[1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
        }
        let sample: Vec<u8> = match encoding {
            Encoding::Float32 => 0.25f32.to_le_bytes().to_vec(),
            Encoding::Pcm24 => vec![0, 0, 32],
            Encoding::Pcm16 => vec![0, 32],
        };
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        chunk(&mut bytes, b"fmt ", &fmt);
        if extra_chunk {
            chunk(&mut bytes, b"JUNK", &[7, 8, 9]);
        }
        if encoding == Encoding::Float32 {
            chunk(&mut bytes, b"fact", &3u32.to_le_bytes());
        }
        chunk(
            &mut bytes,
            b"data",
            &sample.repeat(3 * usize::from(channels)),
        );
        let length = bytes.len() as u32 - 8;
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        bytes
    }

    #[::core::prelude::v1::test]
    fn parses_float_and_extensible_pcm_with_variable_offsets_and_word_padding() {
        for encoding in [Encoding::Float32, Encoding::Pcm24, Encoding::Pcm16] {
            for channels in [1, 2, 8] {
                for extra in [false, true] {
                    let bytes = wav(encoding, channels, extra);
                    let proof =
                        verify_bytes(&bytes, encoding, 3, 48_000, usize::from(channels)).unwrap();
                    assert_eq!(proof.samples, 3 * u64::from(channels));
                    assert_eq!(proof.nonzero_samples, proof.samples);
                    assert_eq!(proof.sample_min, 0.25);
                    assert_eq!(proof.sample_max, 0.25);
                    assert_eq!(
                        proof.data_offset,
                        if encoding == Encoding::Float32 {
                            58
                        } else {
                            68
                        } + if extra { 12 } else { 0 }
                    );
                    assert_eq!(proof.bits_per_sample, encoding.bits());
                }
            }
        }
    }

    #[::core::prelude::v1::test]
    fn rejects_wrong_format_frames_rate_lengths_extensible_metadata_and_nonfinite_audio() {
        let pcm = wav(Encoding::Pcm24, 2, false);
        for (encoding, frames, rate, channels) in [
            (Encoding::Pcm16, 3, 48_000, 2),
            (Encoding::Pcm24, 4, 48_000, 2),
            (Encoding::Pcm24, 3, 44_100, 2),
            (Encoding::Pcm24, 3, 48_000, 1),
        ] {
            assert!(verify_bytes(&pcm, encoding, frames, rate, channels).is_err());
        }
        for offset in [4, 28, 32, 34, 36, 38, 40, 44, 64] {
            let mut corrupt = pcm.clone();
            corrupt[offset] ^= 1;
            assert!(
                verify_bytes(&corrupt, Encoding::Pcm24, 3, 48_000, 2).is_err(),
                "offset {offset}"
            );
        }
        assert!(verify_bytes(&pcm[..pcm.len() - 1], Encoding::Pcm24, 3, 48_000, 2).is_err());
        let mut float = wav(Encoding::Float32, 2, false);
        float[58..62].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(verify_bytes(&float, Encoding::Float32, 3, 48_000, 2).is_err());
    }

    #[::core::prelude::v1::test]
    fn decodes_signed_pcm_and_rejects_silent_or_duplicate_payloads() {
        let mut pcm = wav(Encoding::Pcm24, 1, false);
        pcm[68..71].copy_from_slice(&[0, 0, 192]); // -0.5, signed 24-bit.
        assert_eq!(
            verify_bytes(&pcm, Encoding::Pcm24, 3, 48_000, 1)
                .unwrap()
                .sample_min,
            -0.5
        );
        pcm[68..77].fill(0);
        assert!(verify_bytes(&pcm, Encoding::Pcm24, 3, 48_000, 1).is_err());
        let mut duplicate = wav(Encoding::Pcm16, 2, false);
        chunk(&mut duplicate, b"data", &[0, 32, 0, 32]);
        let length = duplicate.len() as u32 - 8;
        duplicate[4..8].copy_from_slice(&length.to_le_bytes());
        assert!(verify_bytes(&duplicate, Encoding::Pcm16, 3, 48_000, 2).is_err());
    }
}
