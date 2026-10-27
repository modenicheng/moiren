use crate::{
    service::{
        AppSnapshot, AudioLatencyProfile, ExportDiagnostics, ExportDirection, ExportIntent,
        ExportResourceState, ExportStatus, ServiceContext, SessionPhase,
    },
    ui::presentation::export_diagnostic_rows,
};
use moiren_engine::node::VirtualEndpointId;
use moiren_windows_audio::vad::VadPcmTelemetrySnapshot;

fn export() -> ExportStatus {
    ExportStatus {
        intent: ExportIntent {
            endpoint_id: VirtualEndpointId::from_bytes([7; 16]),
            direction: ExportDirection::Playback,
            name: "Requested playback".into(),
            channels: 2,
            enabled: true,
            binding: None,
        },
        actual_name: Some("Observed playback".into()),
        rename_pending: true,
        resource: ExportResourceState::Present,
        allow_default: false,
        gate_open: false,
        active_revision: None,
        diagnostics: Some(ExportDiagnostics {
            generation: 42,
            native_stamp: Some([1, 2, 3, 0, 5]),
            transport_prepared: true,
            telemetry: Some(VadPcmTelemetrySnapshot {
                schema_version: 2,
                packet_frames: 128,
                max_packet_frames: 256,
                submission_attempts: u64::MAX,
                submitted_packets: (1 << 53) + 1,
                completed_packets: (1 << 32) + 3,
                completion_interval_observations: 7,
                max_completion_interval_100ns: 123_456,
                completion_clock_regressions: 11,
                dropped_packets: 13,
                driver_dropped_packets: 17,
                quota_dropped_packets: 19,
                carrier_dropped_packets: 23,
                stale_packets: 29,
                silence_packets: 31,
                silence_frames: (1 << 53) + 37,
                epoch_resets: 41,
                recovery_events: 43,
            }),
            error: Some("PCM preparation refused: packet quota 0x80070057".into()),
            windows_name: None,
        }),
        #[cfg(feature = "vad-integration-probe")]
        native_stamp: None,
        #[cfg(feature = "vad-integration-probe")]
        native_retirement: None,
        windows_name: None,
    }
}

fn value<'a>(rows: &'a [(String, String)], key: &str) -> &'a str {
    rows.iter()
        .find(|(label, _)| label == key)
        .unwrap_or_else(|| panic!("missing {key:?} in {rows:?}"))
        .1
        .as_str()
}

#[test]
fn provider_created_name_is_not_presented_as_windows_name_evidence() {
    let mut status = export();
    status.actual_name = Some(status.intent.name.clone());
    status.rename_pending = false;
    let rows = export_diagnostic_rows(&status);
    assert_eq!(
        value(&rows, "Provider-created name"),
        value(&rows, "Requested endpoint name")
    );
    assert_eq!(
        value(&rows, "Windows endpoint name"),
        "Unknown / not observed"
    );
    assert_eq!(value(&rows, "Windows name match"), "Unknown / not observed");
    assert!(!rows.iter().any(|(key, _)| key == "Rename"));
}

