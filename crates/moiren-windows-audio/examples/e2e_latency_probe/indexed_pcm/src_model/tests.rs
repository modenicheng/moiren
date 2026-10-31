use super::*;
use crate::live::source::{INDEXED_WAVEFORM_VERSION, SourceCursor, SourceSchedule};
use crate::resample::StereoResampler;
use moiren_windows_audio::latency::{
    MarkerAnalysisConfig, MarkerAnalysisError, MarkerGenerator, analyze_marker,
};

const FRAMES: usize = 256;

fn codes(count: usize) -> Vec<Vec<f32>> {
    codes_with_frames(count, FRAMES)
}

fn codes_with_frames(count: usize, frames: usize) -> Vec<Vec<f32>> {
    let plan = schedule(count, frames).prepare(600).unwrap();
    (0..count)
        .map(|i| plan.indexed_marker(i).unwrap().to_vec())
        .collect()
}

fn schedule(count: usize, frames: usize) -> SourceSchedule {
    SourceSchedule {
        schema_version: 1,
        schedule_id: "synthetic-src-codebook".into(),
        waveform_version: INDEXED_WAVEFORM_VERSION.into(),
        sample_rate: 48_000,
        channels: 2,
        marker_frames: frames,
        marker_amplitude: 0.025,
        total_source_frames: (count * (frames + 100) + 125) as u64,
        starts: (0..count)
            .map(|i| (125 + i * (frames + 100)) as u64)
            .collect(),
    }
}

// Exercise the production FIR, not the model's template construction. Prior
// silence establishes fractional state without changing any source code bytes.
fn production(input: &[f32], ppm: f64, chunk: usize) -> Vec<f32> {
    let mut src = StereoResampler::new(48_000, 48_000, 4096).unwrap();
    assert!(StereoResampler::storage_bytes(4096).unwrap() > 4096);
    src.reset();
    assert_eq!(src.input_frames_needed(1), Some(17));
    src.set_correction_ppm(ppm);
    let mut out = Vec::new();
    for packet in input.chunks(chunk) {
        for sample in packet {
            assert!(src.push_frame([f64::from(*sample); 2]));
        }
        while let Some(frame) = src.next_frame() {
            out.push(frame[0] as f32);
        }
    }
    out
}

fn fixture(code: &[f32], prefix: usize, rates: &[f64], chunk: usize) -> Vec<f32> {
    let mut raw = vec![0.0; prefix];
    raw.extend_from_slice(code);
    raw.resize(raw.len() + 128, 0.0);
    for rate in rates {
        raw = production(&raw, *rate, chunk);
    }
    raw
}

fn run(codes: &[Vec<f32>], samples: &[f32]) -> ModelReport {
    analyze(
        codes,
        &[RecorderWindow {
            start_frame: 9000,
            samples,
        }],
        &ModelConfig::default(),
    )
    .unwrap()
}

#[test]
fn fractional_production_src_recovers_exact_nonzero_source_index() {
    let book = codes(4);
    let pcm = fixture(&book[2], 125, &[2000.0], 37);
    let report = run(&book, &pcm);
    assert_eq!(report.results[0].status, Status::Unique);
    assert_eq!(report.results[0].source_index, Some(2));
    let interval = report.results[0].recording_frame_interval.unwrap();
    assert!(interval[1] - interval[0] <= 2);
    assert!(interval[0] > 9000);
    assert_eq!(report.source_origin, "unknown_not_estimated");
    assert_eq!(report.packet_quality, "not_evaluated_no_complete_set_claim");
    assert!(report.correlation_samples <= WORK_CAP);
    assert_eq!(report.model, "affine_fir_synthetic_v1_not_exact_cascade");
    assert_eq!(
        (CORRELATION_FLOOR, PEAK_MARGIN, RMS_FLOOR),
        (0.7, 0.05, 0.0001)
    );
}

