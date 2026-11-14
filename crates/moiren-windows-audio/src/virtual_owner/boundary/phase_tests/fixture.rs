//! Actual callback checkpoints versus a separately fed production FIR oracle.
//! All collection/allocation is test-only, outside any realtime owner.
use super::*;
use crate::resample::SrcCursor;
use std::{fs::OpenOptions, io::Write, path::PathBuf};

const CALLBACKS: usize = 512;
const RATES: [u32; 5] = [8000, 44_100, 48_000, 96_000, 384_000];

fn sample(index: usize) -> [f32; 2] {
    let bits = (index as u32)
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    let value = ((bits >> 8) as f32 / 16_777_216.0 - 0.5) * 0.5;
    [value, -value]
}

fn oracle(
    src: &mut StereoResampler,
    input: &[[f32; 2]],
    count: usize,
) -> Vec<(SrcCursor, [f64; 2])> {
    for frame in input {
        assert!(src.push_frame(frame.map(f64::from)));
    }
    (0..count)
        .map(|_| {
            let cursor = src.cursor();
            let frame = src.next_frame().expect("known complete FIR output");
            (cursor, frame)
        })
        .collect()
}

fn record(
    output: &mut impl Write,
    node: &str,
    kind: &str,
    point: BoundarySrcCheckpoint,
    oracle: &[(SrcCursor, [f64; 2])],
) {
    let centers: Vec<_> = oracle
        .iter()
        .enumerate()
        .map(|(offset, (cursor, _))| {
            serde_json::json!({"offset":offset,"input_frame":cursor.next_input_frame,"fraction":cursor.fraction})
        })
        .collect();
    serde_json::to_writer(
        &mut *output,
        &serde_json::json!({"record":"point","node":node,"kind":kind,"point":point,"oracle_centers":centers}),
    )
    .unwrap();
    output.write_all(b"\n").unwrap();
}

fn run_rate(output: &mut impl Write, rate: u32) {
    const BLOCK: usize = 32;
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let (writer, mut consumer) = audio_bridge(2, 1024, BUDGET).unwrap();
    let playback = Arc::new(BoundaryProbe::default());
    let microphone = Arc::new(BoundaryProbe::default());
    let mut source = VirtualPlaybackSource::new(reader, 2, BLOCK, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(playback.clone());
    let mut sink = VirtualMicrophoneSink::new(writer, 2, BLOCK, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(microphone.clone());
    let mut playback_oracle = StereoResampler::new(48_000, rate, 4096).unwrap();
    let mut microphone_oracle = StereoResampler::new(rate, 48_000, 4096).unwrap();
    let mut buffer = CallbackBuffer::<f64>::new(BLOCK, rate);
    let (mut submitted, mut consumed) = (0_usize, 0_usize);
    for index in 0..CALLBACKS {
        buffer.context.timeline_start = index as u64 * BLOCK as u64;
        let target: usize = if index < 192 { 768 } else { 384 };
        let fill = target.saturating_sub(source.carrier.available_frames());
        let input: Vec<_> = (submitted..submitted + fill).flat_map(sample).collect();
        producer.write_interleaved_exact(&input).unwrap();
        submitted += fill;
        assert_eq!(buffer.read_source(&mut source).xruns, 0);
        let point = playback.src_checkpoint().unwrap();
        let pushed =
            (point.after.unwrap().input_frames - point.before.unwrap().input_frames) as usize;
        let frames: Vec<_> = (consumed..consumed + pushed).map(sample).collect();
        consumed += pushed;
        let step = point.processing_step.unwrap();
        // This is a separate FIR instance; only applied correction is shared.
        // The external rational reader independently checks phase accumulation.
        playback_oracle.set_correction_ppm(source.drift.as_ref().unwrap().servo.correction());
        assert_eq!(playback_oracle.cursor().step, step);
        let expected = oracle(&mut playback_oracle, &frames, BLOCK);
        buffer
            .arena
            .with_io(&buffer.read, 0..BLOCK, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    unreachable!()
                };
                let block = inputs.get(0).unwrap();
                for (frame, (_, expected)) in expected.iter().enumerate() {
                    for (channel, sample) in expected.iter().enumerate() {
                        let value = if frame < point.muted_frames as usize {
                            0.0
                        } else {
                            *sample
                        };
                        assert_eq!(block.channel(channel)[frame], value);
                    }
                }
            })
            .unwrap();
        assert_eq!(playback_oracle.cursor(), point.after.unwrap());
        record(
            output,
            &format!("playback-{rate}"),
            "virtual_playback",
            point,
            &expected,
        );

        // Independent known native occupancy, not a native packet clock. Read
        // all after each callback; the prefix is a deliberate test-only seed.
        let queued = if index < 192 { 768 } else { 128 };
        sink.carrier.write_interleaved_exact(&pcm(queued)).unwrap();
        let input: Vec<_> = (index * BLOCK..(index + 1) * BLOCK).map(sample).collect();
        buffer
            .arena
            .with_io(&buffer.write, 0..BLOCK, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    unreachable!()
                };
                let mut block = outputs.get_mut(0).unwrap();
                for (frame, values) in input.iter().enumerate() {
                    for (channel, value) in values.iter().enumerate() {
                        block.channel_mut(channel)[frame] = f64::from(*value);
                    }
                }
            })
            .unwrap();
        assert_eq!(buffer.write_sink(&mut sink).xruns, 0);
        let point = microphone.src_checkpoint().unwrap();
        let produced =
            (point.after.unwrap().output_frames - point.before.unwrap().output_frames) as usize;
        microphone_oracle.set_correction_ppm(sink.drift.as_ref().unwrap().servo.correction());
        assert_eq!(
            microphone_oracle.cursor().step,
            point.processing_step.unwrap()
        );
        let expected = oracle(&mut microphone_oracle, &input, produced);
        let mut actual = vec![0.0; (queued + produced) * 2];
        assert_eq!(
            consumer
                .read_interleaved(&mut actual)
                .unwrap()
                .transferred_frames,
            queued + produced
        );
        for (actual, (_, expected)) in actual[queued * 2..]
            .as_chunks::<2>()
            .0
            .iter()
            .zip(&expected)
        {
            assert_eq!(*actual, expected.map(|value| value as f32));
        }
        assert_eq!(microphone_oracle.cursor(), point.after.unwrap());
        record(
            output,
            &format!("microphone-{rate}"),
            "virtual_microphone",
            point,
            &expected,
        );
    }
}

