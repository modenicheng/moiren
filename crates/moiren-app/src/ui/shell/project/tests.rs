//! Headless acceptance tests for the session menu, the status messages, and
//! the load/save/autosave paths.

use super::messages::{PersistedFailure, message_text_in};
use super::{Duration, ProjectDocument, ProjectMessage};
use crate::i18n::{ENGLISH, SIMPLIFIED_CHINESE, zh};
use crate::ui::UiState;
use crate::ui::shell::test_harness::{
    finish_service, initial_projection, shell_with_state, spawn_service, test_shell, wait_until,
};
use gpui_kit::AppContext;

#[test]
fn project_message_text_flips_with_the_locale() {
    // Static sentences relabel; en is the literal fallback.
    assert_eq!(
        message_text_in(&ProjectMessage::Saved, ENGLISH),
        "Project saved"
    );
    assert_eq!(
        message_text_in(&ProjectMessage::Saved, SIMPLIFIED_CHINESE),
        zh("Project saved")
    );
    assert_eq!(
        message_text_in(&ProjectMessage::LoadRequested, SIMPLIFIED_CHINESE),
        zh("Project load requested")
    );
    // Interpolated payloads keep their data across the flip; only the
    // wrapper sentence relabels.
    let failed = ProjectMessage::NotLoaded {
        cause: "stale revision".into(),
    };
    assert_eq!(
        message_text_in(&failed, ENGLISH),
        "Project was not loaded: stale revision"
    );
    assert_eq!(
        message_text_in(&failed, SIMPLIFIED_CHINESE),
        zh("Project was not loaded: %{error}").replace("%{error}", "stale revision")
    );
    // A persisted failure: the closed variant drives the sentence.
    let too_large = ProjectMessage::Persisted(PersistedFailure::TooLarge {
        path: std::path::PathBuf::from("Z:/sets/live.moiren.json"),
    });
    assert_eq!(
        message_text_in(&too_large, ENGLISH),
        "Document Z:/sets/live.moiren.json exceeds the 8 MiB limit"
    );
    assert_eq!(
        message_text_in(&too_large, SIMPLIFIED_CHINESE),
        zh("Document %{path} exceeds the 8 MiB limit")
            .replace("%{path}", "Z:/sets/live.moiren.json")
    );
}

