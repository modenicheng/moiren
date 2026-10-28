use super::*;
use crate::service::{MixerEdit, MixerState};

fn edit(rig: &mut Rig, node: NodeId, edit: MixerEdit) {
    rig.request(AppRequest::EditGraph(GraphEdit::EditMixer { node, edit }));
}

fn settled(rig: &Rig) -> bool {
    rig.driver.unsent.is_empty()
        && rig.driver.parameters.is_empty()
        && rig.c.control.accepted_pending == 0
        && rig.c.control.pending_node_parameters == 0
}

#[test]
fn mixer_intents_live_gain_dim_mute_and_balance_preserve_each_other_without_rebuild() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait(|r| r.sample() == 1.0 && settled(r));
    let source = rig.node(NodeKind::Source);
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    let starts = rig.state.starts.load(Ordering::SeqCst);
    let revision = rig.c.graph.snapshot().active_revision;
    // Queue all controls before any driver poll or snapshot publication.
    edit(&mut rig, source, MixerEdit::SetGain(0.5));
    edit(&mut rig, source, MixerEdit::ToggleDim);
    edit(&mut rig, source, MixerEdit::ToggleMute);
    edit(&mut rig, source, MixerEdit::SetBalance(0.5));
    rig.wait(|r| r.sample() == 0.0 && settled(r));
    edit(&mut rig, source, MixerEdit::ToggleMute);
    let dimmed = 0.25 * 10f64.powf(-12.0 / 20.0);
    rig.wait(|r| (r.sample() - dimmed).abs() < 1e-6 && settled(r));
    edit(&mut rig, source, MixerEdit::AdjustBalance(-0.25));
    edit(&mut rig, source, MixerEdit::AdjustBalance(-0.25));
    edit(&mut rig, source, MixerEdit::ToggleDim);
    rig.wait(|r| (r.sample() - 0.5).abs() < 1e-6 && settled(r));
    // Both toggles must reach the candidate even when the RT queue retains
    // only the last absolute parameter value for the target.
    edit(&mut rig, source, MixerEdit::ToggleMute);
    edit(&mut rig, source, MixerEdit::ToggleMute);
    rig.wait(settled);
    assert!((rig.sample() - 0.5).abs() < 1e-6);
    assert_eq!(
        rig.c.graph.snapshot().node(source).unwrap().mixer,
        MixerState {
            gain: 0.5,
            ..Default::default()
        }
    );
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), starts);
    assert_eq!(rig.c.graph.snapshot().active_revision, revision);
    assert!(rig.driver.auto_apply_deadline.is_none());
}

#[test]
fn mixer_intents_live_solo_fans_out_and_restores_effective_mutes() {
    let mut rig = Rig::new();
    rig.start();
    let source = rig.node(NodeKind::Source);
    rig.request(AppRequest::EditGraph(GraphEdit::AddNode {
        kind: NodeKind::Source,
        channels: 2,
    }));
    let other = rig.c.graph.snapshot().nodes.last().unwrap().id;
    rig.request(AppRequest::EditGraph(GraphEdit::BindSource {
        node: other,
        source: InputSelection::Tone {
            frequency_hz: 440.0,
        },
    }));
    rig.request(AppRequest::ApplyGraph);
    rig.wait(|r| {
        r.c.graph.snapshot().active_revision == Some(r.c.graph.snapshot().candidate_revision)
            && r.sample() == 1.0
            && settled(r)
    });
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    let starts = rig.state.starts.load(Ordering::SeqCst);
    let revision = rig.c.graph.snapshot().active_revision;
    // The isolated source's solo must silence the real output chain, not
    // merely flip the edited strip's raw mute field.
    assert!(rig.driver.strip_live.is_empty());
    edit(&mut rig, other, MixerEdit::ToggleSolo);
    rig.wait(|r| r.sample() == 0.0 && settled(r));
    assert_eq!(rig.driver.strip_live.get(&source), Some(&(1.0, true)));
    assert!(!rig.c.graph.snapshot().node(source).unwrap().mixer.muted);
    // Restoration must also work without relying on diagnostic cache history.
    rig.driver.strip_live.clear();
    edit(&mut rig, other, MixerEdit::ToggleSolo);
    rig.wait(|r| r.sample() == 1.0 && settled(r));
    edit(&mut rig, source, MixerEdit::ToggleSolo);
    rig.wait(|r| r.driver.strip_live.get(&other) == Some(&(1.0, true)) && settled(r));
    assert_eq!(rig.sample(), 1.0);
    edit(&mut rig, source, MixerEdit::ToggleSolo);
    rig.wait(|r| r.driver.strip_live.get(&other) == Some(&(1.0, false)) && settled(r));
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), starts);
    assert_eq!(rig.c.graph.snapshot().active_revision, revision);
    assert!(rig.driver.auto_apply_deadline.is_none());
}

