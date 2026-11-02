//! Offline master-render projection using the unchanged production FIR/codec.
//! This constructs no native stream and cannot certify its submitted output.
#[allow(dead_code)]
#[path = "../src/codec.rs"]
mod codec;
mod native {
    pub type StereoFrame = [f64; 2];
}
#[allow(dead_code)]
#[path = "../src/resample.rs"]
mod resample;

#[cfg(any(windows, test))]
const MAX_FRAMES: usize = 2_880_000;

#[cfg(any(windows, test))]
struct Projection {
    samples: Vec<[f32; 2]>,
    valid: Vec<u8>,
    generation: Vec<u64>,
}

#[cfg(any(windows, test))]
fn read_capped(reader: impl std::io::Read, limit: usize) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= limit, "input grew beyond byte cap");
    Ok(bytes)
}

#[cfg(any(windows, test))]
fn project(input: &[[f32; 2]], valid: &[u8], generation: &[u64]) -> anyhow::Result<Projection> {
    use anyhow::ensure;
    ensure!(
        (17..=MAX_FRAMES).contains(&input.len()),
        "input frame cap17..2880000"
    );
    ensure!(
        valid.len() == input.len() && generation.len() == input.len(),
        "input mapping lengths differ"
    );
    ensure!(
        input
            .iter()
            .flatten()
            .all(|x| x.is_finite() && x.abs() < 1.0),
        "nonfinite/clipped input"
    );
    ensure!(valid.iter().all(|&v| v <= 1), "validity must be binary");
    ensure!(
        valid.iter().zip(generation).all(|(&v, &g)| v == 0 || g > 0),
        "valid frame lacks generation"
    );
    let mut src = resample::StereoResampler::new(48_000, 48_000, 128)?;
    let codec = codec::SampleCodec::with_layout(
        moiren_engine::audio_config::SampleFormat::Float32,
        2,
        32,
        32,
    )?;
    let count = input.len() - 16;
    let mut output = Projection {
        samples: Vec::with_capacity(count),
        valid: Vec::with_capacity(count),
        generation: Vec::with_capacity(count),
    };
    for frame in input {
        ensure!(
            src.push_frame(frame.map(f64::from)),
            "FIR storage unexpectedly full"
        );
        while let Some(sample) = src.next_frame() {
            let mut bytes = [0u8; 8];
            codec.encode(&[sample], &mut bytes)?;
            output.samples.push([
                f32::from_le_bytes(bytes[..4].try_into()?),
                f32::from_le_bytes(bytes[4..].try_into()?),
            ]);
            let center = output.samples.len() - 1;
            // No native write witness exists for this projection. Conservatively
            // refuse initial history and every UNKNOWN/generation dependency.
            let known = center >= 15
                && (center - 15..=center + 16)
                    .all(|i| valid[i] == 1 && generation[i] == generation[center]);
            output.valid.push(u8::from(known));
            output
                .generation
                .push(if known { generation[center] } else { 0 });
        }
    }
    ensure!(
        output.samples.len() == count,
        "unexpected same-rate FIR output frontier"
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_read_refuses_growth_and_accepts_exact_limit() {
        assert_eq!(read_capped(&[1u8; 8][..], 8).unwrap(), [1; 8]);
        assert!(read_capped(&[1u8; 9][..], 8).is_err());
        assert!(read_capped(std::io::repeat(1), 8).is_err());
    }

    #[test]
    fn same_rate_projection_has_real_fir_and_no_synthetic_tail() {
        let input: Vec<_> = (0..96)
            .map(|n| [if n == 40 { 0.25 } else { 0.0 }; 2])
            .collect();
        let output = project(&input, &[1; 96], &[7; 96]).unwrap();
        assert_eq!(output.samples.len(), 80);
        assert_ne!(output.samples[40], input[40]);
        assert!(
            output.samples[25..57]
                .iter()
                .filter(|x| x[0] != 0.0)
                .count()
                > 20
        );
        assert!(output.valid[..15].iter().all(|&v| v == 0));
        assert!(output.valid[15..].iter().all(|&v| v == 1));
    }

    #[test]
    fn unknown_support_expands_to_every_fir_dependency() {
        let mut valid = [1; 96];
        valid[40] = 0;
        let mut generation = [7; 96];
        generation[40] = 0;
        let output = project(&[[0.125; 2]; 96], &valid, &generation).unwrap();
        assert!(output.valid[24..56].iter().all(|&v| v == 0));
        assert_eq!(output.valid[23], 1);
        assert_eq!(output.valid[56], 1);
    }

    #[test]
    fn generation_crossing_is_unknown_without_resetting_master_fir() {
        let mut generation = [7; 96];
        generation[40..].fill(8);
        let output = project(&[[0.125; 2]; 96], &[1; 96], &generation).unwrap();
        assert!(output.valid[24..55].iter().all(|&v| v == 0));
        assert_eq!(output.generation[23], 7);
        assert_eq!(output.generation[55], 8);
        assert_eq!(output.samples[50], [0.125; 2]);
    }

    #[test]
    fn malformed_input_is_refused_without_normalizing_nonfinite_pcm() {
        assert!(project(&[[0.0; 2]; 32], &[1; 31], &[7; 32]).is_err());
        assert!(project(&[[f32::NAN; 2]; 32], &[1; 32], &[7; 32]).is_err());
        assert!(project(&[[0.0; 2]; 32], &[2; 32], &[7; 32]).is_err());
        assert!(project(&[[0.0; 2]; 32], &[1; 32], &[0; 32]).is_err());
    }
}

#[cfg(any(windows, test))]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        path::PathBuf,
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 4,
        "usage: process_render_replay INPUT.f32le VALID.u8 GENERATION.u64le NEW_OUTPUT_DIRECTORY"
    );
    let paths: Vec<_> = args.iter().map(PathBuf::from).collect();
    let sizes = [MAX_FRAMES * 8, MAX_FRAMES, MAX_FRAMES * 8];
    for (path, limit) in paths[..3].iter().zip(sizes) {
        let metadata = fs::metadata(path)?;
        ensure!(
            metadata.is_file() && metadata.len() <= limit as u64,
            "input byte cap"
        );
    }
    let pcm = read_capped(fs::File::open(&paths[0])?, sizes[0])?;
    let valid = read_capped(fs::File::open(&paths[1])?, sizes[1])?;
    let generations = read_capped(fs::File::open(&paths[2])?, sizes[2])?;
    ensure!(
        pcm.len().is_multiple_of(8) && generations.len().is_multiple_of(8),
        "partial input frame"
    );
    let input: Vec<_> = pcm
        .as_chunks::<8>()
        .0
        .iter()
        .map(|x| {
            [
                f32::from_le_bytes(x[..4].try_into().unwrap()),
                f32::from_le_bytes(x[4..].try_into().unwrap()),
            ]
        })
        .collect();
    let generations: Vec<_> = generations
        .as_chunks::<8>()
        .0
        .iter()
        .map(|x| u64::from_le_bytes(*x))
        .collect();
    let output = project(&input, &valid, &generations)?;
    ensure!(!paths[3].exists(), "output exists; never overwrite");
    let parent = paths[3]
        .parent()
        .context("output parent required")?
        .canonicalize()?;
    let destination = parent.join(paths[3].file_name().context("output name required")?);
    for path in &paths[..3] {
        ensure!(
            !path.canonicalize()?.starts_with(&destination),
            "output overlaps input"
        );
    }
    fs::create_dir(&destination)?;
    let write_new = |name: &str, bytes: &[u8]| -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    };
    let samples: Vec<_> = output
        .samples
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let generation_bytes: Vec<_> = output
        .generation
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    write_new("render.f32le", &samples)?;
    write_new("render-valid.u8", &output.valid)?;
    write_new("render-generation.u64le", &generation_bytes)?;
    let report = serde_json::json!({"schema_version":1,"qualified":false,"scope":"Production same-rate master FIR plus Float32 codec projection, no native submission witness","input_frames":input.len(),"output_frames":output.samples.len(),"known_frames":output.valid.iter().filter(|&&v|v==1).count(),"kernel":"unchanged production StereoResampler phase0 step1 32tap cutoff0.94","support":"center-15 through center+16; initial/UNKNOWN/generation crossings unqualified","native_submitted":false,"physical_latency_ms":null});
    write_new("summary.json", &serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}

#[cfg(not(any(windows, test)))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("native Float32 codec replay currently requires a Windows build")
}