#[test]
fn windows_name_mismatch_and_read_error_are_visible() {
    let mut status = export();
    status.actual_name = Some(status.intent.name.clone());
    status.rename_pending = true;
    status.windows_name = Some(crate::service::WindowsEndpointNameObservation {
        native_identity: [1, 2, 3],
        observed_qpc_100ns: Some(1234),
        mmdevice_id: Some("{observed-device}".into()),
        name: Some("Stale Windows name".into()),
        error: None,
    });
    let rows = export_diagnostic_rows(&status);
    assert_eq!(value(&rows, "Provider-created name"), "Requested playback");
    assert_eq!(value(&rows, "Windows endpoint name"), "Stale Windows name");
    status.rename_pending = false;
    let rows = export_diagnostic_rows(&status);
    assert_eq!(
        value(&rows, "Windows name match"),
        "Does not match requested name"
    );
    assert_eq!(
        value(&rows, "Windows name MMDevice ID"),
        "{observed-device}"
    );
    assert_eq!(
        value(&rows, "Windows name observation QPC (100 ns)"),
        "1234"
    );

    status.windows_name = Some(crate::service::WindowsEndpointNameObservation {
        native_identity: [1, 2, 3],
        observed_qpc_100ns: Some(5678),
        mmdevice_id: Some("{failed-device}".into()),
        name: None,
        error: Some("GetValue FriendlyName: 0x80070005".into()),
    });
    let rows = export_diagnostic_rows(&status);
    assert_eq!(
        value(&rows, "Windows endpoint name"),
        "Unknown / not observed"
    );
    assert_eq!(value(&rows, "Windows name match"), "Read failed");
    assert_eq!(
        value(&rows, "Windows name readback error"),
        "GetValue FriendlyName: 0x80070005"
    );
}

#[test]
fn provider_diagnostics_preserve_all_counters_and_observed_timing_units() {
    let rows = export_diagnostic_rows(&export());
    for (key, expected) in [
        ("Observed packet", "128 frames"),
        ("Maximum observed packet", "256 frames"),
        (
            "Completion interval observations",
            "7 observations; max 123456 x 100 ns",
        ),
        (
            "Cumulative submissions",
            "18446744073709551615 attempts / 9007199254740993 submitted packets",
        ),
        ("Cumulative completions", "4294967299 packets"),
        (
            "Cumulative packet drops",
            "Driver 17 / quota 19 / carrier 23 / total 13 packets",
        ),
        ("Cumulative stale packets", "29 packets"),
        ("Cumulative silence", "31 packets / 9007199254741029 frames"),
        ("Cumulative epoch resets", "41"),
        ("Cumulative recoveries", "43"),
        ("Cumulative completion clock regressions", "11"),
    ] {
        assert_eq!(value(&rows, key), expected, "{key}");
    }
}

#[test]
fn provider_diagnostics_identify_requested_actual_and_pending_names_with_exact_endpoint_error() {
    let status = export();
    let rows = export_diagnostic_rows(&status);
    assert_eq!(
        value(&rows, "Stable endpoint ID"),
        status.intent.endpoint_id.to_string()
    );
    assert_eq!(value(&rows, "Export direction"), "Playback");
    assert_eq!(
        value(&rows, "Requested endpoint name"),
        "Requested playback"
    );
    assert_eq!(value(&rows, "Provider-created name"), "Observed playback");
    assert_eq!(
        value(&rows, "Windows endpoint name"),
        "Unknown / not observed"
    );
    assert_eq!(
        value(&rows, "Windows name match"),
        "Pending native recreation"
    );
    assert_eq!(
        value(&rows, "Rename"),
        "Name pending; waiting for native recreation"
    );
    assert_eq!(value(&rows, "Endpoint"), "Present (not a flow signal)");
    assert_eq!(
        value(&rows, "Endpoint error"),
        "PCM preparation refused: packet quota 0x80070057"
    );
}

#[test]
fn provider_diagnostics_zero_packet_and_unobserved_intervals_are_unknown_timing() {
    let mut status = export();
    let telemetry = status
        .diagnostics
        .as_mut()
        .unwrap()
        .telemetry
        .as_mut()
        .unwrap();
    telemetry.packet_frames = 0;
    telemetry.max_packet_frames = 0;
    telemetry.completion_interval_observations = 0;
    // No interval samples means no timing evidence, even with a retained maximum.
    telemetry.max_completion_interval_100ns = 123_456;
    let rows = export_diagnostic_rows(&status);
    assert_eq!(value(&rows, "Observed packet"), "Unknown");
    assert_eq!(value(&rows, "Maximum observed packet"), "Unknown");
    assert_eq!(
        value(&rows, "Completion interval observations"),
        "0 observations; timing unknown"
    );
    assert_eq!(value(&rows, "Cumulative completions"), "4294967299 packets");
}

