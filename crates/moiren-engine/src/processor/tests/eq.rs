use super::*;

fn render<S: ProcessingSample>(chunks: &[usize], in_place: bool) -> Vec<f64> {
    let frames = chunks.iter().sum::<usize>();
    let mut arena = BufferArena::<S>::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: frames,
        }; 2],
        frames * 2 * std::mem::size_of::<S>(),
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
            for (index, sample) in outputs
                .get_mut(0)
                .unwrap()
                .channel_mut(0)
                .iter_mut()
                .enumerate()
            {
                *sample = S::from_f64(f64::from(index == 0));
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
    let (_, parameters) = parameter_channel(
        Eq::parameters(
            PROCESSOR,
            EqSettings {
                frequency_hz: 1_000.0,
                gain_db: 12.0,
                q: 1.0,
            },
        )
        .to_vec(),
        1,
        1,
        1,
        frames as u64 + 1,
    )
    .unwrap();
    let bindings = parameters.bindings(PROCESSOR);
    let params = parameters.view(&bindings);
    let mut eq = Eq::new();
    RtProcessor::<S>::validate_io(&eq, &io).unwrap();
    RtProcessor::<S>::validate_parameters(&eq, params).unwrap();
    let mut start = 0;
    for &count in chunks {
        arena
            .with_io(&io, start..start + count, |io| {
                eq.process(
                    &ProcessContext {
                        timeline_start: start as u64,
                        frames: count,
                        ..CONTEXT
                    },
                    io,
                    params,
                    crate::signal::ProcessSignals::empty(&mut ()),
                )
            })
            .unwrap();
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
                .channel(0)
                .iter()
                .map(|sample| sample.to_f64())
                .collect()
        })
        .unwrap()
}

#[test]
fn eq_impulse_response_preserves_state_across_variable_blocks_in_both_precisions() {
    fn check<S: ProcessingSample>() {
        for in_place in [false, true] {
            let whole = render::<S>(&[8], in_place);
            let split = render::<S>(&[1, 3, 2, 2], in_place);
            for (left, right) in whole.iter().zip(&split) {
                assert!((left - right).abs() < 2e-6, "{left} != {right}");
            }
            let amplitude = 10.0_f64.powf(12.0 / 40.0);
            let alpha = (2.0 * std::f64::consts::PI * 1_000.0 / 48_000.0).sin() / 2.0;
            let expected_first = (1.0 + alpha * amplitude) / (1.0 + alpha / amplitude);
            assert!((whole[0] - expected_first).abs() < 2e-6);
            assert!(whole[1].abs() > 0.01);
        }
    }
    check::<f32>();
    check::<f64>();
}
