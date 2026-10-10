//! Statistics-only output: never persist PCM, endpoint IDs, paths or process IDs.
use moiren_app::host::{
    HostEvent,
    windows::{SessionReport, SessionSnapshot},
};
use serde::Serialize;
use serde_json::{Value, json};

/// Latch each observed generation before replacement can reuse its source ID.
/// Maxima are diagnostic bounds, not sums of repeatedly sampled counters.
#[derive(Default, Serialize)]
pub(super) struct BridgeHealth {
    pub max_live_underrun_frames: u64,
    pub max_finished_underrun_frames: u64,
}
impl BridgeHealth {
    pub fn observe(&mut self, snapshot: &SessionSnapshot) {
        for source in &snapshot.sources {
            self.max_live_underrun_frames = self
                .max_live_underrun_frames
                .max(source.bridge.live_underrun_frames);
            self.max_finished_underrun_frames = self.max_finished_underrun_frames.max(
                source
                    .bridge
                    .underrun_frames
                    .saturating_sub(source.bridge.live_underrun_frames),
            );
        }
    }
}

pub(super) fn snapshot(label: &str, elapsed: f64, snapshot: &SessionSnapshot) -> Value {
    let mut health = BridgeHealth::default();
    health.observe(snapshot);
    json!({"label":label,"elapsed_seconds":elapsed,"status":snapshot.status,
        "bridge_health":health,
        "runtime":snapshot.runtime,"failure":snapshot.failure,
        "sources":snapshot.sources.iter().map(|source|json!({"id":source.id,"status":source.status,
            "available":source.available,"enabled":source.enabled,"bridge":source.bridge,
            "capture":source.report.as_ref().map(|report|json!({
                "source":report["source"],"status":report["status"],"sample_rate":report["sample_rate"],
                "channels":report["channels"],"buffer_frames":report["buffer_frames"],
                "packets":report["packets"],"elapsed_seconds":report["elapsed_seconds"],
                "audio_wakes":report["audio_wakes"],"timeout_wakes":report["timeout_wakes"]})),
            "failure":source.failure,"last_start_failure":source.last_start_failure})).collect::<Vec<_>>()})
}

pub(super) fn events(events: Vec<HostEvent>) -> Vec<Value> {
    events
        .into_iter()
        .map(moiren_app::host_cli::event_json)
        .collect()
}

pub(super) fn applied_receipt(event: &Value) -> Option<(u64, bool)> {
    if event["type"] != "parameter" {
        return None;
    }
    let late = match event["reply"]["code"].as_str()? {
        "Applied" => false,
        "AppliedLate" => true,
        _ => return None,
    };
    Some((event["reply"]["request_id"].as_u64()?, late))
}

pub(super) fn report(report: &SessionReport) -> Value {
    json!({"snapshot":snapshot("stopped",0.0,&report.snapshot),"events":report.events,
        "render":report.render.as_ref().map(|render|json!({"status":render.status,"elapsed_seconds":render.elapsed_seconds,
            "stream_started":render.stream_started,"stop_succeeded":render.stop_succeeded,
            "native_mix_sample_rate":render.native_mix_sample_rate,"native_mix_channels":render.native_mix_channels,
            "buffer_frames":render.buffer_frames,"stats":render.stats,"failure":render.failure}))})
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_app::host::windows::{
        CaptureSelection, SessionRuntime, SessionStatus, SourceDiagnostic, SourceStatus,
    };
    use moiren_windows_audio::clock_bridge::BridgeSnapshot;

    fn diagnostic(bridge: BridgeSnapshot) -> SessionSnapshot {
        SessionSnapshot {
            schema_version: 1,
            status: SessionStatus::Running,
            output_endpoint_id: "private-output".into(),
            failure: None,
            runtime: SessionRuntime::default(),
            sources: vec![SourceDiagnostic {
                id: 2,
                selection: CaptureSelection::Physical {
                    endpoint_id: "private-input".into(),
                },
                status: SourceStatus::Removed,
                enabled: true,
                available: false,
                bridge,
                failure: None,
                last_start_failure: None,
                report: None,
            }],
        }
    }

    #[test]
    fn retired_source_live_shortfall_remains_separate_from_finished_tail() {
        let data = diagnostic(BridgeSnapshot {
            underrun_frames: 242,
            live_underrun_frames: 128,
            last_underrun_producer_finished: true,
            ..BridgeSnapshot::default()
        });
        let result = snapshot("retired", 18.0, &data);
        assert_eq!(result["bridge_health"]["max_live_underrun_frames"], 128);
        assert_eq!(result["bridge_health"]["max_finished_underrun_frames"], 114);
        assert!(!serde_json::to_string(&result).unwrap().contains("private-"));
    }

    #[test]
    fn replacement_with_same_id_cannot_erase_an_observed_live_shortfall() {
        let mut health = BridgeHealth::default();
        health.observe(&diagnostic(BridgeSnapshot {
            underrun_frames: 128,
            live_underrun_frames: 128,
            ..BridgeSnapshot::default()
        }));
        health.observe(&diagnostic(BridgeSnapshot::default()));
        assert_eq!(health.max_live_underrun_frames, 128);
        assert_eq!(health.max_finished_underrun_frames, 0);
    }

    #[test]
    fn applied_late_receipt_proves_application_and_retains_lateness() {
        use moiren_core::protocol::{ControlReply, ReplyCode};
        for (code, expected) in [
            (ReplyCode::Applied, Some((7, false))),
            (ReplyCode::AppliedLate, Some((7, true))),
            (ReplyCode::Accepted, None),
            (ReplyCode::StaleRevision, None),
        ] {
            let event = events(vec![HostEvent::Parameter(ControlReply {
                request_id: 7,
                plan_revision: 2,
                timeline_epoch: 1,
                code,
                effective_frame: if matches!(code, ReplyCode::Applied | ReplyCode::AppliedLate) {
                    48960
                } else {
                    0
                },
            })])
            .remove(0);
            assert_eq!(applied_receipt(&event), expected, "{code:?}");
        }
    }
}
