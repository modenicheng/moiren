//! Snapshot publication: the deep projection build, its content tokens, and
//! the cheap probes the service loop consults to skip rebuilds.
use super::*;

pub(crate) fn snapshot(c: &ServiceContext) -> AppSnapshot {
    #[cfg(test)]
    c.snapshot_builds
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut graph = c.graph.snapshot();
    // Runtime card state rides the graph projection but never its editing
    // domain: cards repaint on phase transitions only, and the ticking
    // duration stays out of it entirely (graph-page keys must stay 1 Hz-free).
    graph.recording = c.recordings.cards();
    AppSnapshot {
        loaded_project: c.loaded_project.clone(),
        exports: c.exports.statuses(&graph),
        export_plan: c.exports.reconcile(&graph),
        #[cfg(windows)]
        professional_observations: c
            .professional_observations
            .clone()
            .filter(|value| c.fresh(value.generation)),
        latency_profile: c.latency_profile,
        active_latency_profile: c.active_latency_profile,
        audio_settings: c.audio_settings.clone(),
        active_audio_settings: c
            .active_audio_settings
            .clone()
            .filter(|_| c.fresh(c.core.generation())),
        graph: Arc::new(graph),
        consumed_request_id: RequestId::default(),
        generation: c.core.generation(),
        phase: c.core.phase(),
        desired: c.core.desired().cloned(),
        error: c.core.error().map(str::to_owned),
        waiting: c.active_execution
            == Some(moiren_engine::host::ExecutionState::WaitingForResources),
        starting_progress: c.starting_progress.clone(),
        recordings: c.recordings.snapshots(),
        catalog: c.catalog.clone(),
        catalog_error: c.catalog_error.clone(),
        plugins: c.plugins.clone(),
        control: c.control.clone(),
        signal_topics: c.signals.topics.clone(),
        tone_session: c.tone_session,
        // The chrome clock renders whole seconds; publishing sub-second
        // precision would change the snapshot on every poll and keep the UI
        // repainting for as long as the session runs.
        elapsed: Duration::from_secs(c.core.elapsed().as_secs()),
    }
}

/// Content tokens for everything [`snapshot`] projects whose construction or
/// comparison is expensive: the graph deep copy, the export statuses/plan
/// projections, the session core fields, the device catalog, and the plugin
/// library. Each token is advanced by the domain's own mutation paths
/// (graph.rs, core.rs, exports.rs/runtime.rs, and `replace_catalog` /
/// `replace_plugins` here), so a change in any of them is guaranteed to be
/// reflected in the next published snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SnapshotRevision {
    graph: u64,
    core: u64,
    exports: u64,
    catalog: u64,
    plugins: u64,
    recordings: u64,
    /// Whole seconds of the session clock: the only snapshot input that
    /// advances on its own (the published elapsed is quantized to seconds,
    /// so the published content moves at 1 Hz at most).
    elapsed_secs: u64,
}

/// Cheap, allocation-free change probe for the tokened snapshot domains.
pub(crate) fn snapshot_revision(c: &ServiceContext) -> SnapshotRevision {
    SnapshotRevision {
        graph: c.graph.state_epoch(),
        core: c.core.state_epoch(),
        exports: c.exports.state_epoch(),
        catalog: c.catalog_epoch,
        plugins: c.plugins_epoch,
        recordings: c.recordings.state_epoch(),
        elapsed_secs: c.core.elapsed().as_secs(),
    }
}

/// Whether the small, directly comparable snapshot inputs still match the
/// last published snapshot. These inputs are all cheap `PartialEq` values,
/// so they are compared by value instead of carrying tokens; they never
/// require building the graph deep copy or the export projections.
pub(crate) fn small_snapshot_inputs_match(c: &ServiceContext, last: &AppSnapshot) -> bool {
    let loaded_project_same = match (&last.loaded_project, &c.loaded_project) {
        (None, None) => true,
        (Some(published), Some(current)) => Arc::ptr_eq(published, current),
        _ => false,
    };
    // The published observations/settings pass through freshness filters;
    // project the same view without cloning before comparing.
    let observations_same = {
        #[cfg(windows)]
        {
            let projected = c
                .professional_observations
                .as_ref()
                .filter(|value| c.fresh(value.generation));
            last.professional_observations.as_ref() == projected
        }
        #[cfg(not(windows))]
        {
            true
        }
    };
    let active_settings_same = {
        let projected = c
            .active_audio_settings
            .as_ref()
            .filter(|_| c.fresh(c.core.generation()));
        last.active_audio_settings.as_ref() == projected
    };
    loaded_project_same
        && observations_same
        && active_settings_same
        && last.latency_profile == c.latency_profile
        && last.active_latency_profile == c.active_latency_profile
        && last.audio_settings == c.audio_settings
        && last.catalog_error == c.catalog_error
        && last.control == c.control
        && last.starting_progress == c.starting_progress
        && last.signal_topics == c.signals.topics
        && last.tone_session == c.tone_session
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod snapshot_publish_tests;
