//! Render injected export snapshots in the real shell; this is not service or
//! native lifecycle evidence, and never qualifies physical audio latency.

use super::common;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, SharedString, TestAppContext, Window, WindowOptions, px, size};
use moiren_app::service::{
    ExportDiagnostics, ExportDirection, ExportIntent, ExportResourceState, ExportStatus,
    SessionPhase,
};
use moiren_app::ui::{PageCell, UiState};
use moiren_engine::node::VirtualEndpointId;
use std::sync::Arc;

// Independent literal expectations: never derive the oracle from presentation code.
const HELD_ROWS: [&str; 9] = [
    "Rename: Name pending; waiting for native recreation",
    "Endpoint: Removing / awaiting confirmation",
    "Gate admission: Closed",
    "Engine-confirmed binding: Inactive",
    "Diagnostic scope: Latest native export observation (not live audio)",
    "Observation generation: 41",
    "Native transport preparation: Not prepared",
    "Endpoint error: None reported",
    "Provider packet telemetry: Unavailable",
];

const REFUSED_ROWS: [&str; 9] = [
    "Rename: Name pending; waiting for native recreation",
    "Endpoint: Absent",
    "Gate admission: Closed",
    "Engine-confirmed binding: Inactive",
    "Diagnostic scope: Latest native export observation (not live audio)",
    "Observation generation: 42",
    "Native transport preparation: Not prepared",
    "Endpoint error: native recreation failed: endpoint name rejected",
    "Provider packet telemetry: Unavailable",
];

const RECREATED_ROWS: [&str; 8] = [
    "Endpoint: Present (not a flow signal)",
    "Gate admission: Closed",
    "Engine-confirmed binding: Inactive",
    "Diagnostic scope: Latest native export observation (not live audio)",
    "Observation generation: 43",
    "Native transport preparation: Prepared (native observation only)",
    "Endpoint error: None reported",
    "Provider packet telemetry: Unavailable",
];

fn held_export(direction: ExportDirection, name: &str, old_name: &str, id: u8) -> ExportStatus {
    ExportStatus {
        intent: ExportIntent {
            endpoint_id: VirtualEndpointId::from_bytes([id; 16]),
            direction,
            name: name.into(),
            channels: 2,
            enabled: true,
            binding: None,
        },
        actual_name: Some(old_name.into()),
        rename_pending: true,
        resource: ExportResourceState::Removing,
        allow_default: false,
        gate_open: false,
        active_revision: None,
        diagnostics: Some(ExportDiagnostics {
            generation: 41,
            native_stamp: None,
            transport_prepared: false,
            telemetry: None,
            error: None,
            windows_name: None,
        }),
        #[cfg(feature = "vad-integration-probe")]
        native_stamp: None,
        #[cfg(feature = "vad-integration-probe")]
        observation_generation: None,
        #[cfg(feature = "vad-integration-probe")]
        native_retirement: None,
        windows_name: None,
    }
}

fn assert_reachable_row(
    window: &mut Window,
    cx: &mut App,
    id: impl Into<gpui_kit::ElementId>,
    expected: &str,
) {
    let id = id.into();
    for _ in 0..80 {
        let row = window.find(id.clone());
        let bounds = row.bounds();
        let viewport = window.find("settings-scroll").bounds();
        if row.visible()
            && bounds.origin.y >= viewport.origin.y
            && bounds.origin.y + bounds.size.height <= viewport.origin.y + viewport.size.height
        {
            assert_eq!(row.label(), Some(expected));
            return;
        }
        window.scroll(
            "settings-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-180.))),
            cx,
        );
    }
    panic!("{expected} must be fully reachable in the compact diagnostics viewport");
}

fn assert_export_rows(
    window: &mut Window,
    cx: &mut App,
    id: &SharedString,
    names: &[&str; 6],
    lifecycle: &[&str],
) {
    window.scroll(
        "settings-scroll",
        gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(10_000.))),
        cx,
    );
    assert!(window.try_find("provider-export-diagnostics").is_some());
    assert!(window.try_find(id.clone()).is_some());
    assert!(
        !window
            .find((id.clone(), names.len() + lifecycle.len() - 1))
            .visible(),
        "compact acceptance must begin with clipped export rows and exercise real scrolling"
    );
    for (index, expected) in names.iter().chain(lifecycle).enumerate() {
        assert_reachable_row(window, cx, (id.clone(), index), expected);
    }
    // Exact row count and order also reject a stale Rename row after replacement.
    assert!(
        window
            .try_find((id.clone(), names.len() + lifecycle.len()))
            .is_none()
    );
    assert_reachable_row(
        window,
        cx,
        "diagnostics-physical-latency",
        "Physical end-to-end latency: Unknown / unqualified",
    );
}