#[test]
fn mixer_intents_live_pan_toggles_and_strip_mode_use_accepted_absolute_values() {
    let mut rig = Rig::new();
    rig.start();
    let pan = rig.node(NodeKind::Pan);
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    let starts = rig.state.starts.load(Ordering::SeqCst);
    let revision = rig.c.graph.snapshot().active_revision;
    rig.request(AppRequest::EditGraph(GraphEdit::SetPan {
        node: pan,
        pan: 0.5,
    }));
    rig.wait(|r| (r.sample() - 0.5).abs() < 1e-6 && settled(r));
    for (request, expected) in [
        (GraphEdit::ToggleSwap { node: pan }, 1.0),
        (GraphEdit::ToggleSwap { node: pan }, 0.5),
        (GraphEdit::ToggleMono { node: pan }, 0.75),
        (GraphEdit::ToggleMono { node: pan }, 0.5),
    ] {
        rig.request(AppRequest::EditGraph(request));
        rig.wait(|r| (r.sample() - expected).abs() < 1e-6 && settled(r));
    }
    for request in [
        GraphEdit::ToggleMono { node: pan },
        GraphEdit::ToggleSwap { node: pan },
    ] {
        rig.request(AppRequest::EditGraph(request.clone()));
        rig.request(AppRequest::EditGraph(request));
    }
    rig.wait(|r| (r.sample() - 0.5).abs() < 1e-6 && settled(r));
    let snapshot = rig.c.graph.snapshot();
    assert!(!snapshot.node(pan).unwrap().mono);
    assert!(!snapshot.node(pan).unwrap().swap);
    edit(&mut rig, pan, MixerEdit::CycleMode);
    rig.wait(|r| (r.sample() - 0.75).abs() < 1e-6 && settled(r));
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
    assert_eq!(rig.state.starts.load(Ordering::SeqCst), starts);
    assert_eq!(rig.c.graph.snapshot().active_revision, revision);
    assert!(rig.driver.auto_apply_deadline.is_none());
}

#[test]
fn mixer_intents_rejected_edits_submit_no_live_parameters() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait(|r| r.sample() == 1.0 && settled(r));
    let node = rig.node(NodeKind::Source);
    let before = rig.c.graph.snapshot();
    let prepares = rig.state.prepares.load(Ordering::SeqCst);
    let strip_live = rig.driver.strip_live.clone();
    for request in [
        GraphEdit::EditMixer {
            node,
            edit: MixerEdit::SetGain(17.0),
        },
        GraphEdit::EditMixer {
            node,
            edit: MixerEdit::SetBalance(f64::NAN),
        },
        GraphEdit::EditMixer {
            node,
            edit: MixerEdit::AdjustBalance(f64::INFINITY),
        },
        GraphEdit::ToggleMono { node },
        GraphEdit::ToggleSwap { node },
    ] {
        rig.request(AppRequest::EditGraph(request));
        assert!(settled(&rig));
        assert_eq!(rig.c.graph.snapshot().nodes, before.nodes);
        assert_eq!(
            rig.c.graph.snapshot().candidate_revision,
            before.candidate_revision
        );
        assert_eq!(rig.c.graph.snapshot().can_undo, before.can_undo);
        assert_eq!(rig.driver.strip_live, strip_live);
        assert!(rig.driver.auto_apply_deadline.is_none());
    }
    assert_eq!(rig.sample(), 1.0);
    assert_eq!(rig.state.prepares.load(Ordering::SeqCst), prepares);
}
