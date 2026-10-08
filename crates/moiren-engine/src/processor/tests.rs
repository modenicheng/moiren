use super::*;
use crate::{
    buffer::{BufferArena, BufferSlotLayout, PortAccess},
    control::{ParamDomain, ParamSpec, parameter_channel},
    meter::level_meter,
    runtime::{Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources},
};
use moiren_core::protocol::{ParamValue, ParameterKey, ProcessorId};

const PROCESSOR: ProcessorId = ProcessorId(1);
const METER: ProcessorId = ProcessorId(2);

#[test]
fn bus_is_a_transform_with_sum_port_contract() {
    let arena = arena::<f64>(&[2, 2]);
    let io = arena
        .prepare_io(&[
            PortAccess::Read {
                port: 42,
                slot: arena.slot(0).unwrap(),
            },
            PortAccess::Write {
                port: 0,
                slot: arena.slot(1).unwrap(),
            },
        ])
        .unwrap();
    assert_eq!(RtProcessor::<f64>::role(&Bus), ProcessorRole::Transform);
    assert_eq!(RtProcessor::<f64>::latency_frames(&Bus), 0);
    RtProcessor::<f64>::validate_io(&Bus, &io).unwrap();
}

fn pan_modes<S: ProcessingSample>() {
    for in_place in [false, true] {
        for (position, left, right) in [
            (-1.0, 1.0, 0.0),
            (-0.5, 1.0, 0.5),
            (0.0, 1.0, 1.0),
            (0.5, 0.5, 1.0),
            (1.0, 0.0, 1.0),
        ] {
            let mut arena = arena::<S>(&[2, 2]);
            let original: &[&[f64]] = &[&[1.0, 2.0, -4.0, 8.0], &[2.0, -4.0, 8.0, 16.0]];
            seed(&mut arena, 0, original);
            seed(&mut arena, 1, &[&[-9.0; 4], &[-9.0; 4]]);
            let input = arena.slot(0).unwrap();
            let output = arena.slot(usize::from(!in_place)).unwrap();
            let io = arena
                .prepare_io(&if in_place {
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
                })
                .unwrap();
            let (_, parameters) =
                parameter_channel(vec![Pan::parameter(PROCESSOR, position)], 1, 1, 1, 16).unwrap();
            let bindings = parameters.bindings(PROCESSOR);
            let params = parameters.view(&bindings);
            RtProcessor::<S>::validate_io(&Pan, &io).unwrap();
            RtProcessor::<S>::validate_parameters(&Pan, params).unwrap();
            arena
                .with_io(&io, 1..4, |io| {
                    Pan.process(
                        &ProcessContext {
                            frames: 3,
                            ..CONTEXT
                        },
                        io,
                        params,
                    )
                })
                .unwrap();
            let untouched = if in_place { 1.0 } else { -9.0 };
            let untouched_right = if in_place { 2.0 } else { -9.0 };
            assert_samples(
                &mut arena,
                usize::from(!in_place),
                &[
                    &[untouched, 2.0 * left, -4.0 * left, 8.0 * left],
                    &[untouched_right, -4.0 * right, 8.0 * right, 16.0 * right],
                ],
            );
            if !in_place {
                assert_samples(&mut arena, 0, original);
            }
        }
    }
}

#[test]
fn pan_balances_stereo_and_preserves_window_for_both_modes_and_precisions() {
    pan_modes::<f32>();
    pan_modes::<f64>();
}

