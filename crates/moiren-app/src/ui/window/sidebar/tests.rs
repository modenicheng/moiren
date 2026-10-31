//! Sidebar cache tests (design 17 hazard 6): rebuild counters pair with
//! paint-output checks, never with the root's per-frame render count.

use std::time::Duration;

use super::StudioShell;
use crate::service::{AppRequest, GraphEdit};
use crate::ui::window::test_harness::{finish_service, settle_panels, test_shell, wait_until};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Entity, SharedString, TestAppContext, VisualTestContext, size};

/// One cached-path frame: notified views re-render, unnotified cached
/// subtrees replay their paint. `render_frame` refreshes the whole window and
/// would rebuild every panel regardless of the caches.
fn cached_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

fn inspector_rebuilds(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| {
        shell.inspector_panel.read(cx).rebuild_count()
    })
}

fn library_rebuilds(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| shell.library_panel.read(cx).rebuild_count())
}

fn rail_rebuilds(view: &Entity<StudioShell>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |shell, cx| {
        shell.library_rail_panel.read(cx).rebuild_count()
    })
}

/// Opens the shell on the settled graph page with the library expanded.
fn open_graph_page(cx: &mut VisualTestContext) {
    cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
    cx.update(|window, cx| window.render_frame(cx));
    settle_panels(cx);
}

/// Lets a panel-width spring run out without `render_frame`, which refreshes
/// the whole window and would rebuild every panel regardless of the caches.
/// The spring notifies through replaying caches, exactly as on screen.
fn settle_spring(cx: &mut VisualTestContext) {
    std::thread::sleep(Duration::from_millis(900));
    for _ in 0..3 {
        cached_frame(cx);
    }
}

/// The first shell notification after a window refresh rebuilds the
/// spring-wrapped sidebar hosts once (gpui-fast retained-frame bookkeeping);
/// every later notification holds. Burn that single rebuild off so the
/// measurements below start from the settled cache.
fn burn_in(
    view: &Entity<StudioShell>,
    rebuilds: impl Fn(&mut VisualTestContext) -> usize,
    cx: &mut VisualTestContext,
) {
    for _ in 0..3 {
        let stable = rebuilds(cx);
        view.update(cx, |_, cx| cx.notify());
        cached_frame(cx);
        if rebuilds(cx) == stable {
            return;
        }
    }
}

fn select_first_node(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(shell.state.read(cx).snapshot.graph.graph.nodes()[0].id());
        cx.notify();
    });
    cached_frame(cx);
}

/// The project name is shell state the inspector never renders: the edit plus
/// its notifications must leave the inspector on its cached paint. Selecting
/// a node afterwards is a key input and must repaint exactly once.
#[gpui_kit::test]
fn inspector_panel_stays_cached_across_key_neutral_shell_notifications(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    open_graph_page(cx);
    assert!(cx.update(|window, _| window.find("inspector-empty").visible()));
    let empty = cx.update(|window, _| window.find("inspector-empty").bounds());
    let before = inspector_rebuilds(&view, cx);

    view.update(cx, |shell, cx| {
        shell.project_name = "Probe project".into();
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        inspector_rebuilds(&view, cx),
        before,
        "a project-name edit must not reach the inspector"
    );
    assert_eq!(
        cx.update(|window, _| window.find("inspector-empty").bounds()),
        empty,
        "the cached paint must not move"
    );

    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        inspector_rebuilds(&view, cx),
        before,
        "a key-neutral notification must replay the cached paint"
    );

    select_first_node(&view, cx);
    assert_eq!(
        inspector_rebuilds(&view, cx),
        before + 1,
        "the selection edit must rebuild the inspector exactly once"
    );
    assert!(
        cx.update(|window, _| window.try_find("inspector-empty").is_none()),
        "the node editor must replace the empty state"
    );
    assert!(cx.update(|window, _| window.find("node-name-display").visible()));
    finish_service(runtime);
}

/// `page_inspector` switches its whole content on `presentation.page`, so a
/// page switch with a selection held must repaint the inspector into the new
/// page's inspector on that notification alone.
#[gpui_kit::test]
fn inspector_panel_follows_the_active_page_with_a_selection_held(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    open_graph_page(cx);
    select_first_node(&view, cx);
    assert!(cx.update(|window, _| window.find("node-name-display").visible()));
    let before = inspector_rebuilds(&view, cx);

    view.update(cx, |shell, cx| {
        shell.presentation.page = crate::ui::pages::Page::Matrix;
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        inspector_rebuilds(&view, cx),
        before + 1,
        "the page switch must repaint the inspector exactly once"
    );
    assert!(cx.update(|window, _| window.find("matrix-inspector").visible()));
    assert!(
        cx.update(|window, _| window.try_find("node-name-display").is_none()),
        "the graph node editor must not survive the page switch"
    );
    finish_service(runtime);
}

/// The library must ride on its key alone: shell notifications that move no
/// key input — the poll-tick shape — replay its cached paint, while a publish
/// that adds a session node repaints the list exactly once.
#[gpui_kit::test]
fn library_panel_stays_cached_across_key_neutral_shell_notifications(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    open_graph_page(cx);
    assert!(cx.update(|window, _| window.find("session-io-panel").visible()));
    let scroll = cx.update(|window, _| window.find("node-library-scroll").bounds());
    burn_in(&view, |cx| library_rebuilds(&view, cx), cx);
    let before = library_rebuilds(&view, cx);

    for _ in 0..2 {
        view.update(cx, |_, cx| cx.notify());
        cached_frame(cx);
    }
    assert_eq!(
        library_rebuilds(&view, cx),
        before,
        "key-neutral notifications must not reach the library panel"
    );
    assert_eq!(
        cx.update(|window, _| window.find("node-library-scroll").bounds()),
        scroll,
        "the cached paint must not move"
    );
    finish_service(runtime);
}

/// The library's session-node list renders from the published snapshot, so a
/// graph edit must list the new node on the publish's notification alone —
/// once — and hold the list across further key-neutral notifications.
#[gpui_kit::test]
fn library_panel_lists_a_published_node_exactly_once(cx: &mut TestAppContext) {
    use moiren_core::graph::NodeKind;
    let (view, cx, runtime) = test_shell(cx);
    open_graph_page(cx);
    let before = library_rebuilds(&view, cx);
    let known = view.update(cx, |shell, cx| session_node_ids(shell, cx));

    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Eq,
                channels: 2,
            }));
        });
    });
    // Land the publish without rendering: `render_frame` refreshes the whole
    // window and would rebuild every panel regardless of the caches.
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == known.len() + 1
        })
    });
    let added = view
        .update(cx, |shell, cx| {
            session_node_ids(shell, cx)
                .into_iter()
                .find(|id| !known.contains(id))
        })
        .expect("the published node must be projectable");
    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        library_rebuilds(&view, cx),
        before + 1,
        "the publish must rebuild the library exactly once"
    );
    // The session-node list sits below the template fold, so presence — not
    // visibility — is the paint witness for the published row.
    assert!(cx.update(|window, _| {
        window
            .try_find(SharedString::from(format!("locate-{added:?}")))
            .is_some()
    }));

    burn_in(&view, |cx| library_rebuilds(&view, cx), cx);
    let settled = library_rebuilds(&view, cx);
    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        library_rebuilds(&view, cx),
        settled,
        "a key-neutral notification must replay the cached list"
    );
    finish_service(runtime);
}

