//! Render production diagnostics without opening a native audio device.

use super::common;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, Window, WindowOptions, px, size};
use moiren_app::service::{
    AppSnapshot, NativeSessionObservations, SessionGeneration, SessionPhase,
};
use moiren_app::ui::{PageCell, UiState};
use moiren_windows_audio::{
    clock_bridge::{BridgeSnapshot, QueuedFrameAge, QueuedFrameStamp},
    engine_backend::{
        NativeBridgeObservation, NativeObservationSnapshot, NativeRenderPaddingObservation,
        NativeStreamKind, NativeStreamObservation,
    },
    latency::{InputDeliverySnapshot, NativeStreamTiming, render::RenderPaddingSnapshot},
};
use std::sync::Arc;

type ExpectedRows = [(&'static str, &'static str); 19];

// Literal expectations deliberately do not call the presentation function.
const LIVE_ROWS: ExpectedRows = [
    (
        "1 Physical input: actual timing",
        "48000 Hz; 240 frames (5.00 ms); capacity 960 frames",
    ),
    (
        "2 Master output: actual timing",
        "48000 Hz; 240 frames (5.00 ms); capacity 960 frames",
    ),
    ("Stream 1: live queue / clock", "960 frames; +0.0 ppm"),
    (
        "Stream 1: faults / recovery",
        "Underrun 0 / dropped 0 frames; resets 0 / backlog recoveries 0",
    ),
    (
        "Stream 1: native delivery packets",
        "9007199254740993 packets",
    ),
    ("Stream 1: maximum native packet", "480 frames"),
    ("Stream 1: maximum callback burst", "960 frames"),
    (
        "Stream 1: native arrival gaps",
        "2 observations; max 100000 x 100 ns",
    ),
    (
        "Stream 1: delivery bound violations",
        "Packet 1 / gap 2 observations",
    ),
    (
        "Stream 1: delivery clock observations",
        "Unavailable 3 / regressed 4 observations",
    ),
    (
        "Stream 1: oldest pending frame age estimate",
        "95000 x 100 ns (owner-local estimate, not physical E2E)",
    ),
    (
        "Stream 1: oldest frame correlation",
        "Generation 3; packet position 960 frames; offset 24 / accepted 480 frames",
    ),
    (
        "Stream 1: age QPC correlation",
        "Packet 1000000 / frame 1005000 / observed 1100000 x 100 ns",
    ),
    (
        "Stream 1: age rate basis",
        "48000 Hz; qualified Capture/QPC correction -17 ppm",
    ),
    (
        "Stream 2: native render padding",
        "0 / capacity 960 frames (native queue only, not E2E)",
    ),
    ("Stream 2: observed render rate", "48000 Hz"),
    ("Stream 2: observed render period", "240 frames"),
    ("Stream 2: render sample sequence", "12"),
    ("Physical end-to-end latency", "Unknown / unqualified"),
];

fn live_snapshot(mut snapshot: AppSnapshot) -> AppSnapshot {
    snapshot.phase = SessionPhase::Running;
    snapshot.generation = SessionGeneration(8);
    let nodes = snapshot.graph.graph.nodes();
    let input = nodes[0].id();
    let output = nodes[1].id();
    let stream = |node, kind| NativeStreamObservation {
        node,
        kind,
        timing: Some(NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(240),
            capacity_frames: 960,
        }),
        shared_period: None,
        render_queue: None,
        input_queue: None,
        virtual_budget: None,
    };
    snapshot.native_observations = Some(NativeSessionObservations {
        generation: SessionGeneration(8),
        observation: NativeObservationSnapshot {
            process_delivery: Vec::new(),
            owner_generation: 3,
            running: true,
            streams: vec![
                stream(input, NativeStreamKind::PhysicalInput),
                stream(output, NativeStreamKind::MasterOutput),
            ],
            boundary_probes: Vec::new(),
            bridges: vec![NativeBridgeObservation {
                node: input,
                queue_history: None,
                counters: BridgeSnapshot {
                    // These deliberately disagree with age: neither is its provenance.
                    fill_frames: 960,
                    last_qpc_100ns: 777,
                    native_delivery: Some(InputDeliverySnapshot {
                        packets: (1 << 53) + 1,
                        max_packet_frames: 480,
                        max_callback_burst_frames: 960,
                        gap_samples: 2,
                        max_gap_100ns: 100_000,
                        packet_bound_violations: 1,
                        gap_bound_violations: 2,
                        unavailable_clocks: 3,
                        regressed_clocks: 4,
                    }),
                    oldest_frame_age: Some(QueuedFrameAge {
                        stamp: QueuedFrameStamp {
                            generation: 3,
                            packet_position_frames: 960,
                            packet_qpc_100ns: 1_000_000,
                            packet_accepted_frames: 480,
                            packet_offset_frames: 24,
                            rate_correction_ppm: Some(-17),
                        },
                        observed_qpc_100ns: 1_100_000,
                        input_sample_rate: 48_000,
                        estimated_frame_qpc_100ns: 1_005_000,
                        estimated_age_100ns: 95_000,
                    }),
                    ..Default::default()
                },
            }],
            render_padding: vec![NativeRenderPaddingObservation {
                node: output,
                sample: Some(RenderPaddingSnapshot {
                    padding_frames: 0,
                    capacity_frames: 960,
                    sample_rate: Some(48_000),
                    period_frames: Some(240),
                    sequence: 12,
                }),
            }],
        },
    });
    snapshot
}