#[test]
fn pan_rejects_wrong_ports_channels_and_parameter_domains() {
    let arena = arena::<f64>(&[1, 2, 2, 3]);
    let stereo = arena.slot(1).unwrap();
    let input = PortAccess::Read {
        port: 0,
        slot: stereo,
    };
    let output = PortAccess::Write {
        port: 0,
        slot: arena.slot(2).unwrap(),
    };
    for accesses in [
        vec![],
        vec![input],
        vec![output],
        vec![
            PortAccess::Read {
                port: 1,
                slot: stereo,
            },
            output,
        ],
        vec![
            input,
            PortAccess::Write {
                port: 1,
                slot: arena.slot(2).unwrap(),
            },
        ],
        vec![
            input,
            output,
            PortAccess::Read {
                port: 1,
                slot: stereo,
            },
        ],
        vec![
            input,
            output,
            PortAccess::Write {
                port: 1,
                slot: arena.slot(3).unwrap(),
            },
        ],
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: arena.slot(0).unwrap(),
        }],
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: arena.slot(3).unwrap(),
        }],
        vec![PortAccess::InPlace {
            input: 1,
            output: 0,
            slot: stereo,
        }],
        vec![PortAccess::InPlace {
            input: 0,
            output: 1,
            slot: stereo,
        }],
        vec![
            PortAccess::Read {
                port: 0,
                slot: arena.slot(0).unwrap(),
            },
            output,
        ],
        vec![
            input,
            PortAccess::Write {
                port: 0,
                slot: arena.slot(3).unwrap(),
            },
        ],
    ] {
        let io = arena.prepare_io(&accesses).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Pan, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    for specs in [
        vec![],
        vec![Pan::parameter(METER, 0.0)],
        vec![ParamSpec {
            key: ParameterKey {
                processor: PROCESSOR,
                parameter: Pan::POSITION,
            },
            domain: ParamDomain::Bool,
            initial: ParamValue::Bool(false),
        }],
    ] {
        let (_, parameters) = parameter_channel(specs, 1, 1, 1, 16).unwrap();
        let bindings = parameters.bindings(PROCESSOR);
        assert_eq!(
            RtProcessor::<f64>::validate_parameters(&Pan, parameters.view(&bindings)),
            Err(ProcessorError::MissingParameter)
        );
    }
    let (_, parameters) = parameter_channel(
        vec![ParamSpec {
            domain: ParamDomain::Float {
                min: -2.0,
                max: 2.0,
            },
            ..Pan::parameter(PROCESSOR, 0.0)
        }],
        1,
        1,
        1,
        16,
    )
    .unwrap();
    let bindings = parameters.bindings(PROCESSOR);
    assert_eq!(
        RtProcessor::<f64>::validate_parameters(&Pan, parameters.view(&bindings)),
        Err(ProcessorError::InvalidParameterDomain)
    );
    for position in [f64::NAN, f64::INFINITY, -1.01, 1.01] {
        assert!(parameter_channel(vec![Pan::parameter(PROCESSOR, position)], 1, 1, 1, 16).is_err());
    }
}
const CONTEXT: ProcessContext = ProcessContext {
    timeline_epoch: 1,
    timeline_start: 17,
    frames: 4,
    processing_sr: 48000.0,
};

fn arena<S: ProcessingSample>(channels: &[usize]) -> BufferArena<S> {
    let layouts = channels
        .iter()
        .map(|&channels| BufferSlotLayout {
            channels,
            capacity_frames: 4,
        })
        .collect::<Vec<_>>();
    BufferArena::new(&layouts, 4096).unwrap()
}

fn seed<S: ProcessingSample>(arena: &mut BufferArena<S>, slot: usize, samples: &[&[f64]]) {
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(slot).unwrap(),
        }])
        .unwrap();
    arena
        .with_io(&io, 0..4, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                panic!("expected separate IO");
            };
            let mut block = outputs.get_mut(0).unwrap();
            assert_eq!(block.channel_count(), samples.len());
            for (channel, samples) in block.channels_mut().zip(samples) {
                assert_eq!(channel.len(), samples.len());
                for (dst, &src) in channel.iter_mut().zip(*samples) {
                    *dst = S::from_f64(src);
                }
            }
        })
        .unwrap();
}

fn assert_samples<S: ProcessingSample>(
    arena: &mut BufferArena<S>,
    slot: usize,
    expected: &[&[f64]],
) {
    let io = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: arena.slot(slot).unwrap(),
        }])
        .unwrap();
    arena
        .with_io(&io, 0..4, |io| {
            let ProcessIo::ReadOnly { inputs } = io else {
                panic!("expected read-only IO");
            };
            let block = inputs.get(0).unwrap();
            assert_eq!(block.channel_count(), expected.len());
            for (channel, expected) in block.channels().zip(expected) {
                assert_eq!(channel.len(), expected.len());
                for (&sample, &expected) in channel.iter().zip(*expected) {
                    assert_eq!(sample.to_f64(), expected);
                }
            }
        })
        .unwrap();
}

