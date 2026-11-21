//! Catalog refresh, sandboxed plugin scans, and the VAD endpoint catalog
//! reconciliation that closes every poll round.
use super::super::*;

#[cfg(test)]
#[path = "vad_catalog_tests.rs"]
mod vad_catalog_tests;

pub(super) fn update_vad_catalog(context: &mut ServiceContext) {
    // The export projection reads only the export list and the active
    // revision, so borrow the graph snapshot instead of deep-copying it
    // into `statuses(&GraphSnapshot)`.
    let statuses = {
        let graph = context.graph.snapshot_ref();
        context
            .exports
            .statuses_for(&graph.exports, graph.active_revision)
    };
    let live_exports = statuses
        .iter()
        .filter(|status| status.resource != crate::service::ExportResourceState::Absent)
        .map(|status| status.intent.endpoint_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut owned = context.catalog.vad_endpoints.clone();
    owned.retain(|endpoint_id, _| live_exports.contains(endpoint_id));
    for status in statuses {
        if !matches!(
            status.resource,
            crate::service::ExportResourceState::Present
                | crate::service::ExportResourceState::Removing
        ) {
            continue;
        }
        if let Some(mmdevice_id) = status.windows_name.and_then(|name| name.mmdevice_id) {
            owned.insert(status.intent.endpoint_id, mmdevice_id);
        }
    }
    if owned != context.catalog.vad_endpoints {
        let mut catalog = (*context.catalog).clone();
        catalog.vad_endpoints = owned;
        context.replace_catalog(catalog);
    }
}

impl ManagedDriver {
    /// Kick a catalog refresh when the refresh interval elapsed.
    pub(super) fn poll_catalog(&mut self, pool: &mut JobPool) {
        let completions = self
            .completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()));
        self.catalog.poll(
            self.exiting,
            self.catalog_backend.as_ref(),
            completions,
            pool,
            Completion::Catalog,
        );
    }
    /// Submit the requested sandboxed plugin scan on the prepare lane. The
    /// job serves unchanged roots from the scan cache (or spawns one scanner
    /// subprocess per remaining standard root when forced) and waits out
    /// their deadlines, so it never runs on the service thread.
    pub(super) fn poll_plugin_scan(&mut self, pool: &mut JobPool) {
        if self.exiting || !self.plugin_scan.requested || self.plugin_scan.running {
            return;
        }
        let completions = self
            .completions
            .get_or_insert_with(|| CompletionQueue::new(thread::current()));
        let Ok(credit) = completions.port().try_reserve() else {
            return;
        };
        let force = self.plugin_scan.force;
        let job: Job = Box::new(move || {
            let scan = if force {
                crate::service::run_forced_plugin_scan
            } else {
                crate::service::run_sandbox_scan
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(scan))
                .unwrap_or_else(|_| {
                    Err(crate::service::BackendError::new(
                        "plugin-scan",
                        "the scan worker panicked",
                    ))
                });
            credit.send_or_dispose(Completion::PluginScan(result), drop);
        });
        if pool.try_submit_owned(JobPriority::Prepare, job).is_ok() {
            self.plugin_scan.running = true;
            self.plugin_scan.requested = false;
        }
    }
}