#[test]
fn provider_diagnostics_missing_telemetry_keeps_native_preparation_and_refusal() {
    let mut status = export();
    let diagnostic = status.diagnostics.as_mut().unwrap();
    diagnostic.telemetry = None;
    diagnostic.transport_prepared = false;
    let rows = export_diagnostic_rows(&status);
    assert_eq!(value(&rows, "Provider packet telemetry"), "Unavailable");
    assert_eq!(value(&rows, "Native transport preparation"), "Not prepared");
    assert_eq!(
        value(&rows, "Endpoint error"),
        "PCM preparation refused: packet quota 0x80070057"
    );
    assert!(!rows.iter().any(|(key, _)| key.starts_with("Cumulative")));
    assert!(!rows.iter().any(|(key, _)| key == "Observed packet"));
}

#[test]
fn provider_diagnostics_missing_evidence_never_fabricates_zero_or_preparation() {
    let mut status = export();
    status.diagnostics = None;
    status.actual_name = None;
    let rows = export_diagnostic_rows(&status);
    for key in [
        "Native export diagnostics",
        "Native transport preparation",
        "Provider packet telemetry",
        "Endpoint error",
    ] {
        assert_eq!(value(&rows, key), "Unavailable", "{key}");
    }
    assert_eq!(
        value(&rows, "Windows endpoint name"),
        "Unknown / not observed"
    );
    assert!(!rows.iter().any(|(key, _)| key.starts_with("Cumulative")));
    assert!(
        !rows
            .iter()
            .any(|(_, text)| text == "0" || text.starts_with("Prepared"))
    );
}

#[test]
fn provider_diagnostics_remain_last_observations_while_engine_stops_or_endpoint_retires() {
    let mut snapshot: AppSnapshot = crate::service::snapshot(&ServiceContext::default());
    snapshot.exports = vec![export()];
    let requested_profile = snapshot.latency_profile;
    let requested_transport = snapshot.transport_latency;
    for phase in [
        SessionPhase::Running,
        SessionPhase::Idle,
        SessionPhase::Stopping,
        SessionPhase::Failed,
        SessionPhase::Exited,
    ] {
        for resource in [ExportResourceState::Present, ExportResourceState::Removing] {
            snapshot.phase = phase;
            snapshot.exports[0].resource = resource;
            let rows = export_diagnostic_rows(&snapshot.exports[0]);
            assert_eq!(
                value(&rows, "Diagnostic scope"),
                "Latest native export observation (not live audio)"
            );
            assert_eq!(value(&rows, "Observation generation"), "42");
            assert_eq!(
                value(&rows, "Native transport preparation"),
                "Prepared (native observation only)"
            );
            assert_eq!(value(&rows, "Gate admission"), "Closed");
            assert_eq!(value(&rows, "Engine-confirmed binding"), "Inactive");
            assert_eq!(value(&rows, "Observed packet"), "128 frames");
            assert_eq!(
                value(&rows, "Endpoint"),
                if resource == ExportResourceState::Removing {
                    "Removing / awaiting confirmation"
                } else {
                    "Present (not a flow signal)"
                }
            );
        }
    }
    assert_eq!(requested_profile, AudioLatencyProfile::Compatible);
    assert_eq!(snapshot.latency_profile, requested_profile);
    assert_eq!(snapshot.transport_latency, requested_transport);
    assert_eq!(snapshot.active_latency_profile, None);
}

#[test]
fn provider_diagnostics_use_only_service_confirmed_gate_and_revision() {
    let mut status = export();
    status.gate_open = true;
    status.active_revision = Some(u64::MAX);
    // Native preparation and error never supply the Engine authority rows.
    status.diagnostics.as_mut().unwrap().transport_prepared = false;
    let rows = export_diagnostic_rows(&status);
    assert_eq!(value(&rows, "Gate admission"), "Open (not a flow signal)");
    assert_eq!(
        value(&rows, "Engine-confirmed binding"),
        "Active revision 18446744073709551615"
    );
    assert_eq!(value(&rows, "Native transport preparation"), "Not prepared");
}