fn gain_modes<S: ProcessingSample>() {
    for in_place in [false, true] {
        let mut arena = arena::<S>(&[2, 2]);
        let input = arena.slot(0).unwrap();
        let output = arena.slot(usize::from(!in_place)).unwrap();
        let original: &[&[f64]] = &[&[1.0, 2.0, 3.0, 4.0], &[-1.0, -2.0, -3.0, -4.0]];
        seed(&mut arena, 0, original);
        seed(&mut arena, 1, &[&[-9.0; 4], &[-9.0; 4]]);
        let io = arena
            .prepare_io(&if in_place {
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
            })
            .unwrap();
        let (_, parameters) =
            parameter_channel(vec![Gain::parameter(PROCESSOR, 2.0)], 1, 1, 1, 16).unwrap();
        let bindings = parameters.bindings(PROCESSOR);
        let params = parameters.view(&bindings);
        let mut gain = Gain;
        RtProcessor::<S>::validate_io(&gain, &io).unwrap();
        RtProcessor::<S>::validate_parameters(&gain, params).unwrap();
        let ctx = ProcessContext {
            frames: 3,
            ..CONTEXT
        };
        arena
            .with_io(&io, 1..4, |io| gain.process(&ctx, io, params))
            .unwrap();
        if in_place {
            assert_samples(
                &mut arena,
                0,
                &[&[1.0, 4.0, 6.0, 8.0], &[-1.0, -4.0, -6.0, -8.0]],
            );
        } else {
            assert_samples(&mut arena, 0, original);
            assert_samples(
                &mut arena,
                1,
                &[&[-9.0, 4.0, 6.0, 8.0], &[-9.0, -4.0, -6.0, -8.0]],
            );
        }
    }
}

#[test]
fn gain_separate_and_in_place_preserve_channels_and_window_for_both_precisions() {
    gain_modes::<f32>();
    gain_modes::<f64>();
}

#[test]
fn gain_requires_a_float_level_bound_to_its_processor() {
    for specs in [
        vec![],
        vec![Gain::parameter(METER, 1.0)],
        vec![ParamSpec {
            key: ParameterKey {
                processor: PROCESSOR,
                parameter: Gain::LEVEL,
            },
            domain: ParamDomain::Bool,
            initial: ParamValue::Bool(true),
        }],
    ] {
        let (_, parameters) = parameter_channel(specs, 1, 1, 1, 16).unwrap();
        let bindings = parameters.bindings(PROCESSOR);
        assert_eq!(
            RtProcessor::<f64>::validate_parameters(&Gain, parameters.view(&bindings)),
            Err(ProcessorError::MissingParameter)
        );
    }
}

