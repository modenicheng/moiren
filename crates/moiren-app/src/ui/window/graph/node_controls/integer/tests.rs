//! Pure integer admission and draft coverage; native events live in native_tests.
use super::*;
use crate::service::{RecordingPhase, ServiceContext};

fn fixture(kind: NodeKind, channels: usize) -> (ServiceContext, NodeId) {
    let mut context = ServiceContext::default();
    context
        .graph
        .edit(GraphEdit::AddNode { kind, channels })
        .unwrap();
    let node = context.graph.snapshot().nodes.last().unwrap().id;
    (context, node)
}

#[::core::prelude::v1::test]
fn integer_parser_accepts_only_decimal_digits_without_rounding_or_exponents() {
    for (text, expected) in [("1", 1), (" 0480 ", 480), ("1048576", 1_048_576)] {
        assert_eq!(parse_integer(text), Some(expected));
    }
    for text in [
        "",
        " ",
        "1.0",
        "1.5",
        "1e3",
        "+1",
        "-1",
        "NaN",
        "inf",
        "1,000",
        "1_000",
        "480 frames",
        "99999999999999999999999999999999999999999999999",
    ] {
        assert_eq!(parse_integer(text), None, "{text}");
    }
}

#[::core::prelude::v1::test]
fn meter_integer_bounds_and_service_admission_agree() {
    let (mut context, node) = fixture(NodeKind::Meter, 6);
    let key = (node, Field::MeterWindow);
    for value in [1, 480, 1_048_576] {
        let snapshot = crate::service::snapshot(&context);
        let edit = edit_for(&snapshot, key, &value.to_string()).unwrap();
        assert!(context.graph.edit(edit).unwrap());
        assert_eq!(
            value_and_maximum(&crate::service::snapshot(&context), key),
            Some((value, 1_048_576))
        );
    }
    for text in ["0", "1048577", "1.1", "1e6"] {
        assert!(edit_for(&crate::service::snapshot(&context), key, text).is_err());
    }
}

#[::core::prelude::v1::test]
fn recorder_integer_limit_uses_actual_channels_not_confirmed_file_metadata() {
    for channels in [1, 2, 64, 256] {
        let (mut context, node) = fixture(NodeKind::Recorder, channels);
        let key = (node, Field::RecorderCapacity);
        let maximum = RecorderSettings::max_capacity_frames(channels).unwrap();
        let mut snapshot = crate::service::snapshot(&context);
        snapshot
            .recordings
            .iter_mut()
            .find(|entry| entry.node == node)
            .unwrap()
            .channels = Some(2);
        assert_eq!(value_and_maximum(&snapshot, key).unwrap().1, maximum);
        for value in [1, maximum] {
            context
                .graph
                .edit(edit_for(&snapshot, key, &value.to_string()).unwrap())
                .unwrap();
            assert_eq!(
                context
                    .graph
                    .snapshot()
                    .node(node)
                    .unwrap()
                    .builtin
                    .recorder()
                    .unwrap()
                    .capacity_frames,
                value
            );
        }
        assert!(edit_for(&snapshot, key, "0").is_err());
        assert!(edit_for(&snapshot, key, &(maximum + 1).to_string()).is_err());
    }
}

#[::core::prelude::v1::test]
fn recorder_busy_phases_reject_capacity_but_do_not_disable_meter() {
    let (mut context, recorder) = fixture(NodeKind::Recorder, 2);
    context
        .graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Meter,
            channels: 2,
        })
        .unwrap();
    let meter = context.graph.snapshot().nodes.last().unwrap().id;
    let mut snapshot = crate::service::snapshot(&context);
    for phase in [
        RecordingPhase::Idle,
        RecordingPhase::Starting,
        RecordingPhase::Recording,
        RecordingPhase::Stopping,
        RecordingPhase::Canceled,
        RecordingPhase::Finished,
        RecordingPhase::Failed,
    ] {
        snapshot
            .recordings
            .iter_mut()
            .find(|entry| entry.node == recorder)
            .unwrap()
            .phase = phase;
        let key = (recorder, Field::RecorderCapacity);
        assert_eq!(recording_busy(&snapshot, key), phase.is_busy());
        assert_eq!(edit_for(&snapshot, key, "48000").is_err(), phase.is_busy());
        assert!(!recording_busy(&snapshot, (meter, Field::MeterWindow)));
        assert!(edit_for(&snapshot, (meter, Field::MeterWindow), "960").is_ok());
    }
}

#[::core::prelude::v1::test]
fn integer_edits_preserve_node_state_and_reject_missing_or_wrong_kinds() {
    let (mut context, node) = fixture(NodeKind::Meter, 2);
    context
        .graph
        .edit(GraphEdit::SetLabel {
            node,
            label: "Retained label".into(),
        })
        .unwrap();
    let before = context.graph.snapshot().node(node).unwrap().clone();
    let snapshot = crate::service::snapshot(&context);
    assert!(edit_for(&snapshot, (node, Field::RecorderCapacity), "48000").is_err());
    context
        .graph
        .edit(edit_for(&snapshot, (node, Field::MeterWindow), "1000").unwrap())
        .unwrap();
    let mut expected = before;
    expected.builtin = BuiltinState::Meter(moiren_engine::meter::MeterSettings {
        window_frames: 1000,
    });
    assert_eq!(context.graph.snapshot().node(node), Some(&expected));
    context.graph.edit(GraphEdit::RemoveNode(node)).unwrap();
    assert!(
        edit_for(
            &crate::service::snapshot(&context),
            (node, Field::MeterWindow),
            "480"
        )
        .is_err()
    );
}

#[::core::prelude::v1::test]
fn integer_pending_draft_waits_for_its_request_and_allows_fast_reversal() {
    let mut draft = Draft::new("480".into());
    assert!(draft.dirty("960"));
    draft.submitted("960".into(), RequestId(2));
    assert!(!draft.can_sync("960", false));
    draft.acknowledge(RequestId(1));
    assert_eq!(draft.pending, Some(RequestId(2)));
    assert!(
        draft.dirty("480"),
        "a return to the old snapshot is still new intent"
    );
    draft.submitted("480".into(), RequestId(3));
    draft.acknowledge(RequestId(2));
    assert!(!draft.can_sync("480", false));
    draft.acknowledge(RequestId(3));
    assert!(draft.can_sync("480", false));
    assert!(!draft.can_sync("480", true));
}

#[::core::prelude::v1::test]
fn integer_failed_or_invalid_drafts_remain_dirty_and_retryable() {
    let mut draft = Draft::new("480".into());
    draft.error = Some("Edit could not be sent; try again".into());
    assert!(draft.dirty("960"));
    assert!(!draft.can_sync("960", false));
    assert!(!draft.can_sync("invalid", false));
    assert_eq!(draft.baseline, "480");
    draft.submitted("960".into(), RequestId(1));
    assert!(draft.error.is_none());
    assert!(
        !draft.dirty("960"),
        "Enter followed by Blur does not resubmit"
    );
}
