//! W00 software-device management. Fixed namespace; never install a driver.
use crate::{
    catalog::{self, ApiFailure, CatalogSnapshot},
    owner::{Apartment, OwnedHandle, take_string},
};
use serde::Serialize;
use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{CONFIGRET, CR_NO_SUCH_DEVNODE, CR_SUCCESS},
            Enumeration::Pnp::{SWDeviceLifetimeHandle, SwDeviceGetLifetime, SwDeviceSetLifetime},
        },
        Foundation::{E_ACCESSDENIED, E_UNEXPECTED, HANDLE},
        Media::Audio::{DEVICE_STATEMASK_ALL, IMMDeviceEnumerator, MMDeviceEnumerator, eAll},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
    core::{HRESULT, Result},
};

mod devnode;
mod lifecycle;

pub use devnode::NodeState;
use devnode::{node_state, owned_identity, uninstall_owned, wait_absent};
use lifecycle::create;

const ENUMERATOR: &str = "MoirenW00";

fn unexpected() -> windows::core::Error {
    windows::core::Error::from_hresult(E_UNEXPECTED)
}
fn hr(value: HRESULT) -> String {
    format!("0x{:08X}", value.0 as u32)
}
fn cr(value: CONFIGRET) -> String {
    format!("0x{:08X}", value.0)
}

fn all_audio_ids() -> Result<Vec<String>> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let devices = unsafe {
        enumerator.EnumAudioEndpoints(
            eAll,
            windows::Win32::Media::Audio::DEVICE_STATE(DEVICE_STATEMASK_ALL),
        )?
    };
    let mut ids = Vec::new();
    for i in 0..unsafe { devices.GetCount()? } {
        ids.push(take_string(unsafe { devices.Item(i)?.GetId()? })?);
    }
    ids.sort();
    Ok(ids)
}
fn elevated() -> Result<bool> {
    let mut token = HANDLE::default();
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
    }
    let token = OwnedHandle(token);
    let mut elevation = TOKEN_ELEVATION::default();
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )?;
    }
    Ok(elevation.TokenIsElevated != 0)
}