fn with_native_diagnostics(
    cx: &mut TestAppContext,
    test: impl FnOnce(&mut Window, &mut App, &Entity<UiState>, AppSnapshot),
) {
    let runtime = common::service();
    let snapshot = live_snapshot((*common::wait_snapshot(&runtime.handle())).clone());
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
        window.click("open-settings", cx);
        window.click("settings-diagnostics", cx);
        assert!(window.find("settings-dialog").visible());
        assert!(
            !window.find(("native-timing-row", 18_usize)).visible(),
            "compact acceptance must exercise clipped rows, not an oversized window"
        );
        test(window, cx, &state, snapshot);
    })
    .unwrap();
    common::finish(runtime);
}

fn replace_snapshot(
    window: &mut Window,
    cx: &mut App,
    state: &Entity<UiState>,
    snapshot: &AppSnapshot,
) {
    state.update(cx, |state, cx| {
        state.snapshot = Arc::new(snapshot.clone());
        cx.notify();
    });
    window.render_frame(cx);
}

fn assert_row(window: &mut Window, cx: &mut App, id: impl Into<gpui_kit::ElementId>, text: &str) {
    let id = id.into();
    // Keep the existing acceptance scroll budget; never wait for device data.
    for _ in 0..80 {
        let row = window.find(id.clone());
        let bounds = row.bounds();
        let viewport = window.find("settings-scroll").bounds();
        if row.visible()
            && bounds.origin.y >= viewport.origin.y
            && bounds.origin.y + bounds.size.height <= viewport.origin.y + viewport.size.height
        {
            assert_eq!(row.label(), Some(text));
            return;
        }
        window.scroll(
            "settings-scroll",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-180.))),
            cx,
        );
    }
    panic!("{text} must be fully reachable by scrolling");
}

fn assert_rows(window: &mut Window, cx: &mut App, rows: &[(&str, &str)]) {
    window.scroll(
        "settings-scroll",
        gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(10_000.))),
        cx,
    );
    for (index, (label, value)) in rows.iter().enumerate() {
        assert_row(
            window,
            cx,
            ("native-timing-row", index),
            &format!("{label}: {value}"),
        );
    }
    assert!(window.try_find(("native-timing-row", rows.len())).is_none());
    assert_row(
        window,
        cx,
        "diagnostics-physical-latency",
        "Physical end-to-end latency: Unknown / unqualified",
    );
}

#[gpui_kit::test]
fn native_live_rendered_acceptance_qualified_age_and_observed_zero_padding(
    cx: &mut TestAppContext,
) {
    with_native_diagnostics(cx, |window, cx, _, _| {
        assert_rows(window, cx, &LIVE_ROWS);
    });
}

#[gpui_kit::test]
fn native_live_rendered_acceptance_missing_and_unobserved_timing(cx: &mut TestAppContext) {
    with_native_diagnostics(cx, |window, cx, state, mut snapshot| {
        assert_rows(window, cx, &LIVE_ROWS);
        let observation = &mut snapshot.native_observations.as_mut().unwrap().observation;
        observation.bridges[0].counters.native_delivery = None;
        observation.bridges[0].counters.oldest_frame_age = None;
        observation.render_padding[0].sample = None;
        let mut expected = LIVE_ROWS;
        for row in &mut expected[4..18] {
            row.1 = "Unknown";
        }
        replace_snapshot(window, cx, state, &snapshot);
        assert_rows(window, cx, &expected);

        // Retained maxima do not establish timing when no packets were observed.
        snapshot
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .bridges[0]
            .counters
            .native_delivery = Some(InputDeliverySnapshot {
            max_packet_frames: 128,
            max_callback_burst_frames: 256,
            gap_samples: 3,
            max_gap_100ns: 123,
            ..Default::default()
        });
        expected[4].1 = "0 packets";
        expected[7].1 = "3 observations; timing unknown";
        expected[8].1 = "Packet 0 / gap 0 observations";
        expected[9].1 = "Unavailable 0 / regressed 0 observations";
        replace_snapshot(window, cx, state, &snapshot);
        assert_rows(window, cx, &expected);

        snapshot
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .bridges[0]
            .counters
            .native_delivery = Some(InputDeliverySnapshot {
            packets: 1,
            max_gap_100ns: 999,
            ..Default::default()
        });
        expected[4].1 = "1 packets";
        expected[7].1 = "0 observations; timing unknown";
        replace_snapshot(window, cx, state, &snapshot);
        assert_rows(window, cx, &expected);

        snapshot.native_observations = None;
        replace_snapshot(window, cx, state, &snapshot);
        assert_rows(window, cx, &[("Actual native timing", "\u{2014}")]);
    });
}

