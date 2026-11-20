use moiren_windows_audio::latency::{
    MarkerAnalysisConfig, MarkerAnalysisError, MarkerGenerator, analyze_marker, frames_to_seconds,
};

#[path = "common/alloc_guard.rs"]
mod alloc_guard;

const MARKER_FRAMES: usize = 511;
const AMPLITUDE: f32 = 0.1;

fn marker() -> Vec<f32> {
    let mut samples = vec![0.0; MARKER_FRAMES];
    let mut generator = MarkerGenerator::new(MARKER_FRAMES, AMPLITUDE).unwrap();
    assert_eq!(generator.fill(&mut samples), MARKER_FRAMES);
    samples
}

fn config(max_delay_frames: usize) -> MarkerAnalysisConfig {
    MarkerAnalysisConfig {
        max_marker_frames: 1024,
        max_recording_frames: 8192,
        max_delay_frames,
        max_correlation_samples: 8_000_000,
        min_correlation: 0.85,
        min_peak_margin: 0.15,
        min_rms: 0.0001,
    }
}

fn delayed(reference: &[f32], delay: usize, max_delay: usize, gain: f32) -> Vec<f32> {
    let mut recording = vec![0.0; reference.len() + max_delay];
    for (destination, sample) in recording[delay..].iter_mut().zip(reference) {
        *destination = sample * gain;
    }
    recording
}

fn add_noise(recording: &mut [f32], amplitude: f32) {
    let mut state = 0x1234_5678_u32;
    for sample in recording {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *sample += ((state >> 8) as f32 / 8_388_608.0 - 1.0) * amplitude;
    }
}

#[test]
fn detects_known_delays_including_zero_and_the_inclusive_search_limit() {
    let reference = marker();
    for delay in [0, 64, 128, 480, 4096] {
        let result = analyze_marker(
            &reference,
            &delayed(&reference, delay, delay, 1.0),
            config(delay),
        )
        .unwrap();
        assert_eq!(result.delay_frames, delay);
        assert!(result.confidence > 0.999999);
        assert!(result.peak_margin >= 0.15);
        assert!(!result.inverted);
    }
}

#[test]
fn normalization_handles_attenuation_polarity_and_bounded_noise() {
    let reference = marker();
    for gain in [0.04, -0.04, 0.5, -0.5] {
        let mut recording = delayed(&reference, 128, 480, gain);
        add_noise(&mut recording, gain.abs() * 0.025);
        let result = analyze_marker(&reference, &recording, config(480)).unwrap();
        assert_eq!(result.delay_frames, 128);
        assert!(result.confidence > 0.95);
        assert_eq!(result.inverted, gain < 0.0);
    }
}

#[test]
fn centered_correlation_tolerates_dc_offset() {
    let reference = marker();
    let mut recording = delayed(&reference, 64, 128, 0.25);
    for sample in &mut recording {
        *sample += 0.3;
    }
    let result = analyze_marker(&reference, &recording, config(128)).unwrap();
    assert_eq!(result.delay_frames, 64);
    assert!(result.confidence > 0.99999);
}

#[test]
fn silence_and_dc_do_not_become_zero_latency() {
    let reference = marker();
    for value in [0.0, 0.25, 0.000001] {
        assert_eq!(
            analyze_marker(&reference, &vec![value; MARKER_FRAMES + 64], config(64)),
            Err(MarkerAnalysisError::SilentRecording)
        );
    }
}

#[test]
fn missing_unrelated_and_noise_dominated_markers_are_rejected() {
    let reference = marker();
    let mut noise = vec![0.0; MARKER_FRAMES + 480];
    add_noise(&mut noise, 0.1);
    assert_eq!(
        analyze_marker(&reference, &noise, config(480)),
        Err(MarkerAnalysisError::NoMatch)
    );
    let mut buried = delayed(&reference, 64, 480, 0.01);
    add_noise(&mut buried, 0.1);
    assert_eq!(
        analyze_marker(&reference, &buried, config(480)),
        Err(MarkerAnalysisError::NoMatch)
    );
    let outside = delayed(&reference, 129, 480, 1.0);
    assert_eq!(
        analyze_marker(&reference, &outside, config(64)),
        Err(MarkerAnalysisError::NoMatch)
    );
}

#[test]
fn repeated_and_opposite_polarity_matches_are_ambiguous() {
    let reference = marker();
    for gain in [1.0, -1.0] {
        let mut recording = delayed(&reference, 0, 1024, 1.0);
        for (destination, sample) in recording[1024..].iter_mut().zip(&reference) {
            *destination = sample * gain;
        }
        assert_eq!(
            analyze_marker(&reference, &recording, config(1024)),
            Err(MarkerAnalysisError::Ambiguous)
        );
    }
}

