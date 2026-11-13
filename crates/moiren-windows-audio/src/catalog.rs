use crate::{
    negotiation::{StageError, friendly_name},
    win32_raii::{Process, TaskMemory, take_string},
};
use serde::Serialize;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use windows::{
    Win32::Media::Audio::{Endpoints::IAudioEndpointVolume, *},
    Win32::System::Com::{CLSCTX_ALL, CoCreateInstance},
    core::{Interface, Result},
};

#[derive(Serialize)]
pub struct ApiFailure {
    pub stage: String,
    pub hresult: String,
}
impl ApiFailure {
    pub fn new(stage: &str, error: windows::core::Error) -> Self {
        Self::from_hresult(stage, error.code().0)
    }
    /// The shared stage+HRESULT shape (kernel `StageError`).
    fn from_hresult(stage: &str, hresult: i32) -> Self {
        Self {
            stage: stage.to_owned(),
            hresult: format!("0x{:08X}", hresult as u32),
        }
    }
}
impl From<StageError> for ApiFailure {
    fn from(failure: StageError) -> Self {
        Self::from_hresult(failure.stage, failure.hresult)
    }
}
fn record<T>(stage: &str, errors: &mut Vec<ApiFailure>, value: Result<T>) -> Option<T> {
    match value {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(ApiFailure::new(stage, error));
            None
        }
    }
}

#[derive(Serialize)]
pub struct FormatSnapshot {
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub container_bits: u16,
    pub block_align: u16,
    pub avg_bytes_per_sec: u32,
    pub extra_bytes: u16,
    pub valid_bits: Option<u16>,
    pub channel_mask: Option<u32>,
    pub sub_format: Option<String>,
}
impl FormatSnapshot {
    pub fn from_base(format: WAVEFORMATEX) -> Self {
        Self {
            format_tag: format.wFormatTag,
            channels: format.nChannels,
            sample_rate: format.nSamplesPerSec,
            container_bits: format.wBitsPerSample,
            block_align: format.nBlockAlign,
            avg_bytes_per_sec: format.nAvgBytesPerSec,
            extra_bytes: format.cbSize,
            valid_bits: None,
            channel_mask: None,
            sub_format: None,
        }
    }
}

#[derive(Serialize)]
pub struct SessionSnapshot {
    pub process_id: u32,
    pub pid_query_hresult: String,
    pub process_name: Option<String>,
    pub instance_key: String,
    pub state: Option<i32>,
    pub volume_scalar: Option<f32>,
    pub muted: Option<bool>,
    pub errors: Vec<ApiFailure>,
}

#[derive(Serialize)]
pub struct EndpointSnapshot {
    pub id: String,
    pub name: Option<String>,
    pub flow: &'static str,
    pub mix_format: Option<FormatSnapshot>,
    pub default_period_100ns: Option<i64>,
    pub minimum_period_100ns: Option<i64>,
    pub volume_scalar: Option<f32>,
    pub muted: Option<bool>,
    pub sessions: Vec<SessionSnapshot>,
    pub errors: Vec<ApiFailure>,
}

#[derive(Serialize, PartialEq)]
pub struct DefaultEndpoint {
    pub flow: &'static str,
    pub role: &'static str,
    pub id: Option<String>,
}

#[derive(Serialize)]
pub struct CatalogSnapshot {
    pub defaults: Vec<DefaultEndpoint>,
    pub endpoints: Vec<EndpointSnapshot>,
    pub errors: Vec<ApiFailure>,
}

/// Worker-side COM apartment is released before this owned DTO returns.
pub fn owned_snapshot() -> Result<CatalogSnapshot> {
    let _apartment = crate::win32_raii::Apartment::new()?;
    snapshot()
}