#[gpui_kit::test]
fn native_live_rendered_acceptance_fencing_and_fresh_replacement(cx: &mut TestAppContext) {
    with_native_diagnostics(cx, |window, cx, state, mut snapshot| {
        assert_rows(window, cx, &LIVE_ROWS);
        for phase in [
            SessionPhase::Idle,
            SessionPhase::Starting,
            SessionPhase::Stopping,
            SessionPhase::Failed,
            SessionPhase::Exiting,
            SessionPhase::Exited,
        ] {
            snapshot.phase = phase;
            replace_snapshot(window, cx, state, &snapshot);
            assert_rows(window, cx, &[("Actual native timing", "\u{2014}")]);
        }
        snapshot.phase = SessionPhase::Running;
        snapshot
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .running = false;
        replace_snapshot(window, cx, state, &snapshot);
        assert_rows(window, cx, &[("Actual native timing", "\u{2014}")]);

        snapshot
            .native_observations
            .as_mut()
            .unwrap()
            .observation
            .running = true;
        snapshot.generation = SessionGeneration(9);
        replace_snapshot(window, cx, state, &snapshot);
        assert_rows(window, cx, &[("Actual native timing", "\u{2014}")]);

        let status = snapshot.native_observations.as_mut().unwrap();
        status.generation = SessionGeneration(9);
        status.observation.owner_generation = 4;
        let counters = &mut status.observation.bridges[0].counters;
        counters.fill_frames = 123;
        counters.native_delivery = Some(InputDeliverySnapshot {
            packets: 7,
            max_packet_frames: 240,
            max_callback_burst_frames: 480,
            gap_samples: 1,
            max_gap_100ns: 0,
            ..Default::default()
        });
        counters.oldest_frame_age = Some(QueuedFrameAge {
            stamp: QueuedFrameStamp {
                generation: 4,
                packet_position_frames: 48,
                packet_qpc_100ns: 2_000_000,
                packet_accepted_frames: 240,
                packet_offset_frames: 0,
                rate_correction_ppm: None,
            },
            observed_qpc_100ns: 2_000_000,
            input_sample_rate: 44_100,
            estimated_frame_qpc_100ns: 2_000_000,
            estimated_age_100ns: 0,
        });
        status.observation.render_padding[0].sample = Some(RenderPaddingSnapshot {
            padding_frames: 336,
            capacity_frames: 960,
            sample_rate: None,
            period_frames: None,
            sequence: 1,
        });
        let mut expected = LIVE_ROWS;
        expected[2].1 = "123 frames; +0.0 ppm";
        expected[4].1 = "7 packets";
        expected[5].1 = "240 frames";
        expected[6].1 = "480 frames";
        expected[7].1 = "1 observations; max 0 x 100 ns";
        expected[8].1 = "Packet 0 / gap 0 observations";
        expected[9].1 = "Unavailable 0 / regressed 0 observations";
        expected[10].1 = "0 x 100 ns (owner-local estimate, not physical E2E)";
        expected[11].1 = "Generation 4; packet position 48 frames; offset 0 / accepted 240 frames";
        expected[12].1 = "Packet 2000000 / frame 2000000 / observed 2000000 x 100 ns";
        expected[13].1 = "44100 Hz nominal rate (Capture/QPC correction unqualified)";
        expected[14].1 = "336 / capacity 960 frames (native queue only, not E2E)";
        expected[15].1 = "Unknown";
        expected[16].1 = "Unknown";
        expected[17].1 = "1";
        replace_snapshot(window, cx, state, &snapshot);
        assert_rows(window, cx, &expected);
    });
}
