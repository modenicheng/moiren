use super::*;

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
