use crate::owner::{Process, TaskMemory, take_string};
use serde::Serialize;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::{Endpoints::IAudioEndpointVolume, *},
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ},
            Variant::VT_LPWSTR,
        },
    },
    core::{Interface, Result},
};

#[derive(Serialize)]
pub struct ApiFailure {
    pub stage: String,
    pub hresult: String,
}
impl ApiFailure {
    pub fn new(stage: &str, error: windows::core::Error) -> Self {
        Self {
            stage: stage.to_owned(),
            hresult: format!("0x{:08X}", error.code().0 as u32),
        }
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

fn friendly_name(device: &IMMDevice) -> Result<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        // windows 0.62 owns this returned PROPVARIANT and clears it on Drop.
        let property = store.GetValue(&PKEY_Device_FriendlyName)?;
        let value = &property.Anonymous.Anonymous;
        if value.vt != VT_LPWSTR {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_UNEXPECTED,
            ));
        }
        Ok(value.Anonymous.pwszVal.to_string()?)
    }
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

pub fn snapshot() -> Result<CatalogSnapshot> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let mut result = CatalogSnapshot {
            defaults: Vec::new(),
            endpoints: Vec::new(),
            errors: Vec::new(),
        };
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
                if let Some(item) = record(
                    "endpoint snapshot",
                    &mut result.errors,
                    devices
                        .Item(index)
                        .and_then(|device| endpoint(&device, name)),
                ) {
                    result.endpoints.push(item);
                }
            }
        }
        result.endpoints.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }
}

/// Render selector snapshot: no capture or default-role query. A missing
/// microphone cannot hide usable outputs; failures stay local to endpoints.
pub fn render_snapshot() -> Result<CatalogSnapshot> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let mut result = CatalogSnapshot {
            defaults: Vec::new(),
            endpoints: Vec::new(),
            errors: Vec::new(),
        };
        for index in 0..devices.GetCount()? {
            if let Some(item) = record(
                "render endpoint snapshot",
                &mut result.errors,
                devices
                    .Item(index)
                    .and_then(|device| endpoint(&device, "render")),
            ) {
                result.endpoints.push(item);
            }
        }
        result.endpoints.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }
}

/// Observations only: never restore settings or attribute external changes to the probe.
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