#[test]
fn inverted_whole_polarity_accepts_exact_index_and_same_recorder_coordinates() {
    let book = codes(4);
    for rates in [&[2000.0][..], &[2000.0, 2000.0][..]] {
        let positive = fixture(&book[2], 125, rates, 37);
        let inverted: Vec<_> = positive.iter().map(|sample| -*sample).collect();
        let expected = run(&book, &positive);
        let actual = run(&book, &inverted);
        assert_eq!(actual.results[0].status, Status::Unique);
        assert_eq!(actual.results[0].source_index, Some(2));
        assert_eq!(
            actual.results[0].recording_frame_interval,
            expected.results[0].recording_frame_interval
        );
        assert_eq!(actual.results[0].best_score, expected.results[0].best_score);
    }
}

#[test]
fn opposite_quarter_polarity_refuses_even_when_whole_score_exceeds_floor() {
    let book = codes(4);
    for quarter in 0..4 {
        let mut damaged = book[2].clone();
        // Keep this opposite-polarity section above the RMS floor, while the
        // other three quarters give a misleadingly high whole-window score.
        for sample in &mut damaged[quarter * FRAMES / 4..(quarter + 1) * FRAMES / 4] {
            *sample *= -0.2;
        }
        for rates in [&[2000.0][..], &[2000.0, 2000.0][..]] {
            let pcm = fixture(&damaged, 125, rates, 37);
            let report = run(&book, &pcm);
            let result = &report.results[0];
            assert!(
                result.best_score >= CORRELATION_FLOOR,
                "{quarter}/{rates:?}: {report:?}"
            );
            assert_eq!(
                result.status,
                Status::Unidentified,
                "{quarter}/{rates:?}: {report:?}"
            );
            assert_eq!(result.source_index, None);
            assert_eq!(result.recording_frame_interval, None);
        }
    }
}

#[test]
fn schema_and_cartesian_work_admission_refuse_before_matching() {
    let book = codes(4);
    let pcm = fixture(&book[0], 125, &[2000.0], 37);
    let windows = [RecorderWindow {
        start_frame: 0,
        samples: &pcm,
    }];
    let mut config = ModelConfig {
        schema_version: 2,
        ..ModelConfig::default()
    };
    assert_eq!(
        analyze(&book, &windows, &config).unwrap_err(),
        ModelError::InvalidConfig
    );
    config.schema_version = 1;
    config.max_correlation_samples = 1;
    assert_eq!(
        analyze(&book, &windows, &config).unwrap_err(),
        ModelError::WorkLimit
    );
}

#[test]
fn complete_work_budget_and_invalid_shapes_reject_before_any_sample_search() {
    let book = codes(4);
    let pcm = fixture(&book[2], 125, &[2000.0], 37);
    let admitted = run(&book, &pcm).correlation_samples;
    let windows = [RecorderWindow {
        start_frame: 0,
        samples: &pcm,
    }];
    let config = ModelConfig {
        max_correlation_samples: admitted - 1,
        ..ModelConfig::default()
    };
    assert_eq!(
        analyze(&book, &windows, &config).unwrap_err(),
        ModelError::WorkLimit
    );
    let config = ModelConfig {
        max_correlation_samples: admitted,
        ..ModelConfig::default()
    };
    assert_eq!(
        analyze(&book, &windows, &config)
            .unwrap()
            .correlation_samples,
        admitted
    );
    let many = vec![vec![f32::NAN; 1024]; 64];
    let observation = vec![f32::NAN; 8192];
    assert_eq!(
        analyze(
            &many,
            &[RecorderWindow {
                start_frame: 0,
                samples: &observation
            }],
            &ModelConfig::default()
        )
        .unwrap_err(),
        ModelError::WorkLimit
    );
    assert_eq!(
        analyze(
            &book,
            &[RecorderWindow {
                start_frame: u64::MAX,
                samples: &pcm
            }],
            &ModelConfig::default()
        )
        .unwrap_err(),
        ModelError::InvalidConfig
    );
    assert_eq!(
        analyze(
            &book,
            &[
                RecorderWindow {
                    start_frame: 100,
                    samples: &pcm
                },
                RecorderWindow {
                    start_frame: 101,
                    samples: &pcm
                },
            ],
            &ModelConfig::default()
        )
        .unwrap_err(),
        ModelError::InvalidConfig
    );
    let mut wrong = book.clone();
    wrong[0].pop();
    assert_eq!(
        analyze(&wrong, &windows, &ModelConfig::default()).unwrap_err(),
        ModelError::InvalidConfig
    );
}