fn accept_rename_snapshots(
    cx: &mut TestAppContext,
    status: ExportStatus,
    id: &'static str,
    held_names: [&str; 6],
    recreated_names: [&str; 6],
) {
    // The core-driver service supplies the shell handle only. These replacements
    // deliberately do not dispatch rename requests or exercise a native adapter.
    let runtime = common::service();
    let mut snapshot = (*common::wait_snapshot(&runtime.handle())).clone();
    snapshot.phase = SessionPhase::Idle;
    snapshot.exports = vec![status];
    cx.update(gpui_kit::init);
    let state = cx.new(|_| UiState::new(runtime.handle(), Arc::new(snapshot.clone())));
    let page = cx.new(|_| PageCell::default());
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                    gpui_kit::point(px(0.), px(0.)),
                    size(px(1024.), px(720.)),
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| moiren_app::ui::shell_for_tests(state.clone(), page, window, cx))
            },
        )
        .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("engine-statusbar").visible());
        window.click("open-settings", cx);
        window.click("settings-diagnostics", cx);
        assert!(window.find("settings-dialog").visible());
        let id = SharedString::from(id);
        assert_export_rows(window, cx, &id, &held_names, &HELD_ROWS);

        let status = &mut snapshot.exports[0];
        status.actual_name = None;
        status.windows_name = None;
        status.resource = ExportResourceState::Absent;
        let diagnostics = status.diagnostics.as_mut().unwrap();
        diagnostics.generation = 42;
        diagnostics.error = Some("native recreation failed: endpoint name rejected".into());
        state.update(cx, |state, cx| {
            state.snapshot = Arc::new(snapshot.clone());
            cx.notify();
        });
        window.render_frame(cx);
        let refused_names = [
            held_names[0],
            held_names[1],
            held_names[2],
            "Provider-created name: Unknown / not present",
            "Windows endpoint name: Unknown / not observed",
            "Windows name match: Pending native recreation",
        ];
        assert_export_rows(window, cx, &id, &refused_names, &REFUSED_ROWS);

        let status = &mut snapshot.exports[0];
        status.actual_name = Some(status.intent.name.clone());
        status.windows_name = None;
        status.rename_pending = false;
        status.resource = ExportResourceState::Present;
        let diagnostics = status.diagnostics.as_mut().unwrap();
        diagnostics.generation = 43;
        diagnostics.transport_prepared = true;
        diagnostics.error = None;
        // Successful native metadata/preparation alone is not Engine authority:
        // gate and confirmed binding remain closed/inactive in this fixture.
        state.update(cx, |state, cx| {
            state.snapshot = Arc::new(snapshot.clone());
            cx.notify();
        });
        window.render_frame(cx);
        assert_export_rows(window, cx, &id, &recreated_names, &RECREATED_ROWS);
    })
    .unwrap();
    common::finish(runtime);
}

#[gpui_kit::test]
fn playback_rendered_rename_refusal_and_fresh_replacement(cx: &mut TestAppContext) {
    accept_rename_snapshots(
        cx,
        held_export(
            ExportDirection::Playback,
            "Renamed playback",
            "Held playback",
            7,
        ),
        "provider-export-07070707070707070707070707070707",
        [
            "Stable endpoint ID: 07070707070707070707070707070707",
            "Export direction: Playback",
            "Requested endpoint name: Renamed playback",
            "Provider-created name: Held playback",
            "Windows endpoint name: Unknown / not observed",
            "Windows name match: Pending native recreation",
        ],
        [
            "Stable endpoint ID: 07070707070707070707070707070707",
            "Export direction: Playback",
            "Requested endpoint name: Renamed playback",
            "Provider-created name: Renamed playback",
            "Windows endpoint name: Unknown / not observed",
            "Windows name match: Unknown / not observed",
        ],
    );
}

#[gpui_kit::test]
fn microphone_rendered_rename_refusal_and_fresh_replacement(cx: &mut TestAppContext) {
    accept_rename_snapshots(
        cx,
        // Unicode is endpoint content, not a locale switch or qualified DTO.
        held_export(
            ExportDirection::Microphone,
            "\u{9ea6}\u{514b}\u{98ce} Renamed",
            "Held microphone",
            8,
        ),
        "provider-export-08080808080808080808080808080808",
        [
            "Stable endpoint ID: 08080808080808080808080808080808",
            "Export direction: Microphone",
            "Requested endpoint name: \u{9ea6}\u{514b}\u{98ce} Renamed",
            "Provider-created name: Held microphone",
            "Windows endpoint name: Unknown / not observed",
            "Windows name match: Pending native recreation",
        ],
        [
            "Stable endpoint ID: 08080808080808080808080808080808",
            "Export direction: Microphone",
            "Requested endpoint name: \u{9ea6}\u{514b}\u{98ce} Renamed",
            "Provider-created name: \u{9ea6}\u{514b}\u{98ce} Renamed",
            "Windows endpoint name: Unknown / not observed",
            "Windows name match: Unknown / not observed",
        ],
    );
}
