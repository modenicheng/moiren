use super::*;

fn set(node: NodeId, parameter: RackParameter, value: f64) -> GraphEdit {
    GraphEdit::SetBuiltinParameter {
        node,
        parameter,
        value,
    }
}

#[test]
fn builtin_queued_fields_merge_latest_settings_with_distinct_undo_keys() {
    let initial = EqSettings {
        frequency_hz: 700.,
        gain_db: -3.,
        q: 2.,
    };
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::Eq(initial));
    for frequency in [1000., 1200.] {
        assert!(
            service
                .edit(set(node, RackParameter::EqFrequency, frequency))
                .unwrap()
        );
    }
    service.edit(set(node, RackParameter::EqGain, 6.)).unwrap();
    assert_eq!(
        *builtin(&service, node).eq().unwrap(),
        EqSettings {
            frequency_hz: 1200.,
            gain_db: 6.,
            q: 2.,
        }
    );
    assert_eq!(service.journal.undo.len(), 2);
    assert!(service.undo());
    assert_eq!(
        *builtin(&service, node).eq().unwrap(),
        EqSettings {
            frequency_hz: 1200.,
            ..initial
        }
    );
    assert!(service.undo());
    assert_eq!(*builtin(&service, node).eq().unwrap(), initial);
    assert!(service.redo());
    assert!(service.redo());
    assert_eq!(builtin(&service, node).eq().unwrap().gain_db, 6.);
}

#[test]
fn builtin_delay_preserves_storage_bound_and_uses_ms_and_signed_ratios() {
    let initial = DelaySettings {
        delay_ms: 123.,
        feedback: -0.5,
        mix: 0.4,
        max_delay_ms: 5000.,
    };
    let (mut service, node) = fixture(NodeKind::Delay, 2, 2, BuiltinState::Delay(initial));
    for (parameter, value) in [
        (RackParameter::DelayTime, 999.),
        (RackParameter::DelayFeedback, -0.999),
        (RackParameter::DelayMix, 0.75),
    ] {
        service.edit(set(node, parameter, value)).unwrap();
    }
    assert_eq!(
        *builtin(&service, node).delay().unwrap(),
        DelaySettings {
            delay_ms: 999.,
            feedback: -0.999,
            mix: 0.75,
            max_delay_ms: 5000.,
        }
    );
    assert_eq!(service.journal.undo.len(), 3);
    assert_eq!(
        service.snapshot().node(node).unwrap().label,
        "Preserved node label"
    );
}

#[test]
fn builtin_all_supported_scalar_fields_round_trip_in_engine_units() {
    use RackParameter::*;
    for (kind, edits) in [
        (
            NodeKind::Eq,
            vec![(EqFrequency, 2000.), (EqGain, -6.), (EqQ, 1.5)],
        ),
        (
            NodeKind::Gate,
            vec![
                (GateThreshold, -30.),
                (GateRange, -24.),
                (GateAttack, 4.),
                (GateHold, 12.),
                (GateRelease, 90.),
            ],
        ),
        (
            NodeKind::Limiter,
            vec![(LimitInput, 3.), (LimitThreshold, -2.), (LimitRelease, 80.)],
        ),
        (
            NodeKind::Delay,
            vec![(DelayTime, 400.), (DelayFeedback, -0.25), (DelayMix, 0.6)],
        ),
    ] {
        let (mut service, node) = fixture(kind, 2, 2, BuiltinState::None);
        for &(parameter, value) in &edits {
            assert!(service.edit(set(node, parameter, value)).unwrap());
        }
        let actual = builtin(&service, node);
        let values = match actual {
            BuiltinState::Eq(s) => vec![s.frequency_hz, s.gain_db, s.q],
            BuiltinState::Gate(s) => vec![
                s.threshold_db,
                s.range_db,
                s.attack_ms,
                s.hold_ms,
                s.release_ms,
            ],
            BuiltinState::Limiter(s) => vec![s.input_gain_db, s.threshold_db, s.release_ms],
            BuiltinState::Delay(s) => vec![s.delay_ms, s.feedback, s.mix],
            _ => panic!("wrong builtin kind"),
        };
        assert_eq!(
            values,
            edits.iter().map(|(_, value)| *value).collect::<Vec<_>>()
        );
        assert_eq!(service.journal.undo.len(), edits.len());
    }
}

#[test]
fn builtin_wrong_parameters_nonfinite_and_out_of_domain_values_reject_atomically() {
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::None);
    for (parameter, value) in [
        (RackParameter::EqQ, 0.),
        (RackParameter::EqFrequency, -1.),
        (RackParameter::EqGain, f64::NAN),
        (RackParameter::EqGain, f64::INFINITY),
        (RackParameter::EqGain, f64::NEG_INFINITY),
        (RackParameter::GateAttack, 1.),
        (RackParameter::Gain, 1.),
        (RackParameter::Pan, 0.),
        (
            RackParameter::Compressor(CompressorParameter::Threshold),
            -12.,
        ),
    ] {
        assert_rejected(&mut service, set(node, parameter, value));
    }
    for (parameter, value) in [
        (RackParameter::DelayMix, 1.1),
        (RackParameter::DelayFeedback, 1.),
        (RackParameter::DelayTime, 2001.),
    ] {
        let (mut service, node) = fixture(NodeKind::Delay, 2, 2, BuiltinState::None);
        assert_rejected(&mut service, set(node, parameter, value));
    }
    for kind in [
        NodeKind::Gain,
        NodeKind::Pan,
        NodeKind::Compressor,
        NodeKind::ChannelMapper,
        NodeKind::Rack,
    ] {
        let (mut service, node) = fixture(kind, 2, 2, BuiltinState::None);
        assert_rejected(&mut service, set(node, RackParameter::EqGain, 1.));
    }
}
