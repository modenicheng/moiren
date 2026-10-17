use super::*;

fn render<S, P>(
    mut processor: P,
    specs: Vec<ParamSpec>,
    samples: &[Vec<f64>],
    chunks: &[usize],
    in_place: bool,
) -> Vec<Vec<f64>>
where
    S: ProcessingSample,
    P: RtProcessor<S>,
{
    let frames = samples[0].len();
    let channels = samples.len();
    assert_eq!(chunks.iter().sum::<usize>(), frames);
    let mut arena = BufferArena::<S>::new(
        &[BufferSlotLayout {
            channels,
            capacity_frames: frames,
        }; 2],
        frames * channels * 2 * std::mem::size_of::<S>(),
    )
    .unwrap();
    let input = arena.slot(0).unwrap();
    let output = arena.slot(usize::from(!in_place)).unwrap();
    let write = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: input,
        }])
        .unwrap();
    arena
        .with_io(&write, 0..frames, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                panic!()
            };
            for (destination, source) in outputs.get_mut(0).unwrap().channels_mut().zip(samples) {
                for (destination, source) in destination.iter_mut().zip(source) {
                    *destination = S::from_f64(*source);
                }
            }
        })
        .unwrap();
    let accesses = if in_place {
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
    let io = arena.prepare_io(&accesses).unwrap();
    let (_, mut parameters) =
        parameter_channel(specs, 1, 1, 4, frames as u64 + 1).unwrap();
    let bindings = parameters.bindings(PROCESSOR);
    assert_eq!(processor.validate_io(&io), Ok(()));
    assert_eq!(
        processor.validate_parameters(parameters.view(&bindings)),
        Ok(())
    );
    let mut start = 0;
    for &count in chunks {
        arena
            .with_io(&io, start..start + count, |io| {
                processor.process(
                    &ProcessContext {
                        timeline_start: start as u64,
                        frames: count,
                        processing_sr: 1_000.0,
                        ..CONTEXT
                    },
                    io,
                    parameters.view(&bindings),
                    crate::signal::ProcessSignals::empty(&mut ()),
                )
            })
            .unwrap();
        parameters.advance(count);
        start += count;
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
                .map(|channel| channel.iter().map(|sample| sample.to_f64()).collect())
                .collect()
        })
        .unwrap()
}

