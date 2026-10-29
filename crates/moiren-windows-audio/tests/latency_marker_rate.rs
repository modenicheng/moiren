use moiren_windows_audio::latency::{
    MarkerAnalysisConfig, MarkerAnalysisError, MarkerGenerator, MarkerRateSearchConfig,
    analyze_marker, analyze_rate_aware_marker,
};

#[path = "common/alloc_guard.rs"]
mod alloc_guard;

fn config() -> MarkerAnalysisConfig {
    MarkerAnalysisConfig {
        max_marker_frames: 5000,
        max_recording_frames: 5100,
        max_delay_frames: 40,
        max_correlation_samples: 5000 * 41,
        min_correlation: 0.98,
        min_peak_margin: 0.15,
        min_rms: 0.0001,
    }
}

fn rates() -> MarkerRateSearchConfig {
    MarkerRateSearchConfig {
        min_rate_ppm: -2000,
        max_rate_ppm: 2000,
        rate_step_ppm: 500,
        max_hypotheses: 9,
        max_total_correlation_samples: 5000 * 41 * 9,
    }
}

fn marker() -> Vec<f32> {
    let mut marker = vec![0.0; 4096];
    MarkerGenerator::new(marker.len(), 0.1)
        .unwrap()
        .fill(&mut marker);
    marker
}

// Fixture is sampled in output time, independently from detector scratch.
fn recording(marker: &[f32], delay: usize, ppm: i32) -> Vec<f32> {
    (0..5100)
        .map(|frame| {
            let position = (frame as f64 - delay as f64) / (1.0 + f64::from(ppm) / 1_000_000.0);
            if position < 0.0 || position >= marker.len() as f64 {
                return 0.0;
            }
            let low = position.floor() as usize;
            let fraction = (position - low as f64) as f32;
            (marker[low] * (1.0 - fraction)
                + marker.get(low + 1).copied().unwrap_or(0.0) * fraction)
                * -0.4
        })
        .collect()
}

#[test]
fn rate_search_detects_real_resampled_markers_without_correcting_common_clock_delay() {
    let marker = marker();
    for ppm in [-2000, -1000, 0, 1000, 2000] {
        let recording = recording(&marker, 23, ppm);
        if ppm != 0 {
            assert!(analyze_marker(&marker, &recording, config()).is_err());
        }
        let result = analyze_rate_aware_marker(
            &marker,
            &recording,
            &mut [0.0; 5000],
            &mut [0.0; 41],
            config(),
            rates(),
        )
        .unwrap();
        assert_eq!(result.alignment.delay_frames, 23, "{ppm}");
        assert_eq!(result.template_rate_ppm, ppm);
        assert!(result.alignment.inverted);
        assert!(result.alignment.confidence > 0.99999);
        assert_eq!(result.onset_resolution_frames, 1);
    }
}

#[test]
fn rate_search_requires_complete_worst_case_observation_and_exact_budgets() {
    let marker = marker();
    let prepared = rates().prepare(marker.len(), config()).unwrap();
    let recording = recording(&marker, 23, 1000);
    let mut scratch = vec![0.0; prepared.max_template_frames];
    let mut scores = vec![0.0; 41];
    let result = analyze_rate_aware_marker(
        &marker,
        &recording[..prepared.max_template_frames + 39],
        &mut scratch,
        &mut scores,
        config(),
        rates(),
    );
    assert_eq!(result, Err(MarkerAnalysisError::InsufficientObservation));
    let mut limited = rates();
    limited.max_total_correlation_samples = prepared.correlation_samples - 1;
    assert_eq!(
        limited.prepare(marker.len(), config()),
        Err(MarkerAnalysisError::LimitExceeded)
    );
    limited.max_total_correlation_samples += 1;
    assert!(limited.prepare(marker.len(), config()).is_ok());
    assert_eq!(
        analyze_rate_aware_marker(
            &marker,
            &recording,
            &mut scratch[..prepared.max_template_frames - 1],
            &mut scores,
            config(),
            rates(),
        ),
        Err(MarkerAnalysisError::LimitExceeded)
    );
    assert_eq!(
        analyze_rate_aware_marker(
            &marker,
            &recording,
            &mut scratch,
            &mut scores[..40],
            config(),
            rates(),
        ),
        Err(MarkerAnalysisError::LimitExceeded)
    );
}

#[test]
fn rate_search_retains_silence_damage_clipping_and_off_grid_failures() {
    let marker = marker();
    let run = |recording: &[f32], config| {
        analyze_rate_aware_marker(
            &marker,
            recording,
            &mut [0.0; 5000],
            &mut [0.0; 41],
            config,
            rates(),
        )
    };
    assert_eq!(
        run(&[0.0; 5100], config()),
        Err(MarkerAnalysisError::SilentRecording)
    );
    for value in [f32::NAN, f32::INFINITY, 1.0, -1.0] {
        let mut raw = recording(&marker, 23, 1000);
        raw[5099] = value;
        assert_eq!(
            run(&raw, config()),
            Err(if value.is_finite() {
                MarkerAnalysisError::ClippedRecording
            } else {
                MarkerAnalysisError::NonFiniteSamples
            })
        );
    }
    let mut damaged = recording(&marker, 23, 1000);
    damaged[23 + 1024..23 + 2050].fill(0.0);
    let mut permissive = config();
    permissive.min_correlation = 0.8;
    assert!(
        run(&damaged, permissive).is_err(),
        "quarter erasure cannot hide in full score"
    );
    assert!(
        run(&recording(&marker, 23, 250), config()).is_err(),
        "off-grid distortion is unresolved, not silently accepted as zero ppm"
    );
    assert!(run(&recording(&marker, 23, 20_000), config()).is_err());
}

