//! Looping WAV material for the music soak: exact 48 kHz stereo, PCM16 or
//! Float32. Loading happens once on the control side before any native client
//! starts, so a rejected file fails before audio begins.

use anyhow::{Context, bail, ensure};
use std::path::Path;

pub(super) const MATERIAL_SAMPLE_RATE: u32 = 48_000;
/// One correlation window plus the initial pipeline priming must exist.
const MIN_MATERIAL_FRAMES: usize = 48_000;
/// The alignment search decimates by this factor; padding the loop tail to a
/// multiple keeps decimated indices wrap exactly like full-rate indices.
pub(super) const MATERIAL_ALIGN_FRAMES: usize = 64;
const MAX_MATERIAL_BYTES: usize = 1 << 31;

pub(super) struct Material {
    /// Interleaved stereo Float32 samples; the render source loops forever.
    pub(super) samples: Vec<f32>,
    pub(super) frames: usize,
}

// Only the Windows-gated target coordinator loads a soak file; this module
// also compiles on other targets for its parser tests.
#[cfg(windows)]
pub(super) fn load(path: &Path) -> anyhow::Result<Material> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> anyhow::Result<Material> {
    ensure!(
        bytes.len() <= MAX_MATERIAL_BYTES,
        "material exceeds the bounded soak budget"
    );
    let header = bytes.get(..12).context("truncated RIFF header")?;
    ensure!(
        &header[0..4] == b"RIFF" && &header[8..12] == b"WAVE",
        "material is not a RIFF/WAVE file"
    );
    let mut position = 12;
    let mut format = None;
    let mut data = None;
    while let Some(chunk) = bytes.get(position..) {
        if chunk.len() < 8 {
            break;
        }
        let identifier = &chunk[0..4];
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
        let body = position + 8;
        let end = body
            .checked_add(size)
            .filter(|&end| end <= bytes.len())
            .with_context(|| format!("chunk {identifier:?} exceeds the file"))?;
        match identifier {
            b"fmt " => format = Some(parse_format(&bytes[body..end])?),
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        // RIFF chunks are word aligned.
        position = end + (size & 1);
    }
    let format = format.context("material has no fmt chunk")?;
    let data = data.context("material has no data chunk")?;
    let samples = decode(data, format)?;
    let mut material = Material {
        frames: samples.len() / 2,
        samples,
    };
    ensure!(
        material.frames >= MIN_MATERIAL_FRAMES,
        "material must hold at least one full second; found {} frames",
        material.frames
    );
    pad_to_alignment(&mut material);
    Ok(material)
}

#[derive(Clone, Copy)]
struct Format {
    encoding: u16,
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
}

fn parse_format(chunk: &[u8]) -> anyhow::Result<Format> {
    ensure!(chunk.len() >= 16, "truncated WAVEFORMATEX");
    let encoding = u16::from_le_bytes([chunk[0], chunk[1]]);
    let channels = u16::from_le_bytes([chunk[2], chunk[3]]);
    let sample_rate = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
    let bits_per_sample = u16::from_le_bytes([chunk[14], chunk[15]]);
    // WAVE_FORMAT_EXTENSIBLE keeps the real encoding in the extension.
    if encoding == 0xFFFE
        && let Some(subformat) = chunk
            .get(24..40)
            .map(|bytes| <[u8; 16]>::try_from(bytes).expect("fixed size"))
    {
        let base = u16::from_le_bytes([subformat[0], subformat[1]]);
        ensure!(
            subformat[2..] == [0; 14][..],
            "unsupported extensible subformat GUID"
        );
        return Ok(Format {
            encoding: base,
            channels,
            sample_rate,
            bits_per_sample,
        });
    }
    Ok(Format {
        encoding,
        channels,
        sample_rate,
        bits_per_sample,
    })
}

fn decode(data: &[u8], format: Format) -> anyhow::Result<Vec<f32>> {
    ensure!(
        format.channels == 2,
        "material must be stereo, found {} channels",
        format.channels
    );
    ensure!(
        format.sample_rate == MATERIAL_SAMPLE_RATE,
        "material must be exactly 48 kHz, found {} Hz",
        format.sample_rate
    );
    let bytes_per_sample = usize::from(format.bits_per_sample) / 8;
    ensure!(
        bytes_per_sample > 0 && data.len().is_multiple_of(bytes_per_sample * 2),
        "material data is not a whole number of stereo frames"
    );
    match (format.encoding, format.bits_per_sample) {
        (0x0001, 16) => Ok(data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes(*bytes) as f32 / 32_768.0)
            .collect()),
        (0x0003, 32) => Ok(data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect()),
        (encoding, bits) => {
            bail!("material must be PCM16 or Float32, found encoding {encoding:#x} at {bits} bits")
        }
    }
}

