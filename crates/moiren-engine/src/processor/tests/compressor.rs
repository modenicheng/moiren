use super::*;

fn immediate() -> CompressorSettings {
    CompressorSettings {
        threshold_db: -12.0,
        ratio: 4.0,
        attack_ms: 0.0,
        release_ms: 0.0,
        knee_db: 0.0,
        ..CompressorSettings::default()
    }
}

fn db(value: f64) -> f64 {
    10.0_f64.powf(value / 20.0)
}

fn render<S: ProcessingSample>(
    settings: CompressorSettings,
    samples: &[Vec<f64>],
    chunks: &[usize],
    in_place: bool,
    sr: f64,
) -> Vec<Vec<f64>> {
    render_ramped::<S>(settings, samples, chunks, in_place, sr, None)
}

fn render_ramped<S: ProcessingSample>(
    settings: CompressorSettings,
    samples: &[Vec<f64>],
    chunks: &[usize],
    in_place: bool,
    sr: f64,
    automation: Option<(moiren_core::protocol::ParameterId, f64)>,
) -> Vec<Vec<f64>> {
    let frames = samples[0].len();
    assert_eq!(chunks.iter().sum::<usize>(), frames);
    let channels = samples.len();
    let mut arena = BufferArena::<S>::new(
        &[BufferSlotLayout {
            channels,
            capacity_frames: frames,
        }; 2],
        65536,
    )
    .unwrap();
    let input = arena.slot(0).unwrap();
    let output = arena.slot(usize::from(!in_place)).unwrap();
    let seed = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: input,
        }])
        .unwrap();
    arena
        .with_io(&seed, 0..frames, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                panic!()
            };
            for (dst, src) in outputs.get_mut(0).unwrap().channels_mut().zip(samples) {
                for (dst, src) in dst.iter_mut().zip(src) {
                    *dst = S::from_f64(*src);
                }
            }
        })
        .unwrap();
    let ports = if in_place {
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: input,
        }]
    } else {
        vec![
            PortAccess::Read {
                port: 0,
                slot: input,
            },
            PortAccess::Write {
                port: 0,
                slot: output,
            },
        ]
    };
    let io = arena.prepare_io(&ports).unwrap();
    let (mut control, mut parameters) = parameter_channel(
        Compressor::parameters(PROCESSOR, settings).to_vec(),
        1,
        1,
        16,
        64,
    )
    .unwrap();
    if let Some((parameter, target)) = automation {
        use moiren_core::protocol::{ApplyAt, ParameterRequest, ReplyCode};
        assert_eq!(
            control
                .submit(
                    ParameterRequest {
                        request_id: 1,
                        plan_revision: 1,
                        timeline_epoch: 1,
                        target: ParameterKey {
                            processor: PROCESSOR,
                            parameter
                        },
                        at: ApplyAt::Frame(0),
                        value: ParamValue::Float(target),
                        ramp_frames: 4
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    let bindings = parameters.bindings(PROCESSOR);
    let mut processor = Compressor::new();
    RtProcessor::<S>::validate_io(&processor, &io).unwrap();
    RtProcessor::<S>::validate_parameters(&processor, parameters.view(&bindings)).unwrap();
    let mut start = 0;
    for &frames in chunks {
        let ctx = ProcessContext {
            frames,
            timeline_start: start as u64,
            processing_sr: sr,
            ..CONTEXT
        };
        let mut remaining = 16;
        assert_eq!(
            parameters.next_segment_end(start as u64, (start + frames) as u64, &mut remaining),
            (start + frames) as u64
        );
        arena
            .with_io(&io, start..start + frames, |io| {
                processor.process(&ctx, io, parameters.view(&bindings))
            })
            .unwrap();
        parameters.advance(frames);
        start += frames;
    }
    let read = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: output,
        }])
        .unwrap();
    arena
        .with_io(&read, 0..frames, |io| {
            let ProcessIo::ReadOnly { inputs } = io else {
                panic!()
            };
            inputs
                .get(0)
                .unwrap()
                .channels()
                .map(|channel| channel.iter().map(|v| v.to_f64()).collect())
                .collect()
        })
        .unwrap()
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

fn same_samples(actual: &[Vec<f64>], expected: &[Vec<f64>]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.len(), expected.len());
        for (&actual, &expected) in actual.iter().zip(expected) {
            // Miri can vary the last bits of transcendental results between
            // calls. Block invariance is numerical, rather than bit identity.
            let tolerance = 32.0 * f64::EPSILON * actual.abs().max(expected.abs()).max(1.0);
            assert!(
                (actual - expected).abs() <= tolerance,
                "{actual} != {expected}"
            );
        }
    }
}

