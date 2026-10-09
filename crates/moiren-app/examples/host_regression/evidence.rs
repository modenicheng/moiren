//! Statistics-only output: never persist PCM, endpoint IDs, paths or process IDs.
use moiren_app::host::{
    HostEvent,
    windows::{SessionReport, SessionSnapshot},
};
use serde_json::{Value, json};

pub(super) fn snapshot(label: &str, elapsed: f64, snapshot: &SessionSnapshot) -> Value {
    json!({"label":label,"elapsed_seconds":elapsed,"status":snapshot.status,
        "runtime":snapshot.runtime,"failure":snapshot.failure,
        "sources":snapshot.sources.iter().map(|source|json!({"id":source.id,"status":source.status,
            "available":source.available,"enabled":source.enabled,"bridge":source.bridge,
            "failure":source.failure,"last_start_failure":source.last_start_failure})).collect::<Vec<_>>()})
}

pub(super) fn events(events: Vec<HostEvent>) -> Vec<Value> {
    events
        .into_iter()
        .map(moiren_app::host_cli::event_json)
        .collect()
}

pub(super) fn report(report: &SessionReport) -> Value {
    json!({"snapshot":snapshot("stopped",0.0,&report.snapshot),"events":report.events,
        "render":report.render.as_ref().map(|render|json!({"status":render.status,"elapsed_seconds":render.elapsed_seconds,
            "stream_started":render.stream_started,"stop_succeeded":render.stop_succeeded,
            "native_mix_sample_rate":render.native_mix_sample_rate,"native_mix_channels":render.native_mix_channels,
            "buffer_frames":render.buffer_frames,"stats":render.stats,"failure":render.failure}))})
}
