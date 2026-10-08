use super::*;

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