/// The material loops forever; extending it to a whole alignment block with
/// its own leading frames keeps every loop length identical.
fn pad_to_alignment(material: &mut Material) {
    let block = MATERIAL_ALIGN_FRAMES * 2;
    let remainder = material.samples.len() % block;
    if remainder == 0 {
        return;
    }
    let missing = block - remainder;
    let padding = material.samples[..missing].to_vec();
    material.samples.extend_from_slice(&padding);
    material.frames = material.samples.len() / 2;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(encoding: u16, bits: u16, rate: u32, channels: u16, samples: &[u8]) -> Vec<u8> {
        let block_align = channels * bits / 8;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&encoding.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        bytes.extend_from_slice(samples);
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes
    }

    fn one_second_pcm16(value: i16) -> Vec<u8> {
        // 48_000 stereo frames of 16-bit samples.
        value.to_le_bytes().to_vec().repeat(96_000)
    }

    #[test]
    fn loads_pcm16_and_float32_stereo_material_with_alignment_padding() {
        let pcm = parse(&wav(0x0001, 16, 48_000, 2, &one_second_pcm16(8_192))).unwrap();
        assert_eq!(pcm.frames, 48_000);
        assert!(
            pcm.samples
                .iter()
                .all(|&sample| (sample - 0.25).abs() < 1e-6)
        );

        // 48_001 frames: padded to the next whole 64-frame block.
        let mut samples = one_second_pcm16(16_384);
        samples.extend_from_slice(&[(-16_384i16).to_le_bytes(), 16_384i16.to_le_bytes()].concat());
        let padded = parse(&wav(0x0001, 16, 48_000, 2, &samples)).unwrap();
        assert_eq!(padded.frames, 48_064);
        assert_eq!(&padded.samples[96_000..96_002], &[-0.5, 0.5][..]);
        assert_eq!(&padded.samples[96_002..96_006], &[0.5; 4][..]);

        let floats = (0..48_000)
            .flat_map(|frame| [(frame as f32) * 1e-4, -(frame as f32) * 1e-4])
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let material = parse(&wav(0x0003, 32, 48_000, 2, &floats)).unwrap();
        assert_eq!(material.frames, 48_000);
        assert_eq!(material.samples[0], 0.0);
        assert_eq!(material.samples[2], 1e-4);
    }

    #[test]
    fn rejects_wrong_shapes_truncation_and_oversize() {
        for (label, bytes) in [
            ("empty", Vec::new()),
            ("not-riff", b"NOTRIFF____________WAVEjunk".to_vec()),
        ] {
            assert!(parse(&bytes).is_err(), "accepted {label}");
        }
        // Wrong rate/channel/encoding/length/short material all reject.
        assert!(parse(&wav(0x0001, 16, 44_100, 2, &one_second_pcm16(1))).is_err());
        assert!(parse(&wav(0x0001, 16, 48_000, 1, &one_second_pcm16(1))).is_err());
        assert!(parse(&wav(0x0001, 24, 48_000, 2, &one_second_pcm16(1))).is_err());
        assert!(parse(&wav(0x0001, 16, 48_000, 2, &[])).is_err());
        let short = 8_192i16.to_le_bytes().to_vec().repeat(100);
        assert!(parse(&wav(0x0001, 16, 48_000, 2, &short)).is_err());
        let truncated = {
            let mut bytes = wav(0x0001, 16, 48_000, 2, &one_second_pcm16(1));
            bytes.truncate(bytes.len() - 4);
            bytes
        };
        assert!(parse(&truncated).is_err());
    }
}