#[test]
fn stretched_maximum_marker_budget_covers_every_scored_section_sample() {
    let book = codes_with_frames(1, 4096);
    let len = template_len(4096, 4000);
    let samples = vec![0.0; len + 2];
    let config = ModelConfig {
        phase_quarters: vec![0],
        scale_ppm: vec![4000],
        filter_passes: vec![1],
        ..ModelConfig::default()
    };
    let work = admission(
        &book,
        &[RecorderWindow {
            start_frame: 0,
            samples: &samples,
        }],
        &config,
    )
    .unwrap();
    let first = (GUARD as f64 * 1.004).ceil() as usize;
    let last = ((GUARD + 4096) as f64 * 1.004).ceil() as usize;
    assert!(work >= 3 * (len + last - first));
}

#[test]
fn wrong_code_and_low_energy_or_dc_do_not_manufacture_identity() {
    let book = codes(5);
    let wrong = fixture(&book[4], 125, &[2000.0], 37);
    assert_eq!(
        run(&book[..4], &wrong).results[0].status,
        Status::Unidentified
    );
    for pcm in [
        vec![0.1; wrong.len()],
        wrong.iter().map(|x| x * 0.0001).collect(),
    ] {
        assert_eq!(run(&book, &pcm).results[0].status, Status::Unidentified);
    }
}

#[test]
fn recorded_coordinate_translation_is_not_source_origin_estimation() {
    let book = codes(4);
    let pcm = fixture(&book[2], 125, &[2000.0], 37);
    let a = run(&book, &pcm);
    let b = analyze(
        &book,
        &[RecorderWindow {
            start_frame: 12_345,
            samples: &pcm,
        }],
        &ModelConfig::default(),
    )
    .unwrap();
    let a = a.results[0].recording_frame_interval.unwrap();
    let b = b.results[0].recording_frame_interval.unwrap();
    assert_eq!(b, a.map(|x| x + 3345));
}

#[test]
fn real_nominal_mismatch_single_and_unknown_cascade_are_synthetic_not_native_proof() {
    let book = codes_with_frames(4, 1024);
    for rates in [&[2000.0][..], &[2000.0, 2000.0][..]] {
        let pcm = fixture(&book[3], 125, rates, 37);
        let nominal = analyze_marker(
            &book[3],
            &pcm,
            MarkerAnalysisConfig {
                max_marker_frames: 1024,
                max_recording_frames: 8192,
                max_delay_frames: pcm.len() - 1024,
                max_correlation_samples: WORK_CAP,
                min_correlation: CORRELATION_FLOOR,
                min_peak_margin: PEAK_MARGIN,
                min_rms: RMS_FLOOR,
            },
        );
        assert!(
            matches!(
                nominal,
                Err(MarkerAnalysisError::NoMatch | MarkerAnalysisError::Ambiguous)
            ),
            "nominal unexpectedly matched {rates:?}: {nominal:?}"
        );
        let report = run(&book, &pcm);
        assert_eq!(
            report.results[0].status,
            Status::Unique,
            "{rates:?}: {report:?}"
        );
        assert_eq!(report.results[0].source_index, Some(3));
        println!(
            "synthetic rates={rates:?} nominal={nominal:?} model={:?} score={} work={} origin={}",
            report.results[0].status,
            report.results[0].best_score,
            report.correlation_samples,
            report.source_origin
        );
    }
}

#[test]
fn positive_negative_rates_prior_silence_phases_and_packet_chunking() {
    let book = codes(4);
    for (prefix, rates) in [
        (125, vec![-2000.0]),
        (375, vec![2000.0]),
        (250, vec![-2000.0, 2000.0]),
        (125, vec![-2000.0, -2000.0]),
    ] {
        let pcm = fixture(&book[1], prefix, &rates, 37);
        assert_eq!(pcm, fixture(&book[1], prefix, &rates, 1));
        let report = run(&book, &pcm);
        assert_eq!(
            report.results[0].status,
            Status::Unique,
            "{prefix}/{rates:?}: {report:?}"
        );
        assert_eq!(report.results[0].source_index, Some(1));
    }
}

