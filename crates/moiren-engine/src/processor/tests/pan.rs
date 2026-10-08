use super::*;

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