#[test]
fn gain_and_sum_reject_invalid_port_contracts() {
    let arena = arena::<f64>(&[1, 1, 2]);
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let c = arena.slot(2).unwrap();
    let input = PortAccess::Read { port: 0, slot: a };
    let output = PortAccess::Write { port: 0, slot: b };
    for ports in [
        vec![],
        vec![input],
        vec![input, PortAccess::Write { port: 1, slot: b }],
        vec![input, output, PortAccess::Write { port: 1, slot: c }],
        vec![input, PortAccess::Write { port: 0, slot: c }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Gain, &io),
            Err(ProcessorError::InvalidIo)
        );
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Sum, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    for ports in [
        vec![output],
        vec![PortAccess::Read { port: 1, slot: a }, output],
        vec![input, PortAccess::Read { port: 1, slot: a }, output],
        vec![PortAccess::InPlace {
            input: 1,
            output: 0,
            slot: a,
        }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&Gain, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    let io = arena
        .prepare_io(&[PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: a,
        }])
        .unwrap();
    assert_eq!(
        RtProcessor::<f64>::validate_io(&Sum, &io),
        Err(ProcessorError::InvalidIo)
    );
}

fn sum_inputs<S: ProcessingSample>() {
    let mut arena = arena::<S>(&[2, 2, 2, 2]);
    seed(
        &mut arena,
        0,
        &[&[1.0, 2.0, 3.0, 4.0], &[-1.0, -2.0, -3.0, -4.0]],
    );
    seed(&mut arena, 1, &[&[0.25, 0.5, 0.75, 1.0], &[1.0; 4]]);
    seed(&mut arena, 2, &[&[2.0; 4], &[4.0, 3.0, 2.0, 1.0]]);
    let io = arena
        .prepare_io(&[
            PortAccess::Read {
                port: 1,
                slot: arena.slot(0).unwrap(),
            },
            PortAccess::Read {
                port: 7,
                slot: arena.slot(1).unwrap(),
            },
            PortAccess::Read {
                port: 42,
                slot: arena.slot(2).unwrap(),
            },
            PortAccess::Write {
                port: 0,
                slot: arena.slot(3).unwrap(),
            },
        ])
        .unwrap();
    RtProcessor::<S>::validate_io(&Sum, &io).unwrap();
    let (_, parameters) = parameter_channel(vec![], 1, 1, 1, 16).unwrap();
    let bindings = parameters.bindings(PROCESSOR);
    arena
        .with_io(&io, 0..4, |io| {
            Sum.process(&CONTEXT, io, parameters.view(&bindings))
        })
        .unwrap();
    assert_samples(
        &mut arena,
        3,
        &[&[3.25, 4.5, 5.75, 7.0], &[4.0, 2.0, 0.0, -2.0]],
    );
}

#[test]
fn sum_mixes_multiple_inputs_per_channel_for_both_precisions() {
    sum_inputs::<f32>();
    sum_inputs::<f64>();
}

fn sum_render<S: ProcessingSample>(input_count: usize) {
    let mut arena = arena::<S>(&vec![2; input_count + 1]);
    let mut accesses = Vec::new();
    for index in 0..input_count {
        seed(&mut arena, index, &[&[0.25; 4], &[-0.5; 4]]);
        accesses.push(PortAccess::Read {
            port: index as u16,
            slot: arena.slot(index).unwrap(),
        });
    }
    // Poison the output to verify the executor supplies a fresh accumulator,
    // including for a bus with no inputs and on subsequent render calls.
    seed(&mut arena, input_count, &[&[99.0; 4], &[99.0; 4]]);
    let output = arena.slot(input_count).unwrap();
    accesses.push(PortAccess::Write {
        port: 0,
        slot: output,
    });
    let (meter, mut reader) = level_meter(1, 1).unwrap();
    let resources = RtResources::<S>::new(vec![
        ProcessorInstance::new(PROCESSOR, Sum),
        ProcessorInstance::new(METER, Observer(meter)),
    ])
    .unwrap();
    let (_, parameters) = parameter_channel(vec![], 1, 1, 1, 16).unwrap();
    let ops = vec![
        OpSpec {
            processor: PROCESSOR,
            io: arena.prepare_io(&accesses).unwrap(),
        },
        OpSpec {
            processor: METER,
            io: arena
                .prepare_io(&[PortAccess::Read {
                    port: 0,
                    slot: output,
                }])
                .unwrap(),
        },
    ];
    let plan = ExecutionPlan::prepare(
        arena,
        ops,
        &resources,
        &parameters,
        EngineConfig {
            processing_sr: 48000.0,
            max_block_frames: 4,
            max_events_per_block: 1,
        },
    )
    .unwrap();
    let mut engine = Engine::new(plan, resources, parameters).unwrap();
    let level = 0.25 * input_count as f64;
    for frames in [4, 3] {
        engine.render(frames).unwrap();
        let snapshot = reader.latest().unwrap();
        assert_eq!(snapshot.channels, 2);
        assert_eq!(snapshot.samples, (frames * 2) as u64);
        assert_eq!(snapshot.peak, level * 2.0);
        assert_eq!(snapshot.rms, (level * level * 2.5).sqrt());
    }
}

#[test]
fn sum_handles_empty_inputs_and_clears_stale_output_between_renders() {
    for inputs in [0, 3] {
        sum_render::<f32>(inputs);
        sum_render::<f64>(inputs);
    }
}

#[test]
fn observer_requires_read_only_io_and_delegates_input_validation_and_audio() {
    let mut arena = arena::<f64>(&[1, 1]);
    seed(&mut arena, 0, &[&[1.0, -2.0, 3.0, -4.0]]);
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let input = PortAccess::Read { port: 0, slot: a };
    let output = PortAccess::Write { port: 0, slot: b };
    let (meter, mut reader) = level_meter(1, 1).unwrap();
    let mut observer = Observer(meter);
    assert_eq!(RtProcessor::<f64>::role(&observer), ProcessorRole::Observer);
    for ports in [
        vec![],
        vec![PortAccess::Read { port: 1, slot: a }],
        vec![input, PortAccess::Read { port: 1, slot: a }],
        vec![output],
        vec![input, output],
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: a,
        }],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f64>::validate_io(&observer, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    let io = arena.prepare_io(&[input]).unwrap();
    RtProcessor::<f64>::validate_io(&observer, &io).unwrap();
    let (_, parameters) = parameter_channel(vec![], 1, 1, 1, 16).unwrap();
    let bindings = parameters.bindings(METER);
    arena
        .with_io(&io, 0..4, |io| {
            observer.process(&CONTEXT, io, parameters.view(&bindings))
        })
        .unwrap();
    let snapshot = reader.latest().unwrap();
    assert_eq!(snapshot.timeline_epoch, CONTEXT.timeline_epoch);
    assert_eq!(
        snapshot.end_frame,
        CONTEXT.timeline_start + CONTEXT.frames as u64
    );
    assert_eq!(snapshot.peak, 4.0);
    assert_eq!(snapshot.rms, 7.5_f64.sqrt());
    assert_samples(&mut arena, 0, &[&[1.0, -2.0, 3.0, -4.0]]);
}
