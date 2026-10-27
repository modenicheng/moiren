use super::*;

fn pan_rack(enabled: bool) -> RackSettings {
    RackSettings {
        slots: vec![
            RackSlot {
                id: 1,
                bypass: enabled,
                processor: RackProcessor::Pan {
                    initial: 0.375,
                    mono: enabled,
                    swap: enabled,
                },
            },
            slot(2, RackProcessor::Gain { initial: 0.75 }),
        ],
        routes: vec![
            RackRoute {
                input_port: 0,
                output_port: 1,
                slots: vec![1],
            },
            RackRoute {
                input_port: 1,
                output_port: 0,
                slots: vec![2],
            },
        ],
    }
}

#[test]
fn toggles_twice_merge_latest_state_with_unchanged_epoch_and_pure_preview() {
    for enabled in [false, true] {
        let original = pan_rack(enabled);
        let (mut service, node) = fixture(original.clone(), &[2, 2], &[2, 2]);
        let generation = epoch(&service, node);
        for toggle in [
            RackEdit::ToggleBypass { slot: 1 },
            RackEdit::ToggleMono { slot: 1 },
            RackEdit::ToggleSwap { slot: 1 },
        ] {
            let mut expected = original.clone();
            match &toggle {
                RackEdit::ToggleBypass { .. } => expected.slots[0].bypass = !enabled,
                RackEdit::ToggleMono { .. } => {
                    let RackProcessor::Pan { mono, .. } = &mut expected.slots[0].processor else {
                        unreachable!()
                    };
                    *mono = !enabled;
                }
                RackEdit::ToggleSwap { .. } => {
                    let RackProcessor::Pan { swap, .. } = &mut expected.slots[0].processor else {
                        unreachable!()
                    };
                    *swap = !enabled;
                }
                _ => unreachable!(),
            }
            let preview = toggle.preview(&original, &[2, 2], &[2, 2]).unwrap();
            assert_eq!(preview, expected);
            assert_eq!(
                toggle.preview(&preview, &[2, 2], &[2, 2]).unwrap(),
                original
            );
            assert_eq!(
                original,
                pan_rack(enabled),
                "preview cannot mutate its input"
            );

            let request = GraphEdit::EditRack {
                node,
                epoch: generation,
                edit: toggle,
            };
            let before = service.snapshot();
            let undo_len = service.journal.undo.len();
            assert!(service.edit(request.clone()).unwrap());
            assert_eq!(settings(&service, node), expected);
            assert_eq!(epoch(&service, node), generation);
            assert!(service.edit(request).unwrap());
            assert_eq!(settings(&service, node), original);
            assert_eq!(epoch(&service, node), generation);
            assert_eq!(
                service.snapshot().candidate_revision,
                before.candidate_revision + 2
            );
            assert_eq!(service.journal.undo.len(), undo_len + 2);
        }
    }
}

#[test]
fn toggles_interleaved_with_parameter_edits_preserve_other_fields_and_routes() {
    let original = pan_rack(false);
    let (mut service, node) = fixture(original.clone(), &[2, 2], &[2, 2]);
    let generation = epoch(&service, node);
    for edit in [
        RackEdit::ToggleMono { slot: 1 },
        parameter(1, RackParameter::Pan, -0.25),
        RackEdit::ToggleSwap { slot: 1 },
        parameter(2, RackParameter::Gain, 0.5),
        RackEdit::ToggleBypass { slot: 1 },
        RackEdit::ToggleMono { slot: 1 },
        RackEdit::ToggleSwap { slot: 1 },
        RackEdit::ToggleBypass { slot: 1 },
    ] {
        service
            .edit(GraphEdit::EditRack {
                node,
                epoch: generation,
                edit,
            })
            .unwrap();
        assert_eq!(epoch(&service, node), generation);
    }
    let mut expected = original;
    expected.slots[0].processor = RackProcessor::Pan {
        initial: -0.25,
        mono: false,
        swap: false,
    };
    expected.slots[1].processor = RackProcessor::Gain { initial: 0.5 };
    assert_eq!(settings(&service, node), expected);
}

