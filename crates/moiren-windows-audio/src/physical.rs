//! Explicit endpoint probes: native f32 capture and Shared silent render.
use crate::{
    catalog::{self, ApiFailure},
    owner::Apartment,
};
use std::time::{SystemTime, UNIX_EPOCH};

mod analysis;
mod report;
mod stream;

use analysis::analyze_endpoints;
pub use report::{DemandRecord, EndpointReport, PhysicalReport, RelativeClock, RenderSummary};
use stream::prepare_and_sample;

/// All endpoints are explicit. No name matching or default switching occurs here.
pub fn run(
    ids: Vec<String>,
    seconds: u32,
    observe_pid: Option<u32>,
) -> anyhow::Result<PhysicalReport> {
    if ids.is_empty()
        || ids.len() > 8
        || !(1..=600).contains(&seconds)
        || ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id))
    {
        anyhow::bail!(
            "expected 1..=8 unique endpoint IDs and --seconds within 1..=600, got {} IDs and {seconds}s",
            ids.len()
        );
    }
    let _apartment = Apartment::new()?;
    let before = catalog::snapshot()?;
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |t| t.as_millis());
    let mut threads = Vec::new();
    for id in ids {
        let name = before
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == id)
            .and_then(|endpoint| endpoint.name.clone());
        threads.push(std::thread::spawn(move || {
            let mut report = EndpointReport::new(id, name);
            if let Err(error) = prepare_and_sample(&mut report, seconds) {
                report.status = "api_failed";
                report
                    .errors
                    .push(ApiFailure::new(report.last_stage, error));
            }
            report
        }));
    }
    let mut endpoints = Vec::new();
    let mut worker_panicked = false;
    for thread in threads {
        match thread.join() {
            Ok(endpoint) => endpoints.push(endpoint),
            Err(_) => worker_panicked = true,
        }
    }
    if worker_panicked {
        // Join every owner before returning; dropping a JoinHandle detaches it.
        anyhow::bail!("an endpoint worker thread panicked during sampling");
    }
    let (window, relative_clocks) = analyze_endpoints(&mut endpoints);
    let mut result = PhysicalReport {
        schema_version: 1,
        started_unix_ms,
        requested_seconds: seconds,
        before,
        endpoints,
        common_qpc_window_100ns: window,
        relative_clocks,
        after: None,
        observed_state_changes: Vec::new(),
        errors: Vec::new(),
    };
    match catalog::snapshot() {
        Ok(after) => {
            result.observed_state_changes =
                catalog::changes(&result.before, &after, observe_pid.unwrap_or(0));
            result.after = Some(after);
        }
        Err(error) => result
            .errors
            .push(ApiFailure::new("post-probe snapshot", error)),
    }
    Ok(result)
}