#[test]
fn periodic_matches_are_ambiguous() {
    let reference: Vec<f32> = (0..256)
        .map(|index| if index % 2 == 0 { 0.1 } else { -0.1 })
        .collect();
    let recording: Vec<f32> = reference.iter().copied().cycle().take(384).collect();
    assert_eq!(
        analyze_marker(&reference, &recording, config(128)),
        Err(MarkerAnalysisError::Ambiguous)
    );
}

#[test]
fn truncated_observation_and_partial_marker_do_not_match() {
    let reference = marker();
    for length in [0, MARKER_FRAMES - 1, MARKER_FRAMES + 63] {
        assert_eq!(
            analyze_marker(&reference, &vec![0.0; length], config(64)),
            Err(MarkerAnalysisError::InsufficientObservation)
        );
    }
    let mut partial = delayed(&reference, 64, 128, 1.0);
    partial[64 + MARKER_FRAMES / 2..].fill(0.0);
    assert_eq!(
        analyze_marker(&reference, &partial, config(128)),
        Err(MarkerAnalysisError::NoMatch)
    );
}

#[test]
fn a_high_global_score_does_not_hide_a_truncated_marker_tail() {
    let reference = marker();
    for retained_percent in [80, 90] {
        let mut recording = delayed(&reference, 64, 128, 1.0);
        recording[64 + MARKER_FRAMES * retained_percent / 100..].fill(0.0);
        assert_eq!(
            analyze_marker(&reference, &recording, config(128)),
            Err(MarkerAnalysisError::NoMatch)
        );
    }
}

#[test]
fn extremely_small_rms_thresholds_cannot_turn_silence_into_a_perfect_match() {
    let reference = marker();
    let silence = vec![0.0; MARKER_FRAMES];
    let settings = MarkerAnalysisConfig {
        min_rms: 1e-300,
        ..config(0)
    };
    assert_eq!(
        analyze_marker(&reference, &silence, settings),
        Err(MarkerAnalysisError::SilentRecording)
    );
    assert_eq!(
        analyze_marker(&silence, &reference, settings),
        Err(MarkerAnalysisError::InvalidMarker)
    );
}

#[test]
fn clipped_and_non_finite_recordings_are_explicit_errors() {
    let reference = marker();
    for value in [1.0, -1.0, 1.5, -1.5] {
        let mut recording = delayed(&reference, 64, 128, 1.0);
        recording[100] = value;
        assert_eq!(
            analyze_marker(&reference, &recording, config(128)),
            Err(MarkerAnalysisError::ClippedRecording)
        );
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut recording = delayed(&reference, 64, 128, 1.0);
        recording[100] = value;
        assert_eq!(
            analyze_marker(&reference, &recording, config(128)),
            Err(MarkerAnalysisError::NonFiniteSamples)
        );
    }
}

#[test]
fn invalid_references_are_rejected() {
    let recording = vec![0.0; MARKER_FRAMES];
    for reference in [
        vec![],
        vec![0.1; 31],
        vec![0.0; MARKER_FRAMES],
        vec![0.1; MARKER_FRAMES],
        vec![1.0; MARKER_FRAMES],
        vec![f32::NAN; MARKER_FRAMES],
    ] {
        assert!(analyze_marker(&reference, &recording, config(0)).is_err());
    }
}

#[test]
fn analysis_rejects_invalid_thresholds_and_zero_budgets() {
    let reference = marker();
    for invalid in [f64::NAN, f64::INFINITY, -0.1, 0.0, 1.01] {
        for settings in [
            MarkerAnalysisConfig {
                min_correlation: invalid,
                ..config(0)
            },
            MarkerAnalysisConfig {
                min_peak_margin: invalid,
                ..config(0)
            },
            MarkerAnalysisConfig {
                min_rms: invalid,
                ..config(0)
            },
        ] {
            assert_eq!(
                analyze_marker(&reference, &reference, settings),
                Err(MarkerAnalysisError::InvalidConfig)
            );
        }
    }
    for settings in [
        MarkerAnalysisConfig {
            max_marker_frames: 0,
            ..config(0)
        },
        MarkerAnalysisConfig {
            max_recording_frames: 0,
            ..config(0)
        },
        MarkerAnalysisConfig {
            max_correlation_samples: 0,
            ..config(0)
        },
    ] {
        assert_eq!(
            analyze_marker(&reference, &reference, settings),
            Err(MarkerAnalysisError::InvalidConfig)
        );
    }
}