fn sessions(device: &IMMDevice) -> Result<Vec<SessionSnapshot>> {
    unsafe {
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let enumerator = manager.GetSessionEnumerator()?;
        let mut result = Vec::new();
        for index in 0..enumerator.GetCount()? {
            let session: IAudioSessionControl2 = enumerator.GetSession(index)?.cast()?;
            let mut process_id = 0;
            // Preserve AUDCLNT_S_NO_SINGLE_PROCESS; the convenience binding loses successful HRESULTs.
            let pid_result = (Interface::vtable(&session).GetProcessId)(
                Interface::as_raw(&session),
                &mut process_id,
            );
            pid_result.ok()?;
            let identifier = take_string(session.GetSessionInstanceIdentifier()?)?;
            let mut hash = DefaultHasher::new();
            identifier.hash(&mut hash);
            let mut errors = Vec::new();
            let process_name = if process_id == 0 {
                Some("System Sounds".to_owned())
            } else {
                record("process identity", &mut errors, Process::open(process_id))
                    .map(|process| process.identity.executable_name)
            };
            let state =
                record("session state", &mut errors, session.GetState()).map(|state| state.0);
            let volume: Option<ISimpleAudioVolume> =
                record("session volume interface", &mut errors, session.cast());
            let volume_scalar = volume
                .as_ref()
                .and_then(|v| record("session volume", &mut errors, v.GetMasterVolume()));
            let muted = volume.as_ref().and_then(|v| {
                record("session mute", &mut errors, v.GetMute()).map(|v| v.as_bool())
            });
            result.push(SessionSnapshot {
                process_id,
                pid_query_hresult: format!("0x{:08X}", pid_result.0 as u32),
                process_name,
                instance_key: format!("{:016x}", hash.finish()),
                state,
                volume_scalar,
                muted,
                errors,
            });
        }
        result.sort_by(|a, b| a.instance_key.cmp(&b.instance_key));
        Ok(result)
    }
}