#[test]
fn erased_quarter_truncation_and_sample_slips_are_not_independent_section_warps() {
    let book = codes_with_frames(3, 1024);
    let mut erased = book[1].clone();
    erased[256..512].fill(0.0);
    let mut inserted = book[1].clone();
    inserted.splice(512..512, [0.0; 16]);
    let mut deleted = book[1].clone();
    deleted.drain(512..528);
    for code in [erased, inserted, deleted] {
        let pcm = fixture(&code, 125, &[2000.0], 37);
        assert_ne!(run(&book, &pcm).results[0].status, Status::Unique);
    }
    let truncated = fixture(&book[1][..768], 125, &[2000.0], 37);
    assert_eq!(
        analyze(
            &book,
            &[RecorderWindow {
                start_frame: 0,
                samples: &truncated
            }],
            &ModelConfig::default()
        )
        .unwrap_err(),
        ModelError::InvalidConfig
    );
}

#[test]
fn bounded_servo_changes_and_off_grid_rates_do_not_fabricate_another_identity() {
    let book = codes_with_frames(4, 1024);
    let mut raw = vec![0.0; 125];
    raw.extend_from_slice(&book[2]);
    raw.resize(raw.len() + 128, 0.0);
    let mut src = StereoResampler::new(48_000, 48_000, 4096).unwrap();
    let (mut pcm, mut correction) = (Vec::new(), 0.0);
    // Same frozen 0.01 one-pole as FollowerServo; no occupancy/history inferred
    // from the native recording. Deliberately change the desired value mid-code.
    for (i, block) in raw.chunks(32).enumerate() {
        let desired = if i < 20 { 2000.0 } else { -2000.0 };
        correction += (desired - correction) * 0.01;
        src.set_correction_ppm(correction);
        for sample in block {
            assert!(src.push_frame([f64::from(*sample); 2]));
        }
        while let Some(frame) = src.next_frame() {
            pcm.push(frame[0] as f32);
        }
    }
    for pcm in [pcm, fixture(&book[2], 125, &[1000.0, -500.0], 37)] {
        let report = run(&book, &pcm);
        assert!(report.results[0].source_index.is_none_or(|i| i == 2));
    }
}

#[test]
fn near_equal_wrong_code_still_competes_and_no_threshold_filter_inflates_margin() {
    let mut book = codes(3);
    let original = book[0].clone();
    for (i, sample) in book[1].iter_mut().enumerate() {
        *sample = original[i] * 0.98 + *sample * 0.02;
    }
    let pcm = fixture(&book[0], 125, &[2000.0], 37);
    assert_eq!(run(&book, &pcm).results[0].status, Status::Ambiguous);
}

#[test]
fn strong_and_weaker_duplicate_across_windows_are_not_two_unique_identities() {
    let book = codes(4);
    let a = fixture(&book[2], 125, &[2000.0], 37);
    let b: Vec<_> = a.iter().map(|v| v * 0.2).collect();
    let report = analyze(
        &book,
        &[
            RecorderWindow {
                start_frame: 0,
                samples: &a,
            },
            RecorderWindow {
                start_frame: 48_000,
                samples: &b,
            },
        ],
        &ModelConfig::default(),
    )
    .unwrap();
    assert!(
        report
            .results
            .iter()
            .all(|r| r.status == Status::Ambiguous && r.source_index.is_none())
    );
}

#[test]
fn schema_rejects_unknown_fields_and_never_serializes_qualified_or_origin_fit() {
    let mut json = serde_json::to_value(ModelConfig::default()).unwrap();
    json["min_correlation"] = serde_json::json!(0.69);
    assert!(serde_json::from_value::<ModelConfig>(json).is_err());
    let book = codes(4);
    let pcm = fixture(&book[2], 125, &[2000.0], 37);
    let report = serde_json::to_value(run(&book, &pcm)).unwrap();
    assert!(report.get("qualified").is_none());
    assert!(report.get("identity_set_complete").is_none());
    assert_eq!(report["source_origin"], "unknown_not_estimated");
}

