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
    pub service_joined: bool,
    pub control_session_released: bool,
    pub endpoint_absence_confirmed: bool,
    pub checkpoints: Vec<Checkpoint>,
    pub error: Option<String>,
    pub cleanup_error: Option<String>,
    pub limitations: Vec<&'static str>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ExportEvidence {
    pub endpoint_id: String,
    pub present: bool,
    pub enabled: bool,
    pub bound: bool,
    pub channels: usize,
    pub allow_default: bool,
    pub gate_open: bool,
    pub active_revision: Option<u64>,
    pub resource: String,
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
        self.phase == "running"
            && self.active_revision == Some(self.candidate_revision)
            && self.pending_revision.is_none()
            && self.consumed_request_id >= request
            && self.error.is_none()
            && self.graph_error.is_none()
            && self.exact_exports(ids)
            && self.exports.iter().all(|export| {
                export.present && export.gate_open && export.active_revision == self.active_revision
            })
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
            exports: snapshot
                .exports
                .iter()
                .map(|export| ExportEvidence {
                    endpoint_id: export.intent.endpoint_id.to_string(),
                    present: export.resource == moiren_app::service::ExportResourceState::Present,
                    enabled: export.intent.enabled,
                    bound: export.intent.binding.is_some(),
                    channels: export.intent.channels,
                    allow_default: export.allow_default,
                    gate_open: export.gate_open,
                    active_revision: export.active_revision,
                    resource: format!("{:?}", export.resource).to_ascii_lowercase(),
                })
                .collect(),
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
                .map(|endpoint_id| ExportEvidence {
                    endpoint_id,
                    present: true,
                    enabled: true,
                    bound: true,
                    channels: 2,
                    allow_default: false,
                    gate_open: true,
                    active_revision: Some(7),
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
}
