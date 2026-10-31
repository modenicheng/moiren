//! One read-only observation after successful nonzero Release, never a gain verdict.
use super::source::SourceProgress;
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(super) struct Reading<T> {
    pub hresult: Option<i32>,
    pub value: Option<T>,
}
impl<T> Reading<T> {
    pub(super) fn observed(hresult: i32, value: T) -> Self {
        Self {
            hresult: Some(hresult),
            value: (hresult >= 0).then_some(value),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Preparation {
    pub session_control_hresult: Option<i32>,
    pub session_volume_hresult: Option<i32>,
    pub endpoint_volume_hresult: Option<i32>,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct SourceControls {
    pub qualified: bool,
    pub mmdevice_id: String,
    pub process_id: u32,
    pub process_creation_100ns: Reading<u64>,
    pub qpc_frequency_hz: Option<u64>,
    pub preparation: Preparation,
    pub snapshot: Option<ControlSnapshot>,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct ControlSnapshot {
    pub phase: &'static str,
    pub client_start_succeeded: bool,
    pub source_progress: SourceProgress,
    pub before_qpc: Option<u64>,
    pub after_qpc: Option<u64>,
    pub stop_requested: bool,
    pub session_state: Reading<i32>,
    pub session_volume: Reading<f32>,
    pub session_mute: Reading<bool>,
    pub endpoint_volume: Reading<f32>,
    pub endpoint_mute: Reading<bool>,
}
pub(super) trait ReadControls {
    fn session_state(&self) -> Reading<i32>;
    fn session_volume(&self) -> Reading<f32>;
    fn session_mute(&self) -> Reading<bool>;
    fn endpoint_volume(&self) -> Reading<f32>;
    fn endpoint_mute(&self) -> Reading<bool>;
}
/// The closure includes COM acquisition, so disabled means no diagnostic calls.
pub(super) fn prepare_if<T>(enabled: bool, prepare: impl FnOnce() -> T) -> Option<T> {
    enabled.then(prepare)
}
/// Call only after successful Start and first confirmed nonzero Release, outside
/// the render lease and publication mutex. COM has NO cancellable hard deadline.
pub(super) fn observe(
    reader: &impl ReadControls,
    progress: SourceProgress,
    mut clock: impl FnMut() -> Option<u64>,
    mut stopped: impl FnMut() -> bool,
) -> ControlSnapshot {
    let mut stop_requested = false;
    let mut read = || {
        stop_requested |= stopped();
        !stop_requested
    };
    let before_qpc = clock();
    let session_state = if read() {
        reader.session_state()
    } else {
        Reading::default()
    };
    let session_volume = if read() {
        reader.session_volume()
    } else {
        Reading::default()
    };
    let session_mute = if read() {
        reader.session_mute()
    } else {
        Reading::default()
    };
    let endpoint_volume = if read() {
        reader.endpoint_volume()
    } else {
        Reading::default()
    };
    let endpoint_mute = if read() {
        reader.endpoint_mute()
    } else {
        Reading::default()
    };
    ControlSnapshot {
        phase: "after_first_nonzero_release",
        client_start_succeeded: true,
        source_progress: progress,
        before_qpc,
        after_qpc: clock(),
        stop_requested,
        session_state,
        session_volume,
        session_mute,
        endpoint_volume,
        endpoint_mute,
    }
}
#[cfg(windows)]
#[path = "source_controls/native.rs"]
pub(super) mod native;
#[cfg(test)]
#[path = "source_controls/tests.rs"]
mod tests;
