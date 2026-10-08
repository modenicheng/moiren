use super::*;

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