fn run_barriers(output: &mut impl Write) {
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let (writer, mut consumer) = audio_bridge(2, 1024, BUDGET).unwrap();
    let playback = Arc::new(BoundaryProbe::default());
    let microphone = Arc::new(BoundaryProbe::default());
    let playback_epoch = Arc::new(AtomicU64::new(0));
    let microphone_epoch = Arc::new(AtomicU64::new(0));
    let mut source = VirtualPlaybackSource::new(reader, 2, 32, 48_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_reset_epoch(playback_epoch.clone())
        .with_probe(playback.clone());
    let mut sink = VirtualMicrophoneSink::new(writer, 2, 32, 48_000, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_reset_epoch(microphone_epoch.clone())
        .with_probe(microphone.clone());
    let mut buffer = CallbackBuffer::<f64>::new(32, 48_000);
    for sequence in 1..=18 {
        buffer.context.timeline_start = (sequence - 1) * 32;
        if sequence == 6 {
            playback_epoch.store(1, Ordering::Release);
        } else if sequence == 11 {
            source.carrier.discard();
        } else if ![1, 12].contains(&sequence) {
            let fill = 768 - source.carrier.available_frames();
            producer.write_interleaved_exact(&pcm(fill)).unwrap();
        }
        let report = buffer.read_source(&mut source);
        let point = playback.src_checkpoint().unwrap();
        assert_eq!(point.sequence, sequence);
        if sequence == 11 {
            assert!(report.xruns > 0);
            assert_eq!(point.carrier_frames, 0);
        }
        if [2, 7, 13].contains(&sequence) {
            assert_eq!(point.muted_frames, 32);
        }
        record(output, "playback-barriers", "virtual_playback", point, &[]);

        consumer.discard();
        if sequence == 2 {
            sink.carrier.write_interleaved_exact(&pcm(1024)).unwrap();
        }
        if sequence == 5 {
            microphone_epoch.store(1, Ordering::Release);
        }
        if sequence == 8 {
            sink.carrier.write_interleaved_exact(&pcm(1023)).unwrap();
        }
        buffer.fill([0.125, -0.25]);
        let report = buffer.write_sink(&mut sink);
        let point = microphone.src_checkpoint().unwrap();
        assert_eq!(point.sequence, sequence);
        if [2, 8].contains(&sequence) {
            assert!(report.xruns > 0);
            assert_ne!(
                point.before.unwrap().reset_generation,
                point.after.unwrap().reset_generation
            );
        }
        record(
            output,
            "microphone-barriers",
            "virtual_microphone",
            point,
            &[],
        );
    }
}

#[test]
fn src_phase_actual_callbacks_match_separately_fed_fir_oracles() {
    // A finite discard sink keeps this regression portable and hardware-free.
    for rate in RATES {
        run_rate(&mut std::io::sink(), rate);
    }
    run_barriers(&mut std::io::sink());
}

#[test]
#[ignore = "writes host-only scalar/center fixture to MOIREN_SRC_PHASE_FIXTURE; no native devices"]
fn src_phase_export_actual_callback_fixture() {
    let directory =
        PathBuf::from(std::env::var_os("MOIREN_SRC_PHASE_FIXTURE").expect("explicit fixture path"));
    std::fs::create_dir(&directory).unwrap();
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("actual-boundary-centers.jsonl"))
        .unwrap();
    serde_json::to_writer(&mut output, &serde_json::json!({
        "record":"origin", "native_execution":false, "qualified":false,
        "scope":"actual virtual boundary calls, controlled synthetic carrier occupancy; separately fed production FIR per-output cursors and PCM; no native devices or source/recorder clock",
        "rates":RATES, "callbacks_per_boundary":CALLBACKS,
    })).unwrap();
    output.write_all(b"\n").unwrap();
    for rate in RATES {
        run_rate(&mut output, rate);
    }
    run_barriers(&mut output);
    output.flush().unwrap();
}