fn session_node_ids(
    shell: &StudioShell,
    cx: &mut gpui_kit::App,
) -> Vec<moiren_core::graph::NodeId> {
    shell
        .state
        .read(cx)
        .snapshot
        .graph
        .nodes
        .iter()
        .map(|node| node.id)
        .collect()
}

/// The selected fact the session-node row wrapper carries for tests; the
/// selected button variant itself is pure paint no snapshot can see.
fn row_selected(cx: &mut VisualTestContext, id: moiren_core::graph::NodeId) -> Option<bool> {
    cx.update(|window, _| {
        window
            .find(SharedString::from(format!("locate-row-{id:?}")))
            .selected()
    })
}

/// The session-node list styles each row from `selected_node`, so a selection
/// change with no publish behind it must rebuild the library exactly once and
/// move the selected style to the newly selected row.
#[gpui_kit::test]
fn library_panel_repaints_the_session_selection_without_a_publish(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    open_graph_page(cx);
    let nodes = view.update(cx, |shell, cx| session_node_ids(shell, cx));
    let (first, second) = (nodes[0], nodes[1]);

    view.update(cx, |shell, cx| {
        shell.selected_node = Some(first);
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        row_selected(cx, first),
        Some(true),
        "the selected row must paint the selected style"
    );
    assert_eq!(
        row_selected(cx, second),
        Some(false),
        "unselected rows must not paint the selected style"
    );

    burn_in(&view, |cx| library_rebuilds(&view, cx), cx);
    let before = library_rebuilds(&view, cx);
    view.update(cx, |shell, cx| {
        shell.selected_node = Some(second);
        cx.notify();
    });
    cached_frame(cx);
    assert_eq!(
        library_rebuilds(&view, cx),
        before + 1,
        "the selection edit must rebuild the library exactly once"
    );
    assert_eq!(
        row_selected(cx, first),
        Some(false),
        "the previous row must lose the selected style"
    );
    assert_eq!(
        row_selected(cx, second),
        Some(true),
        "the selected style must follow the selection"
    );
    finish_service(runtime);
}

/// The rail shares the library's key, so the same contract holds: key-neutral
/// notifications replay, a publish rebuilds once. The rail's pixels only move
/// with a pending spawn (unconstructible from a test), so its paint witness
/// is the unchanged icon column across all three frames.
#[gpui_kit::test]
fn library_rail_replays_key_neutral_notifications_and_rebuilds_on_a_publish(
    cx: &mut TestAppContext,
) {
    use moiren_core::graph::NodeKind;
    let (view, cx, runtime) = test_shell(cx);
    open_graph_page(cx);
    view.update(cx, |shell, cx| {
        shell.sidebar_open = false;
        cx.notify();
    });
    settle_spring(cx);
    assert!(cx.update(|window, _| window.find("library-rail").visible()));
    let rail = cx.update(|window, _| window.find("library-rail").bounds());
    burn_in(&view, |cx| rail_rebuilds(&view, cx), cx);
    let before = rail_rebuilds(&view, cx);

    for _ in 0..2 {
        view.update(cx, |_, cx| cx.notify());
        cached_frame(cx);
    }
    assert_eq!(
        rail_rebuilds(&view, cx),
        before,
        "key-neutral notifications must not reach the rail"
    );
    assert_eq!(
        cx.update(|window, _| window.find("library-rail").bounds()),
        rail,
        "the cached paint must not move"
    );

    let known = view.update(cx, |shell, cx| session_node_ids(shell, cx));
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::Eq,
                channels: 2,
            }));
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell.state.read(cx).snapshot.graph.nodes.len() == known.len() + 1
        })
    });
    view.update(cx, |_, cx| cx.notify());
    cached_frame(cx);
    assert_eq!(
        rail_rebuilds(&view, cx),
        before + 1,
        "the publish must rebuild the rail exactly once"
    );
    assert_eq!(
        cx.update(|window, _| window.find("library-rail").bounds()),
        rail,
        "a publish that moves no rail pixel must keep the icon column in place"
    );
    finish_service(runtime);
}
