use crate::{
    service::{
        AppSnapshot, NativeSessionObservations, ServiceContext, SessionGeneration, SessionPhase,
    },
    ui::presentation::native_timing_rows,
};
use moiren_windows_audio::{
    engine_backend::{NativeObservationSnapshot, NativeStreamKind, NativeStreamObservation},
    virtual_owner::BoundaryProbeSample,
};

fn boundary_snapshot(target_frames: Option<u32>) -> AppSnapshot {
    let mut snapshot = crate::service::snapshot(&ServiceContext::default());
    snapshot.phase = SessionPhase::Running;
    snapshot.generation = SessionGeneration(8);
    let nodes = snapshot.graph.graph.nodes();
    let node = nodes[0].id();
    let other = nodes[1].id();
    let stream = |node, kind| NativeStreamObservation {
        node,
        kind,
        timing: None,
        shared_period: None,
        render_queue: None,
        input_queue: None,
        virtual_budget: None,
    };
    snapshot.native_observations = Some(NativeSessionObservations {
        generation: SessionGeneration(8),
        observation: NativeObservationSnapshot {
            virtual_clock: None,
            process_delivery: Vec::new(),
            render_padding: Vec::new(),
            owner_generation: 3,
            running: true,
            // Both node and kind must match; neither alone identifies a probe.
            streams: vec![
                stream(node, NativeStreamKind::VirtualMicrophone),
                stream(node, NativeStreamKind::VirtualPlayback),
                stream(other, NativeStreamKind::VirtualPlayback),
            ],
            bridges: Vec::new(),
            boundary_probes: vec![
                (
                    node,
                    NativeStreamKind::VirtualPlayback,
                    BoundaryProbeSample {
                        capacity_frames: 8192,
                        target_frames,
                        level_frames: 777,
                        applied_milli_ppm: -12_345,
                        xruns: 4_294_967_307,
                        soft_events: 4_294_967_319,
                        playback_events: None,
                    },
                )
                    .into(),
            ],
        },
    });
    snapshot
}

#[test]
fn native_boundary_presentation_shows_geometry_occupancy_correction_and_event_counts() {
    let rows = native_timing_rows(&boundary_snapshot(Some(1536)));
    for (label, value) in [
        (
            "2 Virtual playback: prepared boundary capacity",
            "8192 native frames at 48 kHz (storage headroom)",
        ),
        (
            "2 Virtual playback: total pre-transfer content target",
            "1536 native frames at 48 kHz",
        ),
        (
            "2 Virtual playback: post-transfer occupancy / correction",
            "777 native frames at 48 kHz; applied -12.345 ppm",
        ),
        (
            "2 Virtual playback: cumulative boundary events",
            "Xrun 4294967307 / soft 4294967319 events (not driver drops or silent frames)",
        ),
    ] {
        assert!(
            rows.contains(&(label.into(), value.into())),
            "missing boundary row {label}: {value}; got {rows:?}"
        );
    }
}

#[test]
fn native_boundary_presentation_keeps_unknown_target() {
    let rows = native_timing_rows(&boundary_snapshot(None));
    assert!(rows.contains(&(
        "2 Virtual playback: total pre-transfer content target".into(),
        "Unknown".into(),
    )));
    assert!(rows.contains(&(
        "2 Virtual playback: prepared boundary capacity".into(),
        "8192 native frames at 48 kHz (storage headroom)".into(),
    )));
}

#[test]
fn native_boundary_presentation_hides_stopped_and_stale_observations() {
    let fresh = boundary_snapshot(Some(1536));
    assert!(
        native_timing_rows(&fresh)
            .iter()
            .any(|(label, _)| label.contains("boundary"))
    );
    let unavailable = vec![("Actual native timing".into(), "\u{2014}".into())];
    for phase in [
        SessionPhase::Idle,
        SessionPhase::Starting,
        SessionPhase::Stopping,
        SessionPhase::Failed,
        SessionPhase::Exiting,
        SessionPhase::Exited,
    ] {
        let mut snapshot = fresh.clone();
        snapshot.phase = phase;
        assert_eq!(native_timing_rows(&snapshot), unavailable, "{phase:?}");
    }
    let mut stale = fresh.clone();
    stale.generation = SessionGeneration(9);
    assert_eq!(native_timing_rows(&stale), unavailable);
    let mut stopped = fresh;
    stopped
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .running = false;
    assert_eq!(native_timing_rows(&stopped), unavailable);
}

#[test]
fn native_boundary_presentation_labels_unmapped_node_or_kind_explicitly() {
    for keep_wrong_node in [false, true] {
        let mut snapshot = boundary_snapshot(Some(1536));
        let observation = &mut snapshot.native_observations.as_mut().unwrap().observation;
        let kept = observation.streams[if keep_wrong_node { 2 } else { 0 }].clone();
        observation.streams = vec![kept];
        assert!(native_timing_rows(&snapshot).contains(&(
            "Unmapped stream Virtual playback: prepared boundary capacity".into(),
            "8192 native frames at 48 kHz (storage headroom)".into(),
        )));
    }
    let mut snapshot = boundary_snapshot(Some(1536));
    snapshot
        .native_observations
        .as_mut()
        .unwrap()
        .observation
        .streams
        .clear();
    assert!(native_timing_rows(&snapshot).contains(&(
        "Unmapped stream Virtual playback: total pre-transfer content target".into(),
        "1536 native frames at 48 kHz".into(),
    )));
}

#[test]
fn native_boundary_presentation_translates_units_scope_and_interpolated_values() {
    let locale = crate::i18n::SIMPLIFIED_CHINESE;
    for (key, expected) in [
        (
            "%{prefix}: prepared boundary capacity",
            "虚拟回放：已准备的边界容量",
        ),
        (
            "%{prefix}: total pre-transfer content target",
            "虚拟回放：传输前总内容目标",
        ),
        (
            "%{prefix}: post-transfer occupancy / correction",
            "虚拟回放：传输后占用量 / 校正",
        ),
        (
            "%{prefix}: cumulative boundary events",
            "虚拟回放：累计边界事件",
        ),
    ] {
        assert_eq!(
            rust_i18n::t!(key, locale = locale, prefix = "虚拟回放"),
            expected
        );
    }
    assert_eq!(
        rust_i18n::t!(
            "%{frames} native frames at 48 kHz (storage headroom)",
            locale = locale,
            frames = 8192,
        ),
        "8192 个 48 kHz 原生帧（存储余量）"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{frames} native frames at 48 kHz",
            locale = locale,
            frames = 1536
        ),
        "1536 个 48 kHz 原生帧"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{frames} native frames at 48 kHz; applied %{ppm} ppm",
            locale = locale,
            frames = 777,
            ppm = "-12.345",
        ),
        "777 个 48 kHz 原生帧；已应用 -12.345 ppm"
    );
    assert_eq!(
        rust_i18n::t!(
            "Xrun %{xruns} / soft %{soft} events (not driver drops or silent frames)",
            locale = locale,
            xruns = 47,
            soft = 91,
        ),
        "Xrun 47 / 软事件 91 次（非驱动丢帧数或静音帧数）"
    );
    assert_eq!(rust_i18n::t!("Unknown", locale = locale), "未知");
}
