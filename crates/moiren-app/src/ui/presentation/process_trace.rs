//! Closed predecessor diagnostics intentionally survive active-state filtering.
use super::*;

pub fn process_trace_rows(snapshot: &AppSnapshot) -> Vec<(String, String)> {
    #[cfg(not(windows))]
    {
        let _ = snapshot;
        Vec::new()
    }
    #[cfg(windows)]
    {
        let mut rows = Vec::new();
        if let Some(status) = &snapshot.process_trace_status {
            rows.push((
                t!("Process trace collection").into(),
                status.refusal.clone().unwrap_or_else(|| {
                    t!("Collecting a bounded prefix; not latency qualification.").into()
                }),
            ));
        }
        if let Some(trace) = snapshot
            .last_cleanup
            .as_ref()
            .and_then(|c| c.process_trace.as_ref())
        {
            let origin = trace.origin();
            rows.push((
                t!("Process trace predecessor").into(),
                format!(
                    "session {}; document {}; graph {}; native {}; pid {} / {}",
                    origin.generation.0,
                    origin.document,
                    origin.graph_revision,
                    origin.native_owner_generation,
                    origin.process.pid,
                    origin.process.creation_time_100ns,
                ),
            ));
            let artifact = trace.artifact();
            rows.push((
                t!("Process trace prefix").into(),
                t!(
                    "%{packets} packets / %{reads} reads retained; limit 8192 per writer",
                    packets = artifact.packet_history.events.len(),
                    reads = artifact.queue_history.events.len(),
                )
                .into(),
            ));
            rows.push((
                t!("Process trace result").into(),
                trace.refusal().map(str::to_owned).unwrap_or_else(|| {
                    t!("Closed prefix available; not latency qualification.").into()
                }),
            ));
        }
        rows
    }
}