#[test]
fn provider_diagnostics_pending_rename_recreation_failure_retains_stable_identity() {
    let mut status = export();
    status.resource = ExportResourceState::Absent;
    status.diagnostics.as_mut().unwrap().transport_prepared = false;
    status.diagnostics.as_mut().unwrap().error =
        Some("native recreation failed: endpoint name rejected".into());
    let rows = export_diagnostic_rows(&status);
    assert_eq!(
        value(&rows, "Stable endpoint ID"),
        status.intent.endpoint_id.to_string()
    );
    assert_eq!(
        value(&rows, "Requested endpoint name"),
        "Requested playback"
    );
    assert_eq!(value(&rows, "Provider-created name"), "Observed playback");
    assert_eq!(
        value(&rows, "Windows endpoint name"),
        "Unknown / not observed"
    );
    assert_eq!(
        value(&rows, "Windows name match"),
        "Pending native recreation"
    );
    assert_eq!(
        value(&rows, "Rename"),
        "Name pending; waiting for native recreation"
    );
    assert_eq!(value(&rows, "Endpoint"), "Absent");
    assert_eq!(value(&rows, "Native transport preparation"), "Not prepared");
    assert_eq!(value(&rows, "Engine-confirmed binding"), "Inactive");
    assert_eq!(
        value(&rows, "Endpoint error"),
        "native recreation failed: endpoint name rejected"
    );
}

#[test]
fn provider_diagnostics_translate_scope_units_counters_and_unknown_states() {
    let locale = crate::i18n::SIMPLIFIED_CHINESE;
    for (key, expected) in [
        ("Provider export diagnostics", "提供方导出诊断"),
        ("Diagnostic scope", "诊断范围"),
        (
            "Latest native export observation (not live audio)",
            "最近一次原生导出观测（非实时音频）",
        ),
        ("Native transport preparation", "原生传输准备状态"),
        (
            "Prepared (native observation only)",
            "已准备（仅为原生观测）",
        ),
        ("Not prepared", "未准备"),
        ("Provider packet telemetry", "提供方数据包遥测"),
        ("Provider-created name", "提供方创建名称"),
        ("Windows endpoint name", "Windows 端点名称"),
        ("Windows name match", "Windows 名称匹配状态"),
        ("Unknown / not observed", "未知 / 尚未观测"),
        ("Pending native recreation", "等待原生端点重建"),
        ("Matches requested name", "与请求名称一致"),
        ("Does not match requested name", "与请求名称不一致"),
        ("Endpoint error", "端点错误"),
        ("Unknown / unqualified", "未知 / 未经验证"),
        ("CPU load", "CPU 负载"),
        ("Physical end-to-end latency", "物理端到端延迟"),
        ("0 observations; timing unknown", "0 次观测；时序未知"),
    ] {
        assert_eq!(rust_i18n::t!(key, locale = locale), expected, "{key}");
    }
    assert_eq!(
        rust_i18n::t!("%{count} packets", locale = locale, count = u64::MAX),
        "18446744073709551615 个数据包"
    );
    assert_eq!(
        rust_i18n::t!(
            "%{count} observations; max %{ticks} x 100 ns",
            locale = locale,
            count = 7,
            ticks = 123_456,
        ),
        "7 次观测；最大间隔 123456 x 100 ns"
    );
    assert_eq!(
        rust_i18n::t!(
            "Driver %{driver} / quota %{quota} / carrier %{carrier} / total %{total} packets",
            locale = locale,
            driver = 17,
            quota = 19,
            carrier = 23,
            total = u64::MAX,
        ),
        "驱动 17 / 配额 19 / 载体 23 / 合计 18446744073709551615 个数据包"
    );
}
