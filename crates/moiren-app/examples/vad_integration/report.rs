use moiren_app::service::{ExportDirection, ExportStatus};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Checkpoint {
    pub stage: String,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Report {
    pub schema_version: u32,
    pub stage: &'static str,
    pub status: &'static str,
    pub elapsed_ms: u128,
    pub persisted_project_verified: bool,
    pub native_clients_started_by_probe: u32,
    pub pcm_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename: Option<Vec<super::rename::Case>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_fixture_seed: Option<u64>,
    pub idle_hold: Option<IdleHoldEvidence>,
    pub pcm: Option<PcmEvidence>,
    pub soak_idle: Option<super::soak::IdleSoak>,
    pub soak_pcm: Option<super::soak::PcmSoak>,
    pub soak_music: Option<super::soak::MusicSoak>,
    pub probe_trail: Vec<ProbeTrail>,
    pub native_diagnostics: Option<super::soak::NativeDiagnostics>,
    pub current_phase: String,
    pub last_snapshot_observed_ms: Option<u128>,
    pub gate_transitions: Vec<super::validation::GateProgress>,
    pub service_joined: bool,
    pub control_session_released: bool,
    pub endpoint_absence_confirmed: bool,
    pub checkpoints: Vec<Checkpoint>,
    pub error: Option<String>,
    pub cleanup_error: Option<String>,
    pub limitations: Vec<&'static str>,
}

/// One control-side sample of the virtual boundary stream counters, taken
/// from the professional observation snapshot during a soak. The schema
/// stays portable even when the native coordinator is not compiled.
#[derive(Clone, Debug, Serialize)]
pub(super) struct ProbeTrail {
    pub elapsed_ms: u64,
    pub playback_dropped_frames: Option<u64>,
    pub microphone_silent_frames: Option<u64>,
    pub playback_native_callbacks: Option<u64>,
    pub microphone_native_callbacks: Option<u64>,
}

impl Default for Report {
    fn default() -> Self {
        Self {
            schema_version: 1,
            stage: "stage1_service_lifecycle_no_probe_clients",
            status: "failed",
            elapsed_ms: 0,
            persisted_project_verified: false,
            native_clients_started_by_probe: 0,
            pcm_verified: false,
            rename: None,
            rename_fixture_seed: None,
            idle_hold: None,
            pcm: None,
            soak_idle: None,
            soak_pcm: None,
            soak_music: None,
            probe_trail: Vec::new(),
            native_diagnostics: None,
            current_phase: "preparing".into(),
            last_snapshot_observed_ms: None,
            gate_transitions: Vec::new(),
            service_joined: false,
            control_session_released: false,
            endpoint_absence_confirmed: false,
            checkpoints: Vec::new(),
            error: None,
            cleanup_error: None,
            limitations: vec![
                "Stage1 only: no WASAPI clients or known-signal PCM validation.",
                "No native clients are started by this probe; external clients are not counted.",
                "Exited plus join proves service teardown/control-file release, not final PDO absence.",
                "Same control owner and one Engine owner use production ManagedDriver wiring; raw driver epochs are not public service observations.",
            ],
        }
    }
}

impl Report {
    pub fn checkpoint(&mut self, stage: &str, evidence: Evidence) {
        self.current_phase = stage.into();
        if let Some(checkpoint) = self
            .checkpoints
            .iter_mut()
            .find(|entry| entry.stage == stage)
        {
            checkpoint.evidence = evidence;
        } else {
            self.checkpoints.push(Checkpoint {
                stage: stage.into(),
                evidence,
            });
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct IdleHoldEvidence {
    pub observed_wall_ms: u128,
    pub confirmed_revision: u64,
    pub timeline_verified: bool,
    pub timeline: super::validation::TimelineProgress,
    pub after_old_three_second_threshold: super::validation::TimelineProgress,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct PcmEvidence {
    #[cfg(windows)]
    pub identity: Option<super::native::IdentityEvidence>,
    pub source: [f32; 2],
    pub expected_after_gain: [f32; 2],
    pub second_source: [f32; 2],
    pub second_expected_after_gain: [f32; 2],
    pub signal: super::validation::Observation,
    pub second_signal: super::validation::Observation,
    pub stopped_silence: super::validation::Observation,
    pub signal_elapsed_ms: u64,
    pub second_signal_elapsed_ms: u64,
    pub silence_elapsed_ms: u64,
    pub settle_ms_per_phase: u64,
    pub capture_running_at_stop_request: bool,
    pub capture_running_after_stop_ack: bool,
    pub capture_running_during_silence: bool,
    pub native_discontinuities: u64,
    pub native_clients_stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ExportEvidence {
    pub endpoint_id: String,
    pub direction: ExportDirection,
    pub present: bool,
    pub enabled: bool,
    pub bound: bool,
    pub channels: usize,
    pub allow_default: bool,
    pub gate_open: bool,
    pub active_revision: Option<u64>,
    pub native_stamp: Option<[u64; 5]>,
    pub windows_name: Option<WindowsNameObservation>,
    pub resource: String,
}

/// Detached service evidence only; never grants native readiness or ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct WindowsNameObservation {
    pub native_identity: [u64; 3],
    pub observed_qpc_100ns: Option<u64>,
    pub mmdevice_id: Option<String>,
    pub name: Option<String>,
    pub error: Option<String>,
}

impl From<&ExportStatus> for ExportEvidence {
    fn from(export: &ExportStatus) -> Self {
        Self {
            endpoint_id: export.intent.endpoint_id.to_string(),
            direction: export.intent.direction,
            present: export.resource == moiren_app::service::ExportResourceState::Present,
            enabled: export.intent.enabled,
            bound: export.intent.binding.is_some(),
            channels: export.intent.channels,
            allow_default: export.allow_default,
            gate_open: export.gate_open,
            active_revision: export.active_revision,
            native_stamp: export.native_stamp,
            windows_name: export
                .windows_name
                .as_ref()
                .map(|observation| WindowsNameObservation {
                    native_identity: observation.native_identity,
                    observed_qpc_100ns: observation.observed_qpc_100ns,
                    mmdevice_id: observation.mmdevice_id.clone(),
                    name: observation.name.clone(),
                    error: observation.error.clone(),
                }),
            resource: format!("{:?}", export.resource).to_ascii_lowercase(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Evidence {
    pub phase: String,
    pub candidate_revision: u64,
    pub active_revision: Option<u64>,
    pub pending_revision: Option<u64>,
    pub consumed_request_id: u64,
    pub exports: Vec<ExportEvidence>,
    pub error: Option<String>,
    pub graph_error: Option<String>,
}

impl Evidence {
    pub fn running(&self, ids: &[String; 2], request: u64) -> bool {
        self.owner_running(ids, request) && self.gates_open()
    }

    pub fn owner_running(&self, ids: &[String; 2], request: u64) -> bool {
        self.phase == "running"
            && self.active_revision == Some(self.candidate_revision)
            && self.pending_revision.is_none()
            && self.consumed_request_id >= request
            && self.error.is_none()
            && self.graph_error.is_none()
            && self.exact_exports(ids)
            && self
                .exports
                .iter()
                .all(|export| export.present && export.active_revision == self.active_revision)
    }

    pub fn gates_open(&self) -> bool {
        !self.exports.is_empty() && self.exports.iter().all(|export| export.gate_open)
    }

    pub fn stopped(&self, ids: &[String; 2], request: u64) -> bool {
        self.phase == "idle"
            && self.consumed_request_id >= request
            && self.active_revision.is_none()
            && self.pending_revision.is_none()
            && self.error.is_none()
            && self.graph_error.is_none()
            && self.exact_exports(ids)
            && self.exports.iter().all(|export| {
                export.present && !export.gate_open && export.active_revision.is_none()
            })
    }

    pub fn exited(&self) -> bool {
        self.phase == "exited"
            && self.active_revision.is_none()
            && self.pending_revision.is_none()
            && self.exports.iter().all(|export| {
                !export.gate_open
                    && export.active_revision.is_none()
                    && matches!(export.resource.as_str(), "absent" | "removing")
            })
    }

    fn exact_exports(&self, ids: &[String; 2]) -> bool {
        self.exports.len() == ids.len()
            && ids.iter().all(|id| {
                self.exports
                    .iter()
                    .filter(|export| &export.endpoint_id == id)
                    .count()
                    == 1
            })
            && self.exports.iter().all(|export| {
                export.enabled && export.bound && export.channels == 2 && !export.allow_default
            })
    }
}

#[cfg(windows)]
impl From<&moiren_app::service::AppSnapshot> for Evidence {
    fn from(snapshot: &moiren_app::service::AppSnapshot) -> Self {
        Self {
            phase: format!("{:?}", snapshot.phase).to_ascii_lowercase(),
            candidate_revision: snapshot.graph.candidate_revision,
            active_revision: snapshot.graph.active_revision,
            pending_revision: snapshot.graph.pending_revision,
            consumed_request_id: snapshot.consumed_request_id.0,
            exports: snapshot.exports.iter().map(ExportEvidence::from).collect(),
            error: snapshot.error.clone(),
            graph_error: snapshot.graph.last_error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> [String; 2] {
        ["playback".into(), "microphone".into()]
    }

    fn running() -> Evidence {
        Evidence {
            phase: "running".into(),
            candidate_revision: 7,
            active_revision: Some(7),
            pending_revision: None,
            consumed_request_id: 2,
            exports: ids()
                .into_iter()
                .zip([ExportDirection::Playback, ExportDirection::Microphone])
                .map(|(endpoint_id, direction)| ExportEvidence {
                    endpoint_id,
                    direction,
                    present: true,
                    enabled: true,
                    bound: true,
                    channels: 2,
                    allow_default: false,
                    gate_open: true,
                    active_revision: Some(7),
                    native_stamp: Some([31, 42, 31, 0, 1]),
                    windows_name: None,
                    resource: "present".into(),
                })
                .collect(),
            error: None,
            graph_error: None,
        }
    }

    #[test]
    fn running_needs_both_exact_exports_and_current_engine_confirmation() {
        assert!(running().running(&ids(), 2));
        for change in 0..10 {
            let mut evidence = running();
            match change {
                0 => evidence.phase = "starting".into(),
                1 => evidence.active_revision = Some(6),
                2 => evidence.pending_revision = Some(7),
                3 => evidence.exports[0].gate_open = false,
                4 => evidence.exports[0].present = false,
                5 => evidence.exports[0].active_revision = Some(6),
                6 => evidence.exports[1].endpoint_id = "playback".into(),
                7 => evidence.exports[0].allow_default = true,
                8 => evidence.error = Some("worker failed".into()),
                _ => evidence.graph_error = Some("lifecycle failed".into()),
            }
            assert!(!evidence.running(&ids(), 2), "accepted change {change}");
        }
        assert!(!running().running(&ids(), 3));
    }

    #[test]
    fn owner_validity_is_independent_of_gate_readiness_but_keeps_revision_and_presence() {
        let mut evidence = running();
        evidence.exports[0].gate_open = false;
        assert!(evidence.owner_running(&ids(), 2));
        assert!(!evidence.running(&ids(), 2));
        evidence.exports[0].present = false;
        assert!(!evidence.owner_running(&ids(), 2));
        evidence.exports[0].present = true;
        evidence.active_revision = Some(6);
        assert!(!evidence.owner_running(&ids(), 2));
    }

    #[test]
    fn stop_requires_acknowledged_idle_closed_gates_and_retained_endpoints() {
        let mut stopped = running();
        stopped.phase = "idle".into();
        stopped.active_revision = None;
        for export in &mut stopped.exports {
            export.gate_open = false;
            export.active_revision = None;
        }
        assert!(stopped.stopped(&ids(), 2));
        assert!(!stopped.stopped(&ids(), 3));
        let mut missing = stopped.clone();
        missing.exports[0].present = false;
        assert!(!missing.stopped(&ids(), 2));
        stopped.exports[1].gate_open = true;
        assert!(!stopped.stopped(&ids(), 2));
    }

    #[test]
    fn exit_accepts_removing_without_claiming_confirmed_pdo_absence() {
        let mut exited = running();
        exited.phase = "exited".into();
        exited.active_revision = None;
        for export in &mut exited.exports {
            export.present = false;
            export.resource = "removing".into();
            export.gate_open = false;
            export.active_revision = None;
        }
        assert!(exited.exited());
        let json = serde_json::to_value(&exited).unwrap();
        assert_eq!(json["exports"][0]["resource"], "removing");
        assert_eq!(json["active_revision"], serde_json::Value::Null);
        exited.exports[0].gate_open = true;
        assert!(!exited.exited());
    }

    #[test]
    fn stage1_report_never_promotes_service_running_to_pcm_or_pdo_proof() {
        let json = serde_json::to_value(Report::default()).unwrap();
        assert_eq!(json["status"], "failed");
        assert_eq!(json["native_clients_started_by_probe"], 0);
        assert_eq!(json["pcm_verified"], false);
        assert_eq!(json["service_joined"], false);
        assert_eq!(json["control_session_released"], false);
        assert_eq!(json["endpoint_absence_confirmed"], false);
    }

    #[test]
    fn export_report_retains_direction_and_explicit_unknown_windows_name() {
        let json = serde_json::to_value(running()).unwrap();
        assert_eq!(json["exports"][0]["direction"], "playback");
        assert_eq!(json["exports"][1]["direction"], "microphone");
        for export in json["exports"].as_array().unwrap() {
            assert_eq!(export.get("windows_name"), Some(&serde_json::Value::Null));
        }
    }

    fn export_status() -> ExportStatus {
        let document = super::super::project::document().unwrap();
        let export = &document.exports[0];
        ExportStatus {
            intent: moiren_app::service::ExportIntent {
                endpoint_id: export.endpoint_id,
                direction: export.direction,
                name: export.name.clone(),
                channels: export.channels,
                enabled: export.enabled,
                binding: Some(
                    moiren_core::graph::LogicalGraph::new()
                        .create_node(moiren_core::graph::NodeKind::Source, 2)
                        .unwrap(),
                ),
            },
            actual_name: Some("provider name must not replace Windows readback".into()),
            windows_name: Some(moiren_app::service::WindowsEndpointNameObservation {
                native_identity: [31, 42, 31],
                observed_qpc_100ns: Some(123_456),
                mmdevice_id: Some("{exact-owned-render-id}".into()),
                name: Some("Actual Windows name".into()),
                error: None,
            }),
            rename_pending: false,
            resource: moiren_app::service::ExportResourceState::Present,
            allow_default: false,
            gate_open: true,
            active_revision: Some(7),
            #[cfg(windows)]
            diagnostics: None,
            native_stamp: Some([31, 42, 31, 0, 1]),
            observation_generation: Some(9),
            native_retirement: None,
        }
    }

    #[test]
    fn service_export_projects_exact_typed_windows_name() {
        let evidence = ExportEvidence::from(&export_status());
        let json = serde_json::to_value(&evidence).unwrap();
        assert_eq!(json["direction"], "playback");
        assert_eq!(
            json["windows_name"],
            serde_json::json!({
                "native_identity": [31, 42, 31],
                "observed_qpc_100ns": 123456,
                "mmdevice_id": "{exact-owned-render-id}",
                "name": "Actual Windows name",
                "error": null,
            })
        );
        assert_eq!(json["native_stamp"], serde_json::json!([31, 42, 31, 0, 1]));
    }

    #[test]
    fn stale_missing_and_failed_windows_names_are_raw_evidence_not_readiness() {
        for change in 0..7 {
            let mut status = export_status();
            match change {
                0 => status.windows_name = None,
                1 => status.windows_name.as_mut().unwrap().native_identity[0] += 1,
                2 => status.windows_name.as_mut().unwrap().native_identity[1] += 1,
                3 => status.windows_name.as_mut().unwrap().native_identity[2] += 1,
                4 => status.windows_name.as_mut().unwrap().mmdevice_id = None,
                5 => status.windows_name.as_mut().unwrap().mmdevice_id = Some(String::new()),
                _ => status.windows_name.as_mut().unwrap().error = Some("topology failed".into()),
            }
            let mut evidence = running();
            let mut projected = ExportEvidence::from(&status);
            projected.endpoint_id = ids()[0].clone();
            evidence.exports[0] = projected;
            assert!(
                evidence.running(&ids(), 2),
                "readiness changed for {change}"
            );
            let json = serde_json::to_value(&evidence).unwrap();
            let expected = status.windows_name.as_ref().map(|name| {
                serde_json::json!({
                    "native_identity": name.native_identity,
                    "observed_qpc_100ns": name.observed_qpc_100ns,
                    "mmdevice_id": name.mmdevice_id,
                    "name": name.name,
                    "error": name.error,
                })
            });
            assert_eq!(
                json["exports"][0]["windows_name"],
                expected.unwrap_or(serde_json::Value::Null),
                "lost raw observation for {change}"
            );
        }
    }

    #[test]
    fn progress_retains_typed_windows_name_without_adding_qualification() {
        let directory = std::env::temp_dir().join(format!(
            "moiren-holder-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut report = Report::default();
        let mut evidence = running();
        evidence.exports[0] = ExportEvidence::from(&export_status());
        report.checkpoint("no_clients_timeline_hold", evidence);
        super::super::progress::persist(&directory, &report, 1, 20, 30, false).unwrap();
        let bytes = std::fs::read(directory.join("progress.json")).unwrap();
        std::fs::remove_file(directory.join("progress.json")).unwrap();
        std::fs::remove_dir(directory).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let export = &json["report"]["checkpoints"][0]["evidence"]["exports"][0];
        assert_eq!(export["direction"], "playback");
        assert_eq!(
            export["windows_name"]["native_identity"],
            serde_json::json!([31, 42, 31])
        );
        assert_eq!(
            export["windows_name"]["mmdevice_id"],
            "{exact-owned-render-id}"
        );
        assert_eq!(export["windows_name"]["error"], serde_json::Value::Null);
        assert_eq!(json["final_qualification"], false);
        assert_eq!(json["report"]["native_clients_started_by_probe"], 0);
        assert_eq!(json["report"]["pcm_verified"], false);
    }

    #[test]
    fn rename_fixture_seed_is_traceable_without_changing_default_report() {
        let default = serde_json::to_value(Report::default()).unwrap();
        assert!(default.get("rename_fixture_seed").is_none());
        let seed = u64::MAX;
        let report = Report {
            rename_fixture_seed: Some(seed),
            ..Default::default()
        };
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["rename_fixture_seed"].as_u64(), Some(seed));
        assert_eq!(json["status"], "failed");
        assert_eq!(json["persisted_project_verified"], false);
        assert_eq!(json["endpoint_absence_confirmed"], false);
    }

    #[test]
    fn alternating_soak_phases_keep_only_one_checkpoint_per_fixed_stage() {
        let mut report = Report::default();
        for _ in 0..10_000 {
            report.checkpoint("soak_pcm", running());
            report.checkpoint("soak_clock", running());
        }
        assert_eq!(report.checkpoints.len(), 2);
    }
}