#[test]
fn rate_search_competes_distinct_onsets_across_all_rates_not_same_onset_hypotheses() {
    let mut marker = vec![0.0; 128];
    MarkerGenerator::new(128, 0.1).unwrap().fill(&mut marker);
    let config = MarkerAnalysisConfig {
        max_marker_frames: 256,
        max_recording_frames: 512,
        max_delay_frames: 256,
        max_correlation_samples: 256 * 257,
        min_correlation: 0.98,
        min_peak_margin: 0.15,
        min_rms: 0.0001,
    };
    let mut rates = rates();
    rates.max_total_correlation_samples = 256 * 257 * 9;
    let first = recording(&marker, 20, 2000);
    let mut raw = first[..512].to_vec();
    let hit = analyze_rate_aware_marker(
        &marker,
        &raw,
        &mut [0.0; 256],
        &mut [0.0; 257],
        config,
        rates,
    )
    .unwrap();
    assert_eq!(hit.alignment.delay_frames, 20);
    assert!(
        hit.template_rate_peak_margin < config.min_peak_margin,
        "nearby rate hypotheses must not count as alternate onsets"
    );
    for (sample, other) in raw.iter_mut().zip(recording(&marker, 200, -2000)) {
        *sample += other;
    }
    assert_eq!(
        analyze_rate_aware_marker(
            &marker,
            &raw,
            &mut [0.0; 256],
            &mut [0.0; 257],
            config,
            rates
        ),
        Err(MarkerAnalysisError::Ambiguous)
    );
}

#[test]
fn rate_search_rejects_invalid_grid_arithmetic_and_never_scans_before_admission() {
    for invalid in [
        MarkerRateSearchConfig {
            min_rate_ppm: -1_000_000,
            ..rates()
        },
        MarkerRateSearchConfig {
            min_rate_ppm: 3000,
            ..rates()
        },
        MarkerRateSearchConfig {
            rate_step_ppm: 0,
            ..rates()
        },
        MarkerRateSearchConfig {
            rate_step_ppm: 600,
            ..rates()
        },
        MarkerRateSearchConfig {
            max_hypotheses: 130,
            ..rates()
        },
        MarkerRateSearchConfig {
            max_hypotheses: 0,
            ..rates()
        },
    ] {
        assert_eq!(
            invalid.prepare(4096, config()),
            Err(MarkerAnalysisError::InvalidConfig)
        );
    }
    let limited = MarkerRateSearchConfig {
        max_hypotheses: 8,
        ..rates()
    };
    assert_eq!(
        analyze_rate_aware_marker(
            &[f32::NAN; 4096],
            &[f32::NAN; 5100],
            &mut [],
            &mut [],
            config(),
            limited
        ),
        Err(MarkerAnalysisError::LimitExceeded)
    );
    assert_eq!(
        rates().prepare(usize::MAX, config()),
        Err(MarkerAnalysisError::LimitExceeded)
    );
    let mut template = [9.0; 5000];
    let mut scores = [9.0; 41];
    assert!(
        analyze_rate_aware_marker(
            &marker(),
            &[0.0; 1],
            &mut template,
            &mut scores,
            config(),
            rates()
        )
        .is_err()
    );
    assert_eq!(template, [9.0; 5000]);
    assert_eq!(scores, [9.0; 41]);
}

#[test]
fn rate_search_counts_second_rate_score_pass_at_zero_delay() {
    let config = MarkerAnalysisConfig {
        max_delay_frames: 0,
        ..config()
    };
    let rates = MarkerRateSearchConfig {
        min_rate_ppm: 0,
        max_rate_ppm: 0,
        max_hypotheses: 1,
        max_total_correlation_samples: 8191,
        ..rates()
    };
    assert_eq!(
        rates.prepare(4096, config),
        Err(MarkerAnalysisError::LimitExceeded)
    );
    let prepared = MarkerRateSearchConfig {
        max_total_correlation_samples: 8192,
        ..rates
    }
    .prepare(4096, config)
    .unwrap();
    assert_eq!(prepared.correlation_samples, 8192);
}

#[test]
fn rate_search_degenerate_compressed_template_is_not_recording_silence() {
    let reference: Vec<_> = (0..64)
        .map(|i| if i % 2 == 0 { 0.1 } else { -0.1 })
        .collect();
    let rates = MarkerRateSearchConfig {
        min_rate_ppm: -500_000,
        max_rate_ppm: -500_000,
        max_hypotheses: 1,
        ..rates()
    };
    let mut config = config();
    config.max_delay_frames = 0;
    assert_eq!(
        analyze_rate_aware_marker(
            &reference,
            &reference,
            &mut [0.0; 64],
            &mut [0.0],
            config,
            rates
        ),
        Err(MarkerAnalysisError::InvalidMarker)
    );
}

#[test]
fn rate_search_allocates_and_frees_nothing_with_prepared_caller_scratch() {
    let marker = marker();
    let raw = recording(&marker, 23, 2000);
    let mut template = [0.0; 5000];
    let mut scores = [0.0; 41];
    let result = alloc_guard::assert_allocates_nothing(|| {
        analyze_rate_aware_marker(&marker, &raw, &mut template, &mut scores, config(), rates())
    })
    .unwrap();
    assert_eq!(result.alignment.delay_frames, 23);
    assert_eq!(result.template_rate_ppm, 2000);
    let settings = MarkerAnalysisConfig {
        max_marker_frames: 32,
        ..config()
    };
    let rates = MarkerRateSearchConfig {
        min_rate_ppm: -500_000,
        max_rate_ppm: -500_000,
        ..rates()
    };
    assert_eq!(
        rates.prepare(64, settings),
        Err(MarkerAnalysisError::LimitExceeded),
        "compressed output must not bypass the original source cap"
    );
}