#[derive(Serialize)]
pub struct Cycle {
    pub driver_required: bool,
    pub iteration: u32,
    pub requested_instance: String,
    pub status: &'static str,
    pub callback_hresult: Option<String>,
    pub callback_instance: Option<String>,
    pub lifetime: Option<i32>,
    pub duplicate_create_hresult: Option<String>,
    pub during: Option<NodeState>,
    pub audio_endpoints_added: Vec<String>,
    pub audio_endpoints_removed: Vec<String>,
    pub close_returned: bool,
    pub after_close: Option<NodeState>,
    pub phantom_after_close: Option<NodeState>,
    pub uninstall_succeeded: bool,
    pub reboot_required: Option<bool>,
    pub after_uninstall: Option<NodeState>,
    pub errors: Vec<ApiFailure>,
}
fn cycle(instance: &str, iteration: u32, driver_required: bool, audio_ids: &[String]) -> Cycle {
    let mut report = Cycle {
        driver_required,
        iteration,
        requested_instance: instance.to_owned(),
        status: "initializing",
        callback_hresult: None,
        callback_instance: None,
        lifetime: None,
        duplicate_create_hresult: None,
        during: None,
        audio_endpoints_added: Vec::new(),
        audio_endpoints_removed: Vec::new(),
        close_returned: false,
        after_close: None,
        phantom_after_close: None,
        uninstall_succeeded: false,
        reboot_required: None,
        after_uninstall: None,
        errors: Vec::new(),
    };
    let mut created = match create(instance, driver_required) {
        Ok(created) => created,
        Err(error) => {
            report.status = if error.code() == E_ACCESSDENIED {
                "permission_denied"
            } else {
                "create_failed"
            };
            report.errors.push(ApiFailure::new("SwDeviceCreate", error));
            return report;
        }
    };
    let mut stage = "create callback wait";
    let result: Result<()> = (|| {
        let completion = created.wait()?;
        report.callback_hresult = Some(hr(completion.result));
        report.callback_instance = completion.id;
        completion.result.ok()?;
        let actual = report.callback_instance.as_deref().ok_or_else(unexpected)?;
        if !owned_identity(actual, instance) {
            return Err(unexpected());
        }
        stage = "GetLifetime / SetLifetime(Handle)";
        let handle = created.handle()?;
        report.lifetime = Some(unsafe { SwDeviceGetLifetime(handle)? }.0);
        unsafe {
            SwDeviceSetLifetime(handle, SWDeviceLifetimeHandle)?;
        }
        stage = "duplicate SwDeviceCreate";
        match create(instance, driver_required) {
            Err(error) => report.duplicate_create_hresult = Some(hr(error.code())),
            Ok(mut duplicate) => {
                report.duplicate_create_hresult = Some(hr(HRESULT(0)));
                duplicate.close();
                return Err(unexpected());
            }
        }
        stage = "PnP / driver binding snapshot";
        report.during = Some(node_state(actual, false));
        stage = "audio endpoint enumeration during create";
        let during_audio_ids = all_audio_ids()?;
        report.audio_endpoints_added = during_audio_ids
            .iter()
            .filter(|id| !audio_ids.contains(id))
            .cloned()
            .collect();
        report.audio_endpoints_removed = audio_ids
            .iter()
            .filter(|id| !during_audio_ids.contains(id))
            .cloned()
            .collect();
        Ok(())
    })();
    if let Err(error) = result {
        report.errors.push(ApiFailure::new(stage, error));
    }
    created.close();
    report.close_returned = true;
    // Close joins callbacks; recover an identity even if the bounded wait timed out.
    if report.callback_instance.is_none()
        && let Some(completion) = created.completion()
    {
        report.callback_hresult = Some(hr(completion.result));
        report.callback_instance = completion.id;
    }
    if let Some(actual) = report.callback_instance.as_deref() {
        report.after_close = Some(wait_absent(actual, false));
        report.phantom_after_close = Some(node_state(actual, true));
        match uninstall_owned(actual, instance) {
            Ok(reboot) => {
                report.uninstall_succeeded = true;
                report.reboot_required = Some(reboot);
            }
            Err(error) => report
                .errors
                .push(ApiFailure::new("DiUninstallDevice(owned instance)", error)),
        }
        report.after_uninstall = Some(wait_absent(actual, true));
    }
    report.status = if report.errors.is_empty()
        && report.lifetime == Some(SWDeviceLifetimeHandle.0)
        && report.during.as_ref().is_some_and(|s| {
            s.located && s.status_configret.as_deref() == Some(cr(CR_SUCCESS).as_str())
        })
        && report.audio_endpoints_added.is_empty()
        && report.audio_endpoints_removed.is_empty()
        && report
            .after_close
            .as_ref()
            .is_some_and(|s| s.locate_configret == cr(CR_NO_SUCH_DEVNODE))
        && report.uninstall_succeeded
        && report.reboot_required == Some(false)
        && report
            .after_uninstall
            .as_ref()
            .is_some_and(|s| s.locate_configret == cr(CR_NO_SUCH_DEVNODE))
    {
        "completed"
    } else {
        "incomplete"
    };
    report
}

#[derive(Serialize)]
pub struct Report {
    pub scope: &'static str,
    pub elevated: bool,
    pub before: CatalogSnapshot,
    pub audio_ids_before: Vec<String>,
    pub cycles: Vec<Cycle>,
    pub after: CatalogSnapshot,
    pub audio_ids_after: Vec<String>,
    pub observed_state_changes: Vec<String>,
}
pub fn run(iterations: u32, observe_pid: Option<u32>) -> anyhow::Result<Report> {
    if !(1..=10).contains(&iterations) {
        anyhow::bail!("--iterations must be within 1..=10, got {iterations}");
    }
    let _apartment = Apartment::new()?;
    let before = catalog::snapshot()?;
    let audio_ids_before = all_audio_ids()?;
    let elevated = elevated()?;
    let mut cycles = Vec::new();
    for driver_required in [false, true] {
        let instance = format!("W00-{:?}", unsafe { CoCreateGuid()? });
        for iteration in 1..=iterations {
            let report = cycle(&instance, iteration, driver_required, &audio_ids_before);
            let completed = report.status == "completed";
            cycles.push(report);
            if !completed {
                break;
            }
        }
    }
    let after = catalog::snapshot()?;
    let audio_ids_after = all_audio_ids()?;
    let observed_state_changes = catalog::changes(&before, &after, observe_pid.unwrap_or(0));
    Ok(Report {
        scope: "PnP software nodes; no audio driver installation or PCM endpoint implementation",
        elevated,
        before,
        audio_ids_before,
        cycles,
        after,
        audio_ids_after,
        observed_state_changes,
    })
}