#[test]
fn gate_threshold_hold_and_range_are_block_continuous_in_both_precisions() {
    fn check<S: ProcessingSample>() {
        let settings = GateSettings {
            threshold_db: -20.0,
            attack_ms: 0.0,
            release_ms: 0.0,
            hold_ms: 1.0,
            range_db: -40.0,
        };
        let samples = vec![vec![0.0, 0.5, 0.0, 0.0, 0.0, 0.5, 0.0]];
        for in_place in [false, true] {
            let whole = render::<S, _>(
                Gate::new(),
                Gate::parameters(PROCESSOR, settings).to_vec(),
                &samples,
                &[7],
                in_place,
            );
            let split = render::<S, _>(
                Gate::new(),
                Gate::parameters(PROCESSOR, settings).to_vec(),
                &samples,
                &[1, 2, 1, 3],
                in_place,
            );
            assert_eq!(whole, split);
            assert!((whole[0][0] - 0.0).abs() < 1e-6);
            assert!((whole[0][1] - 0.5).abs() < 1e-6);
            assert!((whole[0][2] - 0.005).abs() < 1e-6);
            assert!((whole[0][5] - 0.5).abs() < 1e-6);
        }
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn limiter_caps_linked_peaks_in_both_precisions_and_variable_blocks() {
    fn check<S: ProcessingSample>() {
        let threshold = 10.0_f64.powf(-6.0 / 20.0);
        let samples = vec![vec![0.25, 2.0, 0.25, -2.0], vec![0.1, 0.1, 2.0, 0.1]];
        for in_place in [false, true] {
            let whole = render::<S, _>(
                Limiter::new(),
                Limiter::parameters(
                    PROCESSOR,
                    LimiterSettings {
                        threshold_db: -6.0,
                        release_ms: 0.0,
                        input_gain_db: 0.0,
                    },
                )
                .to_vec(),
                &samples,
                &[4],
                in_place,
            );
            let split = render::<S, _>(
                Limiter::new(),
                Limiter::parameters(
                    PROCESSOR,
                    LimiterSettings {
                        threshold_db: -6.0,
                        release_ms: 0.0,
                        input_gain_db: 0.0,
                    },
                )
                .to_vec(),
                &samples,
                &[1, 2, 1],
                in_place,
            );
            assert_eq!(whole, split);
            assert!((whole[0][1] - threshold).abs() < 2e-6);
            assert!((whole[1][2] - threshold).abs() < 2e-6);
            assert!(whole.iter().flatten().all(|sample| sample.abs() <= threshold + 2e-6));
        }
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn limiter_input_gain_automation_continues_across_variable_blocks() {
    fn check<S: ProcessingSample>() {
        use moiren_core::protocol::{ApplyAt, ParameterRequest, ReplyCode};
        let samples = vec![vec![0.25; 4]];
        let mut arena = BufferArena::<S>::new(
            &[BufferSlotLayout {
                channels: 1,
                capacity_frames: 4,
            }; 2],
            64,
        )
        .unwrap();
        let input = arena.slot(0).unwrap();
        let output = arena.slot(1).unwrap();
        let seed = arena
            .prepare_io(&[PortAccess::Write {
                port: 0,
                slot: input,
            }])
            .unwrap();
        arena
            .with_io(&seed, 0..4, |io| {
                let ProcessIo::Separate { mut outputs, .. } = io else {
                    panic!()
                };
                for sample in outputs.get_mut(0).unwrap().channel_mut(0) {
                    *sample = S::from_f64(0.25);
                }
            })
            .unwrap();
        let io = arena
            .prepare_io(&[
                PortAccess::Read { port: 0, slot: input },
                PortAccess::Write {
                    port: 0,
                    slot: output,
                },
            ])
            .unwrap();
        let (mut control, mut parameters) = parameter_channel(
            Limiter::parameters(
                PROCESSOR,
                LimiterSettings {
                    threshold_db: 0.0,
                    release_ms: 0.0,
                    input_gain_db: 0.0,
                },
            )
            .to_vec(),
            1,
            1,
            4,
            8,
        )
        .unwrap();
        assert_eq!(
            control
                .submit(
                    ParameterRequest {
                        request_id: 1,
                        plan_revision: 1,
                        timeline_epoch: 1,
                        target: ParameterKey {
                            processor: PROCESSOR,
                            parameter: Limiter::INPUT_GAIN,
                        },
                        at: ApplyAt::Frame(0),
                        value: ParamValue::Float(12.0),
                        ramp_frames: 4,
                    },
                    0,
                )
                .code,
            ReplyCode::Accepted
        );
        let bindings = parameters.bindings(PROCESSOR);
        let mut limiter = Limiter::new();
        let mut start = 0;
        for count in [1, 3] {
            let end = parameters.next_segment_end(start as u64, (start + count) as u64, &mut 8);
            assert_eq!(end, (start + count) as u64);
            arena
                .with_io(&io, start..start + count, |io| {
                    limiter.process(
                        &ProcessContext {
                            timeline_start: start as u64,
                            frames: count,
                            processing_sr: 1_000.0,
                            ..CONTEXT
                        },
                        io,
                        parameters.view(&bindings),
                        crate::signal::ProcessSignals::empty(&mut ()),
                    )
                })
                .unwrap();
            parameters.advance(count);
            start += count;
        }
        let read = arena
            .prepare_io(&[PortAccess::Read { port: 0, slot: output }])
            .unwrap();
        let actual = arena
            .with_io(&read, 0..4, |io| {
                let ProcessIo::ReadOnly { inputs } = io else {
                    panic!()
                };
                inputs
                    .get(0)
                    .unwrap()
                    .channel(0)
                    .iter()
                    .map(|sample| sample.to_f64())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        for (index, actual) in actual.iter().enumerate() {
            let db = (index + 1) as f64 * 3.0;
            assert!((actual - 0.25 * 10.0_f64.powf(db / 20.0)).abs() < 2e-6);
        }
    }
    check::<f32>();
    check::<f64>();
}
