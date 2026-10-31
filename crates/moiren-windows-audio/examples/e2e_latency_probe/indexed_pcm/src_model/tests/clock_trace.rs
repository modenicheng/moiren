use super::*;

const STARTS: [usize; 3] = [24_000, 72_000, 120_000];
const SOURCE_FRAMES: usize = 144_000;
const BLOCK: usize = 32;

struct Stage {
    samples: Vec<f32>,
    centers: Vec<f64>,
    corrections: Vec<f64>,
}

struct Trace {
    source: Vec<f32>,
    playback: Stage,
    microphone: Stage,
}

fn trace(book: &[Vec<f32>], varying: bool) -> Trace {
    let mut source = vec![0.0; SOURCE_FRAMES];
    for (start, code) in STARTS.iter().zip(book) {
        source[*start..*start + code.len()].copy_from_slice(code);
    }
    let playback = stage(&source, varying, true);
    let microphone = stage(&playback.samples, varying, false);
    Trace {
        source,
        playback,
        microphone,
    }
}

fn stage(input: &[f32], varying: bool, playback: bool) -> Stage {
    let mut src = StereoResampler::new(48_000, 48_000, 4096).unwrap();
    let mut servo = crate::servo::FollowerServo::new(1000);
    assert_eq!(servo.target(), 1000);
    assert_eq!(servo.correction(), 0.0);
    let mut result = Stage {
        samples: Vec::new(),
        centers: Vec::new(),
        corrections: Vec::new(),
    };
    let (mut supplied, mut center) = (0, 0.0);
    loop {
        let callback = result.corrections.len();
        let transition = if playback { 1500 } else { 3000 };
        let occupancy = if varying && callback < transition {
            2000
        } else {
            1000
        };
        let correction = servo.observe(occupancy);
        assert_eq!(correction, servo.correction());
        src.set_correction_ppm(correction);
        // Mirror the production call cadence: playback requests output blocks;
        // microphone pushes Engine blocks and drains every available output.
        let needed = if playback {
            src.input_frames_needed(BLOCK).unwrap()
        } else {
            BLOCK
        };
        if supplied + needed > input.len() {
            break;
        }
        result.corrections.push(correction);
        for sample in &input[supplied..supplied + needed] {
            assert!(src.push_frame([f64::from(*sample); 2]));
        }
        supplied += needed;
        let mut produced = 0;
        while !playback || produced < BLOCK {
            let Some(frame) = src.next_frame() else {
                break;
            };
            assert_eq!(frame[0], frame[1]);
            result.samples.push(frame[0] as f32);
            // Global source-center arithmetic is independent of the FIR's
            // local ring position/discard bookkeeping. It is not physical time.
            result.centers.push(center);
            center += 1.0 + correction * 1e-6;
            produced += 1;
        }
        if playback {
            assert_eq!(produced, BLOCK);
        }
    }
    result
}

fn scalar_fir(input: &[f32], centers: &[f64]) -> Vec<f32> {
    // Direct indexing, no streaming ring or production kernel helper. Recreate
    // the documented 32-tap/256-phase interpolation response, not a clock fit.
    let kernels: Vec<[f64; 32]> = (0..=256)
        .map(|phase| {
            let mut kernel = std::array::from_fn(|tap| {
                let distance = tap as f64 - 15.0 - f64::from(phase) / 256.0;
                let x = std::f64::consts::PI * distance * 0.94;
                let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
                let angle = std::f64::consts::PI * distance / 16.0;
                let window = (0.42 + 0.5 * angle.cos() + 0.08 * (2.0 * angle).cos()).max(0.0);
                0.94 * sinc * window
            });
            let sum = kernel.iter().sum::<f64>();
            for coefficient in &mut kernel {
                *coefficient /= sum;
            }
            kernel
        })
        .collect();
    centers
        .iter()
        .map(|center| {
            let whole = center.floor() as isize;
            let phase = center.fract() * 256.0;
            let index = phase.floor() as usize;
            let blend = phase.fract();
            (0..32)
                .map(|tap| {
                    let sample = usize::try_from(whole - 15 + tap as isize)
                        .ok()
                        .and_then(|i| input.get(i))
                        .copied()
                        .unwrap_or(0.0);
                    let a = kernels[index][tap];
                    let b = kernels[index + 1][tap];
                    f64::from(sample) * (a + blend * (b - a))
                })
                .sum::<f64>() as f32
        })
        .collect()
}

fn inverse(centers: &[f64], source: f64) -> f64 {
    let high = centers.partition_point(|center| *center < source);
    assert!(high > 0 && high < centers.len());
    let low = high - 1;
    low as f64 + (source - centers[low]) / (centers[high] - centers[low])
}

fn recorder_coordinate(trace: &Trace, source: usize) -> f64 {
    inverse(
        &trace.microphone.centers,
        inverse(&trace.playback.centers, source as f64),
    )
}