#[test]
fn codebook_bytes_are_exact_prepared_indexed_v2_not_reseeded_or_silence_advanced() {
    let source = schedule(4, FRAMES);
    let plan = source.clone().prepare(1).unwrap();
    let book = codes(4);
    let mut continuous = vec![0.0; 4 * FRAMES];
    MarkerGenerator::new(continuous.len(), 0.025)
        .unwrap()
        .fill(&mut continuous);
    for (index, code) in book.iter().enumerate() {
        assert_eq!(
            code.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            continuous[index * FRAMES..(index + 1) * FRAMES]
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(code, plan.indexed_marker(index).unwrap());
    }
    assert_ne!(book[0], book[1]);
    let mut cursor = SourceCursor::new(Some(plan));
    let mut rendered = Vec::new();
    for _ in 0..source.total_source_frames.div_ceil(37) {
        let mut stereo = [0.0; 74];
        cursor.submit(&mut stereo, false, |_| Ok(())).unwrap();
        rendered.extend(stereo.chunks_exact(2).map(|p| p[0]));
    }
    for (index, start) in source.starts.iter().enumerate() {
        assert_eq!(
            &rendered[*start as usize..*start as usize + FRAMES],
            &book[index]
        );
        assert!(
            rendered[*start as usize - 25..*start as usize]
                .iter()
                .all(|x| *x == 0.0)
        );
    }
}

#[test]
fn strong_and_weaker_complete_duplicates_and_different_codes_refuse() {
    let book = codes(4);
    for second in [&book[0], &book[2]] {
        let mut raw = vec![0.0; 125];
        raw.extend_from_slice(&book[0]);
        raw.resize(raw.len() + 160, 0.0);
        raw.extend(second.iter().map(|v| v * 0.2));
        raw.resize(raw.len() + 128, 0.0);
        let pcm = production(&raw, 2000.0, 37);
        assert_eq!(run(&book, &pcm).results[0].status, Status::Ambiguous);
    }
}

#[test]
fn missing_first_middle_and_reversed_windows_keep_original_code_indices() {
    let book = codes(5);
    let a = fixture(&book[4], 125, &[2000.0], 37);
    let b = fixture(&book[2], 125, &[2000.0], 37);
    let report = analyze(
        &book,
        &[
            RecorderWindow {
                start_frame: 0,
                samples: &a,
            },
            RecorderWindow {
                start_frame: 48_000,
                samples: &b,
            },
        ],
        &ModelConfig::default(),
    )
    .unwrap();
    assert_eq!(
        report
            .results
            .iter()
            .map(|r| r.source_index)
            .collect::<Vec<_>>(),
        vec![Some(4), Some(2)]
    );
}

#[test]
fn all_raw_samples_and_frozen_grid_are_validated_without_threshold_tuning() {
    let book = codes(4);
    let pcm = fixture(&book[2], 125, &[2000.0], 37);
    for (phase, scale, passes) in [
        (vec![0, 0], vec![0], vec![1]),
        (vec![4], vec![0], vec![1]),
        (vec![0], vec![100], vec![1]),
        (vec![0], vec![0], vec![3]),
    ] {
        let config = ModelConfig {
            phase_quarters: phase,
            scale_ppm: scale,
            filter_passes: passes,
            ..ModelConfig::default()
        };
        assert_eq!(
            analyze(
                &book,
                &[RecorderWindow {
                    start_frame: 0,
                    samples: &pcm
                }],
                &config
            )
            .unwrap_err(),
            ModelError::InvalidConfig
        );
    }
    for bad in [f32::NAN, f32::INFINITY, 1.0, -1.0] {
        let mut corrupt = pcm.clone();
        corrupt[0] = bad;
        assert_eq!(
            analyze(
                &book,
                &[RecorderWindow {
                    start_frame: 0,
                    samples: &corrupt
                }],
                &ModelConfig::default()
            )
            .unwrap_err(),
            ModelError::InvalidSamples
        );
    }
    let zero = vec![0.0; pcm.len()];
    assert_eq!(run(&book, &zero).results[0].status, Status::Unidentified);
    let mut duplicate = book.clone();
    duplicate[1] = duplicate[2].clone();
    assert_eq!(run(&duplicate, &pcm).results[0].status, Status::Ambiguous);
}