#[gpui_kit::test]
fn project_presentation_waits_for_service_and_maps_sparse_document_ids(
    cx: &mut gpui_kit::TestAppContext,
) {
    let runtime = spawn_service();
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
    let mut document = ProjectDocument::from_graph("Sparse template", &snapshot.graph).unwrap();
    let graph = document.graph.as_mut().unwrap();
    for node in &mut graph.nodes {
        node.id += 42;
    }
    for edge in &mut graph.edges {
        edge.source_node += 42;
        edge.destination_node += 42;
    }
    document.layout.nodes.push(crate::project::NodeLayout {
        node_id: 42,
        x: -80.,
        y: 120.,
    });
    let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
    let (view, cx) = shell_with_state(cx, state);
    view.update(cx, |shell, cx| {
        shell
            .stage_project(document, "Sparse.moiren.json".into(), cx)
            .unwrap();
        assert_eq!(shell.project_io.name, "Untitled session");
        assert!(shell.project_io.pending.is_some());
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.finish_project_load(cx);
            shell.project_io.pending.is_none()
        })
    });
    view.update(cx, |shell, cx| {
        assert_eq!(shell.project_io.name, "Sparse template");
        let source = shell.state.read(cx).snapshot.graph.graph.nodes()[0].id();
        assert_eq!(
            shell.graph_view.node_positions.get(&source),
            Some(&(-80., 120.))
        );
        assert!(!shell.project_io.busy);
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn project_load_completion_cancels_canvas_gestures(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
    let document = ProjectDocument::from_graph("Loaded", &snapshot.graph).unwrap();
    view.update(cx, |shell, cx| {
        // A pan grab in flight when the load lands: the rubber-band
        // trigger is a drag running while the async load completes and
        // `apply_project_presentation` switches pages.
        shell.graph_gestures.pan_grab = Some((100., 100.));
        shell.sync_drag_demand();
        assert!(shell.drag_active());
        assert!(
            shell
                .stage_project(document, "Loaded.moiren.json".into(), cx)
                .is_ok()
        );
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.finish_project_load(cx);
            shell.project_io.pending.is_none()
        })
    });
    view.update(cx, |shell, cx| {
        assert_eq!(
            shell.project_io.name, "Loaded",
            "the load must have applied"
        );
        assert!(
            shell.graph_gestures.pan_grab.is_none() && !shell.drag_active(),
            "the project apply path must cancel the in-flight gesture"
        );
        assert!(
            !shell.frames_wanted(cx),
            "the DRAG demand must not outlive the load"
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn rejected_project_enqueue_preserves_current_name_and_layout(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let handle = runtime.handle();
    let snapshot = initial_projection(&handle);
    let document = ProjectDocument::from_graph("Rejected", &snapshot.graph).unwrap();
    finish_service(runtime);
    view.update(cx, |shell, cx| {
        let original = shell.state.read(cx).snapshot.graph.graph.nodes()[0].id();
        shell.graph_view.node_positions.insert(original, (10., 20.));
        assert!(
            shell
                .stage_project(document, "Rejected.moiren.json".into(), cx)
                .is_err()
        );
        assert_eq!(shell.project_io.name, "Untitled session");
        assert_eq!(
            shell.graph_view.node_positions.get(&original),
            Some(&(10., 20.))
        );
        assert!(shell.project_io.pending.is_none());
    });
}

#[gpui_kit::test]
fn project_processing_restores_without_starting_or_replacing_device_defaults(
    cx: &mut gpui_kit::TestAppContext,
) {
    use moiren_engine::audio_config::{ProcessingPrecision, ProcessingSettings, SampleFormat};

    let runtime = spawn_service();
    let handle = runtime.handle();
    let mut snapshot = (*initial_projection(&handle)).clone();
    snapshot.audio_settings.output.format = SampleFormat::Pcm24;
    snapshot.audio_settings.input.buffer_frames = Some(512);
    let hardware_defaults = snapshot.audio_settings.clone();
    let mut document = ProjectDocument::from_graph("96k template", &snapshot.graph).unwrap();
    let processing = ProcessingSettings {
        sample_rate: 96_000,
        precision: ProcessingPrecision::F64,
        block_frames: 1024,
    };
    document.processing = Some(processing);
    let state = cx.new(|_| UiState::new(handle.clone(), std::sync::Arc::new(snapshot)));
    let (view, cx) = shell_with_state(cx, state);
    view.update(cx, |shell, cx| {
        shell.apply_project_presentation(&document, cx).unwrap();
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.audio_settings.processing == processing
        })
    });
    view.update(cx, |shell, cx| {
        let state = shell.state.read(cx);
        assert_eq!(
            state.snapshot.audio_settings.output,
            hardware_defaults.output
        );
        assert_eq!(state.snapshot.audio_settings.input, hardware_defaults.input);
        assert!(state.snapshot.active_audio_settings.is_none());
        assert_eq!(state.snapshot.phase, crate::service::SessionPhase::Idle);
        assert_eq!(
            shell.project_document(cx).unwrap().processing,
            Some(processing)
        );
    });
    finish_service(runtime);
}

#[gpui_kit::test]
fn graph_edits_settle_into_the_autosave_file(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let autosave_file = std::env::temp_dir().join(format!(
        "moiren-autosave-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    view.update(cx, |shell, _| {
        shell.autosave.path = Some(autosave_file.clone());
    });
    let renamed = view.update(cx, |shell, cx| {
        let id = shell.state.read(cx).snapshot.graph.graph.nodes()[0].id();
        shell.state.update(cx, |state, _| {
            state.dispatch(crate::service::AppRequest::EditGraph(
                crate::service::GraphEdit::SetLabel {
                    node: id,
                    label: "Renamed by edit".into(),
                },
            ));
        });
        id
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node(renamed)
                .is_some_and(|node| node.label == "Renamed by edit")
        })
    });
    view.update(cx, |shell, cx| {
        shell.poll_autosave(cx);
        assert!(
            shell.autosave.timer.is_some(),
            "a settled graph change must arm the debounce"
        );
        // The test executor does not run the debounce timer, so run its
        // settle step directly; the poll loops drive the timer in
        // production.
        shell.autosave.timer = None;
        shell.autosave.deadline = None;
        shell.autosave_write_now(cx);
    });
    // The test executor runs spawned work only when driven.
    cx.run_until_parked();
    let document = crate::project::load_autosave_at(&autosave_file)
        .expect("the debounce settle step wrote the autosave");
    let graph = document.graph.expect("the autosave carries the graph");
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.label == "Renamed by edit")
    );
    // Design 23 §4.3: the graph carries every binding; a saved document
    // no longer writes a `session` section.
    assert!(document.session.is_none());
    let _ = std::fs::remove_file(&autosave_file);
    finish_service(runtime);
}