#[test]
fn toggles_have_discrete_undo_steps_and_never_coalesce_with_exact_assignments() {
    for (set_true, toggle, set_false) in [
        (
            RackEdit::SetMono {
                slot: 1,
                enabled: true,
            },
            RackEdit::ToggleMono { slot: 1 },
            RackEdit::SetMono {
                slot: 1,
                enabled: false,
            },
        ),
        (
            RackEdit::SetSwap {
                slot: 1,
                enabled: true,
            },
            RackEdit::ToggleSwap { slot: 1 },
            RackEdit::SetSwap {
                slot: 1,
                enabled: false,
            },
        ),
        (
            RackEdit::Bypass {
                slot: 1,
                bypass: true,
            },
            RackEdit::ToggleBypass { slot: 1 },
            RackEdit::Bypass {
                slot: 1,
                bypass: false,
            },
        ),
    ] {
        let original = pan_rack(false);
        let (mut service, node) = fixture(original.clone(), &[2, 2], &[2, 2]);
        let generation = epoch(&service, node);
        edit(&mut service, node, set_true.clone()).unwrap();
        let assigned = settings(&service, node);
        edit(&mut service, node, set_true).unwrap();
        assert_eq!(
            settings(&service, node),
            assigned,
            "exact assignment remains idempotent"
        );
        let undo_len = service.journal.undo.len();
        for edit in [toggle.clone(), toggle, set_false] {
            service
                .edit(GraphEdit::EditRack {
                    node,
                    epoch: generation,
                    edit,
                })
                .unwrap();
            assert_eq!(epoch(&service, node), generation);
        }
        assert_eq!(settings(&service, node), original);
        assert_eq!(service.journal.undo.len(), undo_len + 3);
        for expected in [&assigned, &original, &assigned] {
            assert!(service.undo());
            assert_eq!(&settings(&service, node), expected);
        }
        assert_eq!(service.journal.undo.len(), undo_len);
        for expected in [&original, &assigned, &original] {
            assert!(service.redo());
            assert_eq!(&settings(&service, node), expected);
        }
    }
}

#[test]
fn invalid_toggle_targets_and_stale_epochs_reject_atomically() {
    let original = pan_rack(false);
    let (mut service, node) = fixture(original.clone(), &[2, 2], &[2, 2]);
    let generation = epoch(&service, node);
    for edit in [
        RackEdit::ToggleMono { slot: 2 },
        RackEdit::ToggleSwap { slot: 2 },
        RackEdit::ToggleBypass { slot: 99 },
        RackEdit::ToggleMono { slot: 99 },
        RackEdit::ToggleSwap { slot: 99 },
    ] {
        assert!(edit.preview(&original, &[2, 2], &[2, 2]).is_err());
        assert_rejected(
            &mut service,
            GraphEdit::EditRack {
                node,
                epoch: generation,
                edit,
            },
        );
    }
    let saved = service.snapshot();
    service.replace(saved.graph, saved.nodes).unwrap();
    for edit in [
        RackEdit::ToggleBypass { slot: 1 },
        RackEdit::ToggleMono { slot: 1 },
        RackEdit::ToggleSwap { slot: 1 },
    ] {
        assert_rejected(
            &mut service,
            GraphEdit::EditRack {
                node,
                epoch: generation,
                edit,
            },
        );
    }
}

#[test]
fn toggle_bypass_rejects_width_breaking_mapper_without_changing_history() {
    let original = RackSettings {
        slots: vec![slot(
            1,
            RackProcessor::ChannelMapper(ChannelMapperSettings {
                mapping: vec![0, 0],
                matrix: Vec::new(),
            }),
        )],
        routes: Vec::new(),
    };
    let toggle = RackEdit::ToggleBypass { slot: 1 };
    assert!(toggle.preview(&original, &[1], &[2]).is_err());
    let (mut service, node) = fixture(original, &[1], &[2]);
    let generation = epoch(&service, node);
    assert_rejected(
        &mut service,
        GraphEdit::EditRack {
            node,
            epoch: generation,
            edit: toggle,
        },
    );
}
