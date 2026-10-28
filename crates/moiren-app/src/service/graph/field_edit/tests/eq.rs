use super::*;

fn band(node: NodeId, frequency_hz: f64, gain_db: f64) -> GraphEdit {
    GraphEdit::SetEqBand {
        node,
        frequency_hz,
        gain_db,
    }
}

fn exact_q(node: NodeId, value: f64) -> GraphEdit {
    GraphEdit::SetBuiltinParameter {
        node,
        parameter: RackParameter::EqQ,
        value,
    }
}

#[test]
fn eq_band_samples_undo_together_without_reverting_prior_exact_q() {
    let initial = EqSettings {
        frequency_hz: 700.,
        gain_db: -3.,
        q: 2.,
    };
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::Eq(initial));
    service.edit(exact_q(node, 1.27)).unwrap();
    let before_band = service.snapshot();
    // Simulate queued drag samples without replacing settings from a UI snapshot.
    for (frequency_hz, gain_db) in [(1000., 1.), (1200., 3.), (1500., 6.)] {
        assert!(service.edit(band(node, frequency_hz, gain_db)).unwrap());
        let mut expected = before_band.node(node).unwrap().clone();
        expected.builtin = BuiltinState::Eq(EqSettings {
            frequency_hz,
            gain_db,
            q: 1.27,
        });
        assert_eq!(service.snapshot().node(node), Some(&expected));
        assert_eq!(service.snapshot().graph, before_band.graph);
    }
    assert_eq!(service.journal.undo.len(), 2);
    assert!(service.undo());
    assert_eq!(service.snapshot().nodes, before_band.nodes);
    assert!(service.undo());
    assert_eq!(*builtin(&service, node).eq().unwrap(), initial);
    assert!(!service.undo());
    assert!(service.redo());
    assert!(service.redo());
    assert_eq!(
        *builtin(&service, node).eq().unwrap(),
        EqSettings {
            frequency_hz: 1500.,
            gain_db: 6.,
            q: 1.27,
        }
    );
}

#[test]
fn eq_q_queued_steps_multiply_latest_candidate_and_keep_band_untouched() {
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::None);
    service.edit(band(node, 1234.56, -7.25)).unwrap();
    let before = service.snapshot().node(node).unwrap().clone();
    for q in [1.5, 2.25, 3.375] {
        assert!(
            service
                .edit(GraphEdit::AdjustEqQ { node, factor: 1.5 })
                .unwrap()
        );
        let mut expected = before.clone();
        expected.builtin = BuiltinState::Eq(EqSettings {
            frequency_hz: 1234.56,
            gain_db: -7.25,
            q,
        });
        assert_eq!(service.snapshot().node(node), Some(&expected));
    }
    assert_eq!(service.journal.undo.len(), 2);
    assert!(service.undo());
    assert_eq!(service.snapshot().node(node), Some(&before));
    assert!(service.redo());
    assert_eq!(builtin(&service, node).eq().unwrap().q, 3.375);
}

#[test]
fn eq_exact_q_and_wheel_share_one_undo_key_in_both_orders() {
    let initial = EqSettings::default();
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::Eq(initial));
    let wheel = GraphEdit::AdjustEqQ { node, factor: 1.5 };
    assert_eq!(wheel.undo_key(), exact_q(node, 2.).undo_key());
    assert_ne!(wheel.undo_key(), band(node, 1200., 3.).undo_key());
    for edits in [
        [exact_q(node, 2.), wheel.clone(), wheel.clone()],
        [wheel.clone(), exact_q(node, 3.), wheel],
    ] {
        for edit in edits {
            service.edit(edit).unwrap();
        }
        assert_eq!(
            *builtin(&service, node).eq().unwrap(),
            EqSettings { q: 4.5, ..initial }
        );
        assert_eq!(service.journal.undo.len(), 1);
        assert!(service.undo());
        assert_eq!(*builtin(&service, node).eq().unwrap(), initial);
        assert!(!service.undo());
    }
}

#[test]
fn eq_q_steps_clamp_finite_positive_factors_including_product_overflow() {
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::None);
    service.edit(exact_q(node, 2.)).unwrap();
    for (factor, expected) in [
        (1., 2.),
        (f64::MAX, 20.),
        (2., 20.),
        (0.5, 10.),
        (f64::MIN_POSITIVE, 0.1),
        (0.5, 0.1),
        (2., 0.2),
        (f64::from_bits(1), 0.1),
    ] {
        service.edit(GraphEdit::AdjustEqQ { node, factor }).unwrap();
        assert_eq!(builtin(&service, node).eq().unwrap().q, expected);
    }
}

#[test]
fn eq_band_and_q_invalid_values_reject_atomically_without_rewriting_history() {
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::None);
    service.edit(exact_q(node, 1.27)).unwrap();
    service.edit(band(node, 1200., 3.)).unwrap();
    assert!(service.undo());
    for (frequency_hz, gain_db) in [
        (19., 6.),
        (20_001., 6.),
        (f64::NAN, 6.),
        (f64::INFINITY, 6.),
        (f64::NEG_INFINITY, 6.),
        (2000., -24.1),
        (2000., 24.1),
        (2000., f64::NAN),
        (2000., f64::INFINITY),
        (2000., f64::NEG_INFINITY),
    ] {
        assert_rejected(&mut service, band(node, frequency_hz, gain_db));
    }
    for factor in [0., -0., -1., f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_rejected(&mut service, GraphEdit::AdjustEqQ { node, factor });
    }
    assert!(service.redo());
    assert_eq!(
        *builtin(&service, node).eq().unwrap(),
        EqSettings {
            frequency_hz: 1200.,
            gain_db: 3.,
            q: 1.27,
        }
    );
}

#[test]
fn eq_gestures_require_eq_topology_and_band_accepts_engine_domain_endpoints() {
    for kind in [
        NodeKind::Gain,
        NodeKind::Gate,
        NodeKind::Rack,
        NodeKind::ChannelMapper,
    ] {
        let (mut service, node) = fixture(kind, 2, 2, BuiltinState::None);
        assert_rejected(&mut service, band(node, 1000., 3.));
        assert_rejected(&mut service, GraphEdit::AdjustEqQ { node, factor: 1.5 });
    }
    let (mut service, node) = fixture(NodeKind::Eq, 2, 2, BuiltinState::None);
    for (frequency_hz, gain_db) in [(20., -24.), (20_000., 24.)] {
        service.edit(band(node, frequency_hz, gain_db)).unwrap();
        assert_eq!(
            *builtin(&service, node).eq().unwrap(),
            EqSettings {
                frequency_hz,
                gain_db,
                q: 1.
            }
        );
    }
}