#[gpui_kit::test]
fn quitting_flushes_a_dirty_document_into_the_autosave_file(cx: &mut gpui_kit::TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    let autosave_file = std::env::temp_dir().join(format!(
        "moiren-autosave-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    view.update(cx, |shell, _| {
        shell.autosave.path = Some(autosave_file.clone());
    });
    let mut flush = None;
    view.update(cx, |shell, cx| {
        // A presentation-only change (camera pan) still dirties the
        // document, so the quit path must flush it.
        shell.graph_view.camera.pan_x = 128.0;
        flush = shell.autosave_flush(cx);
        assert!(flush.is_some(), "a dirty document must flush on quit");
    });
    cx.run_until_parked();
    let document = crate::project::load_autosave_at(&autosave_file)
        .expect("the quit flush wrote the autosave");
    assert_eq!(document.layout.camera_x, 128.0);
    drop(flush);
    let _ = std::fs::remove_file(&autosave_file);
    finish_service(runtime);
}

#[gpui_kit::test]
fn project_menu_lists_recents_and_closes_on_outside_press_and_escape(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{
        Modifiers, MouseButton, VisualTestContext, point, px, size, test::TestWindowExt,
    };

    fn render(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.render_frame(cx));
    }

    let (view, cx, runtime) = test_shell(cx);
    cx.update(|window, _| window.resize(size(px(1200.), px(800.))));
    view.update(cx, |shell, cx| {
        // Seeded preferences, no disk round trip: the menu lists the
        // quick-switch targets it was given.
        shell.preferences.values.recent_projects = vec![
            std::path::PathBuf::from("Z:/sets/live.moiren.json"),
            std::path::PathBuf::from("Z:/sets/rehearsal.moiren.json"),
        ];
        shell.chrome.project_menu = true;
        cx.notify();
    });
    render(cx);
    cx.update(|window, _| {
        assert!(window.find("project-popover").visible());
        assert!(window.find("recent-project-0").visible());
        assert!(window.find("recent-project-1").visible());
    });
    // A press on the workspace far from the popover closes the menu.
    let outside = point(px(1000.), px(700.));
    cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    view.update(cx, |shell, _| assert!(!shell.chrome.project_menu));
    render(cx);
    cx.update(|window, _| assert!(window.try_find("project-popover").is_none()));
    // Reopened, Escape closes it too.
    view.update(cx, |shell, cx| {
        shell.chrome.project_menu = true;
        cx.notify();
    });
    render(cx);
    cx.update(|window, _| assert!(window.find("project-popover").visible()));
    cx.simulate_keystrokes("escape");
    view.update(cx, |shell, _| assert!(!shell.chrome.project_menu));
    render(cx);
    cx.update(|window, _| assert!(window.try_find("project-popover").is_none()));
    finish_service(runtime);
}