#[test]
fn analysis_enforces_caller_sizes_and_checked_work_budget_before_scanning() {
    let reference = marker();
    for settings in [
        MarkerAnalysisConfig {
            max_marker_frames: MARKER_FRAMES - 1,
            ..config(0)
        },
        MarkerAnalysisConfig {
            max_recording_frames: MARKER_FRAMES - 1,
            ..config(0)
        },
        MarkerAnalysisConfig {
            max_correlation_samples: MARKER_FRAMES - 1,
            ..config(0)
        },
        MarkerAnalysisConfig {
            max_delay_frames: usize::MAX,
            ..config(0)
        },
        MarkerAnalysisConfig {
            max_delay_frames: usize::MAX / 2,
            ..config(0)
        },
    ] {
        assert_eq!(
            analyze_marker(&reference, &reference, settings),
            Err(MarkerAnalysisError::LimitExceeded)
        );
    }
    let exact = MarkerAnalysisConfig {
        max_marker_frames: MARKER_FRAMES,
        max_recording_frames: MARKER_FRAMES,
        max_correlation_samples: MARKER_FRAMES,
        ..config(0)
    };
    assert_eq!(
        analyze_marker(&reference, &reference, exact)
            .unwrap()
            .delay_frames,
        0
    );
}

#[test]
fn sample_to_time_conversion_requires_an_explicit_valid_rate() {
    assert_eq!(frames_to_seconds(0, 48_000), Ok(0.0));
    assert_eq!(frames_to_seconds(480, 48_000), Ok(0.01));
    assert_eq!(frames_to_seconds(441, 44_100), Ok(0.01));
    assert_eq!(frames_to_seconds(48_000, 96_000), Ok(0.5));
    assert_eq!(
        frames_to_seconds(1, 0),
        Err(MarkerAnalysisError::InvalidSampleRate)
    );
}

#[test]
fn generator_is_chunk_invariant_bounded_and_writes_silence_after_the_marker() {
    let expected = marker();
    assert!(expected.iter().all(|sample| sample.abs() == AMPLITUDE));
    let mut actual = vec![42.0; MARKER_FRAMES + 97];
    let mut generator = MarkerGenerator::new(MARKER_FRAMES, AMPLITUDE).unwrap();
    assert_eq!(generator.fill(&mut []), 0);
    let written: usize = actual
        .chunks_mut(37)
        .map(|chunk| generator.fill(chunk))
        .sum();
    assert_eq!(written, MARKER_FRAMES);
    assert_eq!(&actual[..MARKER_FRAMES], expected);
    assert!(actual[MARKER_FRAMES..].iter().all(|sample| *sample == 0.0));
    assert_eq!(generator.fill(&mut actual), 0);
    assert!(actual.iter().all(|sample| *sample == 0.0));
}

#[test]
fn generator_validates_frame_count_and_amplitude() {
    for frames in [0, 1, 31] {
        assert!(MarkerGenerator::new(frames, AMPLITUDE).is_err());
    }
    for amplitude in [0.0, -0.1, 1.0, f32::NAN, f32::INFINITY] {
        assert!(MarkerGenerator::new(MARKER_FRAMES, amplitude).is_err());
    }
}

#[test]
fn generator_construction_fill_and_drop_allocate_and_free_nothing() {
    let mut storage = [0.0; 97];
    alloc_guard::assert_allocates_nothing(|| {
        let mut generator = MarkerGenerator::new(MARKER_FRAMES, AMPLITUDE).unwrap();
        for _ in 0..10 {
            generator.fill(&mut storage);
        }
    });
}

#[test]
fn small_fractional_phase_and_drift_still_find_the_nearest_frame() {
    let reference = marker();
    let mut recording = vec![0.0; MARKER_FRAMES + 128];
    for (index, sample) in recording[64..64 + MARKER_FRAMES].iter_mut().enumerate() {
        let position = index as f64 * 1.0001 + 0.15;
        let left = position.floor() as usize;
        let fraction = (position - left as f64) as f32;
        *sample = reference.get(left).copied().unwrap_or(0.0) * (1.0 - fraction)
            + reference.get(left + 1).copied().unwrap_or(0.0) * fraction;
    }
    let result = analyze_marker(&reference, &recording, config(128)).unwrap();
    assert_eq!(result.delay_frames, 64);
    assert!(result.confidence > 0.95);
}