#[test]
fn compressor_hard_and_quadratic_soft_knee_include_both_boundaries() {
    for knee in [0.0, 6.0] {
        let settings = CompressorSettings {
            knee_db: knee,
            ..immediate()
        };
        let levels = [-24.0, -15.0, -12.0, -9.0, 0.0];
        let input = levels.iter().copied().map(db).collect();
        let output = render::<f64>(settings, &[input], &[5], false, 48000.0);
        let expected_db = if knee == 0.0 {
            [-24.0, -15.0, -12.0, -11.25, -9.0]
        } else {
            [-24.0, -15.0, -12.5625, -11.25, -9.0]
        };
        for (actual, expected) in output[0].iter().zip(expected_db) {
            close(*actual, db(expected));
        }
    }
}

#[test]
fn compressor_peak_links_every_channel_in_both_modes_and_precisions() {
    fn check<S: ProcessingSample>() {
        let input = vec![
            vec![0.01, -0.01],
            vec![0.1, -0.1],
            vec![1.0, -1.0],
            vec![0.0; 2],
        ];
        for in_place in [false, true] {
            let output = render::<S>(immediate(), &input, &[1, 1], in_place, 48000.0);
            for (src, dst) in input.iter().zip(&output) {
                for (src, dst) in src.iter().zip(dst) {
                    close(*dst, *src * db(-9.0));
                }
            }
        }
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn compressor_attack_and_release_are_db_time_constants_in_milliseconds() {
    let settings = CompressorSettings {
        attack_ms: 2.0,
        release_ms: 2.0,
        ..immediate()
    };
    let input = vec![vec![1.0, 1.0, 0.01, 0.01]];
    let output = render::<f64>(settings, &input, &[4], false, 1000.0);
    let one = 9.0 * (1.0 - (-0.5_f64).exp());
    let two = 9.0 * (1.0 - (-1.0_f64).exp());
    for (actual, (src, reduction)) in output[0].iter().zip([
        (1.0, one),
        (1.0, two),
        (0.01, two * (-0.5_f64).exp()),
        (0.01, two * (-1.0_f64).exp()),
    ]) {
        close(*actual, src * db(-reduction));
    }
}

#[test]
fn compressor_hold_delays_release_but_never_attack_and_persists_between_windows() {
    let settings = CompressorSettings {
        hold_ms: 2.0,
        ..immediate()
    };
    let input = vec![vec![1.0, 0.01, 0.01, 0.01, 1.0, 2.0, 0.01]];
    let whole = render::<f64>(settings, &input, &[7], false, 1000.0);
    let split = render::<f64>(settings, &input, &[1, 2, 1, 1, 2], true, 1000.0);
    same_samples(&whole, &split);
    for (actual, expected) in whole[0].iter().zip([
        db(-9.0),
        0.01 * db(-9.0),
        0.01 * db(-9.0),
        0.01,
        db(-9.0),
        db(-12.0 + (20.0 * 2.0_f64.log10() + 12.0) / 4.0),
        0.01 * db(-(20.0 * 2.0_f64.log10() + 12.0) * 0.75),
    ]) {
        close(*actual, expected);
    }
}

#[test]
fn compressor_dry_is_original_makeup_is_wet_and_output_is_after_mix() {
    let settings = CompressorSettings {
        input_gain_db: 6.0,
        makeup_gain_db: 3.0,
        output_gain_db: -2.0,
        mix: 0.25,
        ..immediate()
    };
    let output = render::<f64>(settings, &[vec![1.0]], &[1], false, 48000.0);
    close(
        output[0][0],
        (0.75 + 0.25 * db(6.0 - 18.0 * 0.75 + 3.0)) * db(-2.0),
    );
    let output = render::<f64>(
        CompressorSettings {
            mix: 0.0,
            ..settings
        },
        &[vec![1.0]],
        &[1],
        true,
        48000.0,
    );
    close(output[0][0], db(-2.0));
    let output = render::<f64>(
        CompressorSettings {
            ratio: 1.0,
            mix: 1.0,
            ..settings
        },
        &[vec![1.0]],
        &[1],
        false,
        48000.0,
    );
    close(output[0][0], db(7.0));
}

#[test]
fn compressor_silence_remains_finite_and_matches_variable_blocks() {
    let settings = CompressorSettings {
        attack_ms: 1.0,
        release_ms: 3.0,
        hold_ms: 1.0,
        knee_db: 6.0,
        ..immediate()
    };
    let input = vec![vec![0.0, 0.01, 1.0, -1.0, 0.0, 0.0, 0.01]];
    for in_place in [false, true] {
        let whole = render::<f64>(settings, &input, &[7], in_place, 1000.0);
        let split = render::<f64>(settings, &input, &[1, 3, 1, 2], in_place, 1000.0);
        same_samples(&whole, &split);
        assert!(whole[0].iter().all(|sample| sample.is_finite()));
        assert_eq!(whole[0][0], 0.0);
    }
}

#[test]
fn compressor_validates_all_ten_parameter_types_and_future_domains() {
    let defaults = Compressor::parameters(PROCESSOR, CompressorSettings::default());
    assert_eq!(defaults.len(), 10);
    assert_eq!(Compressor::OUTPUTGAIN, Compressor::OUTPUT_GAIN);
    for (index, spec) in defaults.iter().enumerate() {
        assert_eq!(
            spec.key.parameter,
            moiren_core::protocol::ParameterId(index as u32)
        );
    }
    for index in 0..10 {
        let mut specs = defaults.to_vec();
        specs.remove(index);
        let (_, runtime) = parameter_channel(specs, 1, 1, 1, 16).unwrap();
        let bindings = runtime.bindings(PROCESSOR);
        assert_eq!(
            RtProcessor::<f64>::validate_parameters(&Compressor::new(), runtime.view(&bindings)),
            Err(ProcessorError::MissingParameter)
        );
        let mut specs = defaults.to_vec();
        specs[index].domain = ParamDomain::Float {
            min: -1e100,
            max: 1e100,
        };
        let (_, runtime) = parameter_channel(specs, 1, 1, 1, 16).unwrap();
        let bindings = runtime.bindings(PROCESSOR);
        assert_eq!(
            RtProcessor::<f64>::validate_parameters(&Compressor::new(), runtime.view(&bindings)),
            Err(ProcessorError::InvalidParameterDomain)
        );
    }
}

#[test]
fn compressor_rejects_missing_wrong_numbered_or_mismatched_ports() {
    let arena = arena::<f64>(&[1, 1, 2]);
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let c = arena.slot(2).unwrap();
    for ports in [
        vec![],
        vec![PortAccess::Read { port: 0, slot: a }],
        vec![
            PortAccess::Read { port: 1, slot: a },
            PortAccess::Write { port: 0, slot: b },
        ],
        vec![
            PortAccess::Read { port: 0, slot: a },
            PortAccess::Write { port: 0, slot: c },
        ],
        vec![PortAccess::InPlace {
            input: 0,
            output: 1,
            slot: a,
        }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Compressor::new(), &io),
            Err(ProcessorError::InvalidIo)
        );
    }
}

#[test]
fn compressor_hold_counts_exact_samples_at_audio_sample_rates() {
    let settings = CompressorSettings {
        hold_ms: 2.0,
        ..immediate()
    };
    let mut samples = vec![0.01; 98];
    samples[0] = 1.0;
    let output = render::<f64>(settings, &[samples], &[7, 91], false, 48000.0);
    close(output[0][96], 0.01 * db(-9.0));
    close(output[0][97], 0.01);
}

#[test]
fn compressor_leaves_samples_outside_active_window_untouched() {
    fn check<S: ProcessingSample>() {
        for in_place in [false, true] {
            let mut arena = arena::<S>(&[2, 2]);
            seed(&mut arena, 0, &[&[1.0; 4], &[-1.0; 4]]);
            seed(&mut arena, 1, &[&[-9.0; 4], &[-9.0; 4]]);
            let slot = usize::from(!in_place);
            let input = arena.slot(0).unwrap();
            let output = arena.slot(slot).unwrap();
            let ports = if in_place {
                vec![PortAccess::InPlace {
                    input: 0,
                    output: 0,
                    slot: input,
                }]
            } else {
                vec![
                    PortAccess::Read {
                        port: 0,
                        slot: input,
                    },
                    PortAccess::Write {
                        port: 0,
                        slot: output,
                    },
                ]
            };
            let io = arena.prepare_io(&ports).unwrap();
            let (_, parameters) = parameter_channel(
                Compressor::parameters(PROCESSOR, immediate()).to_vec(),
                1,
                1,
                1,
                16,
            )
            .unwrap();
            let bindings = parameters.bindings(PROCESSOR);
            let mut compressor = Compressor::new();
            arena
                .with_io(&io, 1..3, |io| {
                    compressor.process(
                        &ProcessContext {
                            frames: 2,
                            ..CONTEXT
                        },
                        io,
                        parameters.view(&bindings),
                    )
                })
                .unwrap();
            let io = arena
                .prepare_io(&[PortAccess::Read {
                    port: 0,
                    slot: output,
                }])
                .unwrap();
            arena
                .with_io(&io, 0..4, |io| {
                    let ProcessIo::ReadOnly { inputs } = io else {
                        panic!()
                    };
                    for (channel_index, channel) in inputs.get(0).unwrap().channels().enumerate() {
                        let sign = if channel_index == 0 { 1.0 } else { -1.0 };
                        for frame in [0, 3] {
                            assert_eq!(channel[frame].to_f64(), if in_place { sign } else { -9.0 });
                        }
                        for frame in [1, 2] {
                            close(channel[frame].to_f64(), sign * db(-9.0));
                        }
                    }
                })
                .unwrap();
        }
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn compressor_every_parameter_samples_automation_and_preserves_cross_block_ramps() {
    fn check<S: ProcessingSample>() {
        for (parameter, target) in [
            (Compressor::INPUT_GAIN, 12.0),
            (Compressor::THRESHOLD, 0.0),
            (Compressor::RATIO, 8.0),
            (Compressor::ATTACK, 4.0),
            (Compressor::RELEASE, 4.0),
            (Compressor::HOLD, 4.0),
            (Compressor::KNEE, 24.0),
            (Compressor::MAKEUP_GAIN, 12.0),
            (Compressor::OUTPUT_GAIN, 12.0),
            (Compressor::MIX, 0.0),
        ] {
            let low_tail = parameter == Compressor::RELEASE || parameter == Compressor::HOLD;
            let samples = if low_tail {
                vec![1.0, 0.01, 0.01, 0.01]
            } else if parameter == Compressor::KNEE {
                vec![db(-12.0); 4]
            } else {
                vec![1.0; 4]
            };
            let samples = vec![samples];
            let automation = Some((parameter, target));
            let mut reductions = [0.0; 4];
            for index in 0..4 {
                let step = (index + 1) as f64;
                reductions[index] = match parameter {
                    Compressor::ATTACK => {
                        let previous = if index == 0 {
                            0.0
                        } else {
                            reductions[index - 1]
                        };
                        9.0 + (previous - 9.0) * (-1.0 / step).exp()
                    }
                    Compressor::RELEASE if index > 0 => reductions[index - 1] * (-1.0 / step).exp(),
                    _ => 9.0,
                };
            }
            let expected = (0..4)
                .map(|index| {
                    let step = (index + 1) as f64;
                    match parameter {
                        Compressor::INPUT_GAIN => db(-9.0 + step * 0.75),
                        Compressor::THRESHOLD => db(-9.0 + step * 2.25),
                        Compressor::RATIO => db(-12.0 * (1.0 - 1.0 / (4.0 + step))),
                        Compressor::KNEE => db(-12.0 - step * 0.5625),
                        Compressor::MAKEUP_GAIN | Compressor::OUTPUT_GAIN => db(-9.0 + step * 3.0),
                        Compressor::MIX => step / 4.0 + (1.0 - step / 4.0) * db(-9.0),
                        _ => samples[0][index] * db(-reductions[index]),
                    }
                })
                .collect::<Vec<_>>();
            for in_place in [false, true] {
                for chunks in [&[4][..], &[1, 2, 1][..]] {
                    let output = render_ramped::<S>(
                        immediate(),
                        &samples,
                        chunks,
                        in_place,
                        1000.0,
                        automation,
                    );
                    for (actual, expected) in output[0].iter().zip(&expected) {
                        close(*actual, *expected);
                    }
                }
            }
        }
    }
    check::<f32>();
    check::<f64>();
}