fn endpoint(device: &IMMDevice, flow: &'static str) -> Result<EndpointSnapshot> {
    unsafe {
        let id = take_string(device.GetId()?)?;
        let mut errors = Vec::new();
        let name = record("friendly name", &mut errors, friendly_name(device));
        let client: Option<IAudioClient> = record(
            "mix format client",
            &mut errors,
            device.Activate(CLSCTX_ALL, None),
        );
        let mix_format = client.as_ref().and_then(|client| {
            record("GetMixFormat", &mut errors, client.GetMixFormat()).map(|ptr| {
                let memory = TaskMemory(ptr);
                let base = memory.0.read_unaligned();
                let mut snapshot = FormatSnapshot::from_base(base);
                if base.wFormatTag == 0xfffe
                    && usize::from(base.cbSize)
                        >= size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()
                {
                    let extended = memory.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned();
                    snapshot.valid_bits = Some(extended.Samples.wValidBitsPerSample);
                    snapshot.channel_mask = Some(extended.dwChannelMask);
                    let guid = extended.SubFormat;
                    snapshot.sub_format = Some(format!("{guid:?}"));
                }
                snapshot
            })
        });
        let mut periods = None;
        if let Some(client) = &client {
            let (mut default, mut minimum) = (0, 0);
            if record(
                "GetDevicePeriod",
                &mut errors,
                client.GetDevicePeriod(Some(&mut default), Some(&mut minimum)),
            )
            .is_some()
            {
                periods = Some((default, minimum));
            }
        }
        let volume: Option<IAudioEndpointVolume> = record(
            "endpoint volume interface",
            &mut errors,
            device.Activate(CLSCTX_ALL, None),
        );
        let volume_scalar = volume.as_ref().and_then(|v| {
            record(
                "endpoint volume",
                &mut errors,
                v.GetMasterVolumeLevelScalar(),
            )
        });
        let muted = volume
            .as_ref()
            .and_then(|v| record("endpoint mute", &mut errors, v.GetMute()).map(|v| v.as_bool()));
        let sessions = if flow == "render" {
            record("session enumeration", &mut errors, sessions(device)).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(EndpointSnapshot {
            id,
            name,
            flow,
            mix_format,
            default_period_100ns: periods.map(|p| p.0),
            minimum_period_100ns: periods.map(|p| p.1),
            volume_scalar,
            muted,
            sessions,
            errors,
        })
    }
}

/// Deadline for the whole per-endpoint probe wave (issue #105): one wedged
/// endpoint's `Activate`/`GetMixFormat`/session enumeration must not stall
/// the catalog refresh — and the automatic startup waiting behind it —
/// forever. Healthy endpoints answer in well under a second.
const PROBE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Tests shrink the wave so a hung probe bounds the test, not the suite.
fn probe_deadline() -> std::time::Duration {
    if cfg!(test) {
        std::time::Duration::from_millis(300)
    } else {
        PROBE_DEADLINE
    }
}

/// A stuck endpoint keeps its row so the device stays visible and selectable;
/// only its unknown fields stay `None` and the failure joins its errors.
fn bare_endpoint(id: String, flow: &'static str, errors: Vec<ApiFailure>) -> EndpointSnapshot {
    EndpointSnapshot {
        id,
        name: None,
        flow,
        mix_format: None,
        default_period_100ns: None,
        minimum_period_100ns: None,
        volume_scalar: None,
        muted: None,
        sessions: Vec::new(),
        errors,
    }
}

fn not_responding(id: String, flow: &'static str) -> EndpointSnapshot {
    bare_endpoint(
        id,
        flow,
        vec![ApiFailure {
            stage: "endpoint probe".to_owned(),
            hresult: format!("no response within {:?}", probe_deadline()),
        }],
    )
}

/// One endpoint's full probe, apartment-private on whichever thread runs it.
type EndpointProbe =
    std::sync::Arc<dyn Fn(&str, &'static str) -> Result<EndpointSnapshot> + Send + Sync>;

/// Probe each enumerated endpoint on its own thread with its own apartment
/// and a shared deadline. Threads that miss the deadline are abandoned — the
/// OS call unwinds on that thread when it finally returns — and their rows
/// are recorded as not responding instead of blocking the snapshot.
fn collect_endpoints(
    ids: Vec<(String, &'static str)>,
    probe: EndpointProbe,
) -> Vec<EndpointSnapshot> {
    let total = ids.len();
    let (sender, receiver) = std::sync::mpsc::channel::<(String, Result<EndpointSnapshot>)>();
    let mut spawned = 0usize;
    for (id, flow) in &ids {
        let sender = sender.clone();
        let id = id.clone();
        let flow = *flow;
        let probe = probe.clone();
        if std::thread::Builder::new()
            .name("moiren-catalog-probe".to_owned())
            .spawn(move || {
                let probed = probe(&id, flow);
                let _ = sender.send((id, probed));
            })
            .is_err()
        {
            // Thread spawn failed: nothing will report for this row.
            continue;
        }
        spawned += 1;
    }
    drop(sender);
    let deadline = std::time::Instant::now() + probe_deadline();
    let mut snapshots = Vec::with_capacity(total);
    let mut received: Vec<String> = Vec::with_capacity(total);
    while received.len() < spawned {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            break;
        };
        match receiver.recv_timeout(remaining) {
            Ok((id, Ok(snapshot))) => {
                received.push(id);
                snapshots.push(snapshot);
            }
            Ok((id, Err(error))) => {
                let flow = ids
                    .iter()
                    .find(|(candidate, _)| *candidate == id)
                    .map(|(_, flow)| *flow)
                    .unwrap_or("render");
                received.push(id.clone());
                snapshots.push(bare_endpoint(
                    id,
                    flow,
                    vec![ApiFailure::new("endpoint probe", error)],
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    for (id, flow) in ids {
        if !received.contains(&id) {
            snapshots.push(not_responding(id, flow));
        }
    }
    snapshots
}

/// One endpoint's full probe: apartment-private COM on whichever thread runs
/// it, so a hung activation only ever blocks its own probe thread.
fn probe_endpoint(id: &str, flow: &'static str) -> Result<EndpointSnapshot> {
    unsafe {
        let _apartment = crate::win32_raii::Apartment::new()?;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDevice(&windows::core::HSTRING::from(id))?;
        endpoint(&device, flow)
    }
}

pub fn snapshot() -> Result<CatalogSnapshot> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let mut result = CatalogSnapshot {
            defaults: Vec::new(),
            endpoints: Vec::new(),
            errors: Vec::new(),
        };
        let mut ids: Vec<(String, &'static str)> = Vec::new();
        for (flow, name) in [(eRender, "render"), (eCapture, "capture")] {
            for (role, role_name) in [
                (eConsole, "console"),
                (eMultimedia, "multimedia"),
                (eCommunications, "communications"),
            ] {
                let id = record(
                    "default endpoint",
                    &mut result.errors,
                    enumerator.GetDefaultAudioEndpoint(flow, role),
                )
                .and_then(|device| {
                    record(
                        "default endpoint ID",
                        &mut result.errors,
                        device.GetId().and_then(take_string),
                    )
                });
                result.defaults.push(DefaultEndpoint {
                    flow: name,
                    role: role_name,
                    id,
                });
            }
            let devices = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
            for index in 0..devices.GetCount()? {
                // Only the cheap identity read stays on the caller thread;
                // the activation-prone probe work moves to bounded threads.
                if let Some(id) = record(
                    "endpoint enumeration",
                    &mut result.errors,
                    devices
                        .Item(index)
                        .and_then(|device| device.GetId().and_then(take_string)),
                ) {
                    ids.push((id, name));
                }
            }
        }
        result.endpoints = collect_endpoints(ids, std::sync::Arc::new(probe_endpoint));
        result.endpoints.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }
}

/// Observations only: never restore settings or attribute external changes to the probe.
/// Consumed by the probe-tool examples (`probe-tools` feature).
#[cfg(feature = "probe-tools")]
pub fn changes(before: &CatalogSnapshot, after: &CatalogSnapshot, pid: u32) -> Vec<String> {
    let mut result = Vec::new();
    if before.defaults != after.defaults {
        result.push("default endpoint roles changed".to_owned());
    }
    for old in &before.endpoints {
        let Some(new) = after.endpoints.iter().find(|new| new.id == old.id) else {
            result.push(format!(
                "endpoint disappeared: {}",
                old.name.as_deref().unwrap_or("unknown")
            ));
            continue;
        };
        if old.volume_scalar != new.volume_scalar || old.muted != new.muted {
            result.push(format!(
                "endpoint volume/mute changed: {}",
                old.name.as_deref().unwrap_or("unknown")
            ));
        }
        for old_session in old
            .sessions
            .iter()
            .filter(|session| session.process_id == pid)
        {
            match new
                .sessions
                .iter()
                .find(|session| session.instance_key == old_session.instance_key)
            {
                Some(new_session)
                    if old_session.volume_scalar == new_session.volume_scalar
                        && old_session.muted == new_session.muted => {}
                Some(_) => result.push(format!(
                    "target session volume/mute changed: {}",
                    old_session.instance_key
                )),
                None => result.push(format!(
                    "target session disappeared: {}",
                    old_session.instance_key
                )),
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn healthy(id: &str, flow: &'static str) -> Result<EndpointSnapshot> {
        Ok(bare_endpoint(id.to_owned(), flow, Vec::new()))
    }

    fn snapshot_row<'a>(snapshots: &'a [EndpointSnapshot], id: &str) -> &'a EndpointSnapshot {
        snapshots
            .iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing row for {id}"))
    }

    #[test]
    fn a_hung_endpoint_probe_returns_within_the_deadline_and_keeps_its_row() {
        let started = std::time::Instant::now();
        let snapshots = collect_endpoints(
            vec![
                ("{healthy}".into(), "render"),
                ("{wedged}".into(), "render"),
            ],
            std::sync::Arc::new(|id: &str, flow: &'static str| {
                if id == "{wedged}" {
                    // A probe that never settles, like an Activate stuck in
                    // the driver.
                    loop {
                        std::thread::sleep(Duration::from_secs(60));
                    }
                }
                healthy(id, flow)
            }),
        );
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "collection must not wait for the wedged probe"
        );
        let healthy_row = snapshot_row(&snapshots, "{healthy}");
        assert!(healthy_row.errors.is_empty());
        let wedged = snapshot_row(&snapshots, "{wedged}");
        assert_eq!(wedged.flow, "render");
        assert_eq!(wedged.errors.len(), 1);
        assert_eq!(wedged.errors[0].stage, "endpoint probe");
        assert!(wedged.errors[0].hresult.contains("no response"));
    }

    #[test]
    fn a_failing_probe_reports_the_api_failure_on_its_row() {
        let snapshots = collect_endpoints(
            vec![("{failing}".into(), "capture")],
            std::sync::Arc::new(|_id: &str, _flow: &'static str| {
                Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004005_u32 as i32, // E_FAIL: any recorded API failure shape
                )))
            }),
        );
        let row = snapshot_row(&snapshots, "{failing}");
        assert_eq!(row.flow, "capture");
        assert_eq!(row.errors.len(), 1);
        assert_eq!(row.errors[0].stage, "endpoint probe");
        assert_eq!(row.errors[0].hresult, "0x80004005");
    }

    #[test]
    fn spawn_failure_leaves_the_row_not_responding_instead_of_hanging() {
        // Zero spawnable ids cannot happen through snapshot(), but an empty
        // wave must return immediately rather than wait out the deadline.
        let started = std::time::Instant::now();
        let snapshots = collect_endpoints(Vec::new(), std::sync::Arc::new(healthy));
        assert!(snapshots.is_empty());
        assert!(started.elapsed() < Duration::from_millis(100));
    }
}
