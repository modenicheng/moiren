//! Clock debt is cumulative Engine loss, not a native packet or latency measure.

use moiren_windows_audio::virtual_owner::VirtualClockSnapshot;
use rust_i18n::t;

pub(super) fn append_rows(rows: &mut Vec<(String, String)>, clock: Option<VirtualClockSnapshot>) {
    let observation = match clock {
        None => super::unknown(),
        Some(clock) if clock.schema_version == 1 => {
            t!("Schema 1 (owner-local cumulative counters, not latency)").into()
        }
        Some(clock) => t!(
            "Unsupported schema %{schema}",
            schema = clock.schema_version
        )
        .into(),
    };
    // Unknown or a future DTO schema must never become zero/healthy evidence.
    let clock = clock.filter(|clock| clock.schema_version == 1);
    rows.extend([
        (t!("Virtual clock observation").into(), observation),
        (
            t!("Virtual clock configuration").into(),
            clock.map(|clock| t!(
                "%{rate} Hz Engine; block %{block} frames; recovery cap %{cap} periods",
                rate = clock.engine_sample_rate,
                block = clock.engine_block_frames,
                cap = clock.recovery_cap_periods,
            ).into()).unwrap_or_else(super::unknown),
        ),
        (
            t!("Virtual clock rendered").into(),
            clock.map(|clock| t!(
                "%{periods} periods / %{frames} Engine frames",
                periods = clock.counters.rendered_periods,
                frames = clock.counters.rendered_frames,
            ).into()).unwrap_or_else(super::unknown),
        ),
        (
            t!("Virtual clock recovery").into(),
            clock.map(|clock| t!(
                "Late wakes %{late}; skipped %{periods} periods / %{frames} Engine frames (not native PCM drops)",
                late = clock.counters.late_wake_bursts,
                periods = clock.counters.clamped_periods,
                frames = clock.counters.clamped_frames,
            ).into()).unwrap_or_else(super::unknown),
        ),
        (
            t!("Virtual clock timeline").into(),
            clock.map(|clock| t!(
                "Resets %{resets}; maximum backlog %{backlog} periods (before cap)",
                resets = clock.counters.timeline_resets,
                backlog = clock.counters.max_backlog_periods,
            ).into()).unwrap_or_else(super::unknown),
        ),
        (
            t!("Virtual clock counter saturation").into(),
            clock.map(|clock| if clock.counters.counters_saturated {
                t!("Saturated (counts are lower bounds)").into()
            } else {
                t!("Not saturated").into()
            }).unwrap_or_else(super::unknown),
        ),
    ]);
}