#[test]
fn two_stage_known_clock_samples_match_direct_index_oracle() {
    let book = codes_with_frames(3, 1024);
    for varying in [false, true] {
        let trace = trace(&book, varying);
        assert_eq!(trace.source.len(), SOURCE_FRAMES);
        for (input, stage) in [
            (&trace.source, &trace.playback),
            (&trace.playback.samples, &trace.microphone),
        ] {
            assert!(stage.samples.len() > 140_000);
            assert!(stage.corrections.len() >= stage.samples.len() / BLOCK);
            assert_eq!(stage.samples.len(), stage.centers.len());
            assert!(stage.centers.windows(2).all(|p| p[0] < p[1]));
            assert!(stage.corrections.iter().all(|v| v.abs() <= 2000.0));
            assert_eq!(stage.corrections.iter().any(|v| *v != 0.0), varying);
            let expected = scalar_fir(input, &stage.centers);
            assert_eq!(expected.len(), stage.samples.len());
            let error = expected
                .iter()
                .zip(&stage.samples)
                .map(|(a, b)| f64::from((a - b).abs()))
                .fold(0.0, f64::max);
            assert!(error <= 2e-7, "varying={varying}, error={error}");
            eprintln!(
                "scalar_oracle varying={varying} output_frames={} callbacks={} max_absolute_sample_error={error}",
                stage.samples.len(),
                stage.corrections.len()
            );
        }
        if varying {
            assert_ne!(
                trace.playback.corrections[2000],
                trace.microphone.corrections[2000]
            );
        }
    }
}

#[test]
fn known_varying_centers_reject_a_constant_affine_map_with_a_constant_control() {
    let book = codes_with_frames(3, 1024);
    for varying in [false, true] {
        let trace = trace(&book, varying);
        let coordinates = STARTS.map(|start| recorder_coordinate(&trace, start));
        let intervals = coordinates.map(|value| [value.floor(), value.ceil()]);
        let bounds = |i: usize| {
            let source_spacing = (STARTS[i + 1] - STARTS[i]) as f64;
            [
                (intervals[i + 1][0] - intervals[i][1]) / source_spacing,
                (intervals[i + 1][1] - intervals[i][0]) / source_spacing,
            ]
        };
        if varying {
            assert!(bounds(0)[1] < bounds(1)[0]);
        } else {
            assert_eq!(coordinates, STARTS.map(|start| start as f64));
            assert_eq!(bounds(0), [1.0, 1.0]);
            assert_eq!(bounds(1), [1.0, 1.0]);
        }
        eprintln!(
            "known_centers varying={varying} coordinates={coordinates:?} adjacent_slopes={:?}/{:?}",
            bounds(0),
            bounds(1)
        );
    }
}

#[test]
fn frozen_model_known_clock_comparison_never_substitutes_identity_for_mapping() {
    let book = codes_with_frames(3, 1024);
    for varying in [false, true] {
        let trace = trace(&book, varying);
        let coordinates = STARTS.map(|start| recorder_coordinate(&trace, start));
        let starts = coordinates.map(|value| value.floor() as usize - 64);
        let windows: Vec<_> = starts
            .iter()
            .map(|start| RecorderWindow {
                start_frame: *start as u64,
                samples: &trace.microphone.samples[*start..*start + 1160],
            })
            .collect();
        let report = analyze(&book, &windows, &ModelConfig::default()).unwrap();
        assert_eq!(report.results.len(), STARTS.len());
        assert_eq!(report.correlation_samples, 31_186_080);
        assert_eq!(report.source_origin, "unknown_not_estimated");
        assert_eq!(report.packet_quality, "not_evaluated_no_complete_set_claim");
        for (index, result) in report.results.iter().enumerate() {
            assert_eq!(result.status, Status::Unique);
            assert_eq!(result.source_index, Some(index));
            let [lo, hi] = result.recording_frame_interval.unwrap();
            assert!(hi - lo <= 2);
            assert!(coordinates[index] >= lo as f64 && coordinates[index] <= hi as f64);
        }
        eprintln!(
            "frozen_model varying={varying} known_centers={coordinates:?} report={}",
            serde_json::to_string(&report).unwrap()
        );
    }
}

#[test]
fn direct_oracle_detects_a_one_frame_center_error_on_the_fixed_waveform() {
    let book = codes_with_frames(3, 1024);
    let trace = trace(&book, true);
    for (input, stage) in [
        (&trace.source, &trace.playback),
        (&trace.playback.samples, &trace.microphone),
    ] {
        let shifted: Vec<_> = stage.centers.iter().map(|value| value + 1.0).collect();
        let wrong = scalar_fir(input, &shifted);
        let peak_error = wrong
            .iter()
            .zip(&stage.samples)
            .map(|(a, b)| f64::from((a - b).abs()))
            .fold(0.0, f64::max);
        assert!(peak_error > 0.005, "wrong-center oracle must not agree");
    }
}

#[test]
fn varying_clock_does_not_turn_erasure_or_sample_slips_into_a_complete_marker() {
    let book = codes_with_frames(3, 1024);
    for fault in 0..3 {
        let mut trace = trace(&book, true);
        let middle = STARTS[1];
        match fault {
            0 => trace.source[middle + 256..middle + 512].fill(0.0),
            1 => {
                trace.source.drain(middle + 512..middle + 528);
            }
            _ => {
                trace.source.splice(middle + 512..middle + 512, [0.0; 16]);
            }
        }
        trace.playback = stage(&trace.source, true, true);
        trace.microphone = stage(&trace.playback.samples, true, false);
        let coordinate = recorder_coordinate(&trace, middle);
        let start = coordinate.floor() as usize - 64;
        let report = analyze(
            &book,
            &[RecorderWindow {
                start_frame: start as u64,
                samples: &trace.microphone.samples[start..start + 1160],
            }],
            &ModelConfig::default(),
        )
        .unwrap();
        assert_ne!(report.results[0].status, Status::Unique, "fault={fault}");
        assert_eq!(report.results[0].source_index, None);
        assert_eq!(report.results[0].recording_frame_interval, None);
    }
}
