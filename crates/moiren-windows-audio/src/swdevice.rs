//! W00 software-device management. Fixed namespace; never install a driver.
use crate::{
    catalog::{self, ApiFailure, CatalogSnapshot},
    owner::{Apartment, OwnedHandle, take_string},
};
use serde::Serialize;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Devices::{DeviceAndDriverInstallation::*, Enumeration::Pnp::*, Properties::*},
        Foundation::{E_ACCESSDENIED, E_UNEXPECTED, HANDLE, HWND, WAIT_OBJECT_0},
        Media::Audio::{DEVICE_STATEMASK_ALL, IMMDeviceEnumerator, MMDeviceEnumerator, eAll},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
            Threading::{GetCurrentProcess, OpenProcessToken, SetEvent, WaitForSingleObject},
        },
    },
    core::{HRESULT, HSTRING, PCWSTR, Result, w},
};

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

// This is deliberately stricter than the API's possible name decoration. A
// surprising callback identity is recorded, but never passed to uninstallation.
fn owned_identity(actual: &str, instance: &str) -> bool {
    actual.eq_ignore_ascii_case(&format!("SWD\\{ENUMERATOR}\\{instance}"))
        && instance.starts_with("W00-")
        && !instance.contains(['\\', '/', '\0'])
}

#[derive(Clone)]
struct Completion {
    result: HRESULT,
    id: Option<String>,
}
struct Context {
    event: OwnedHandle,
    completion: Mutex<Option<Completion>>,
    panicked: AtomicBool,
}
unsafe extern "system" fn completed(
    _: HSWDEVICE,
    result: HRESULT,
    context: *const c_void,
    id: PCWSTR,
) {
    if context.is_null() {
        return;
    }
    // The box stays alive through SwDeviceClose, which joins late callbacks.
    let context = unsafe { &*context.cast::<Context>() };
    if catch_unwind(AssertUnwindSafe(|| {
        let id = if id.is_null() {
            None
        } else {
            unsafe { id.to_string().ok() }
        };
        *context.completion.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Completion { result, id });
    }))
    .is_err()
    {
        context.panicked.store(true, Ordering::Release);
    }
    let _ = unsafe { SetEvent(context.event.0) };
}

struct SwLease(Option<HSWDEVICE>);
impl SwLease {
    fn close(&mut self) {
        if let Some(handle) = self.0.take() {
            unsafe { SwDeviceClose(handle) };
        }
    }
}
impl Drop for SwLease {
    fn drop(&mut self) {
        self.close();
    }
}
struct Created {
    // Struct fields drop in declaration order: close/join before freeing context.
    lease: SwLease,
    context: Box<Context>,
}
impl Created {
    fn wait(&self) -> Result<Completion> {
        if unsafe { WaitForSingleObject(self.context.event.0, 5_000) } != WAIT_OBJECT_0 {
            return Err(windows::core::Error::from_hresult(HRESULT::from_win32(
                1460,
            )));
        }
        if self.context.panicked.load(Ordering::Acquire) {
            return Err(unexpected());
        }
        self.context
            .completion
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(unexpected)
    }
}

fn create(instance: &str, driver_required: bool) -> Result<Created> {
    let context = Box::new(Context {
        event: OwnedHandle::event()?,
        completion: Mutex::new(None),
        panicked: AtomicBool::new(false),
    });
    let instance = HSTRING::from(instance);
    let info = SW_DEVICE_CREATE_INFO {
        cbSize: size_of::<SW_DEVICE_CREATE_INFO>() as u32,
        pszInstanceId: PCWSTR(instance.as_ptr()),
        // No installed audio package matches this W00-only hardware identity.
        pszzHardwareIds: w!("Moiren\\W00NoDriver\0"),
        pszzCompatibleIds: w!("Moiren\\W00Probe\0"),
        CapabilityFlags: (SWDeviceCapabilitiesRemovable.0
            | SWDeviceCapabilitiesSilentInstall.0
            | SWDeviceCapabilitiesNoDisplayInUI.0
            | if driver_required {
                SWDeviceCapabilitiesDriverRequired.0
            } else {
                0
            }) as u32,
        pszDeviceDescription: w!("Moiren W00 temporary lifecycle probe - no audio driver"),
        ..Default::default()
    };
    let handle = unsafe {
        SwDeviceCreate(
            &HSTRING::from(ENUMERATOR),
            w!("HTREE\\ROOT\\0"),
            &info,
            None,
            Some(completed),
            Some((&*context as *const Context).cast()),
        )?
    };
    Ok(Created {
        lease: SwLease(Some(handle)),
        context,
    })
}

#[derive(Default, Serialize)]
pub struct NodeState {
    pub locate_configret: String,
    pub include_nonpresent: bool,
    pub located: bool,
    pub status_configret: Option<String>,
    pub status_flags: Option<u32>,
    pub problem_code: Option<u32>,
    pub driver_inf_configret: Option<String>,
    pub driver_inf_path: Option<String>,
}
fn node_state(id: &str, phantom: bool) -> NodeState {
    let mut devinst = 0;
    let result = unsafe {
        CM_Locate_DevNodeW(
            &mut devinst,
            &HSTRING::from(id),
            if phantom {
                CM_LOCATE_DEVNODE_PHANTOM
            } else {
                CM_LOCATE_DEVNODE_NORMAL
            },
        )
    };
    let mut state = NodeState {
        locate_configret: cr(result),
        include_nonpresent: phantom,
        located: result == CR_SUCCESS,
        ..Default::default()
    };
    if result == CR_SUCCESS {
        let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS::default(), CM_PROB::default());
        let result = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, devinst, 0) };
        state.status_configret = Some(cr(result));
        if result == CR_SUCCESS {
            state.status_flags = Some(status.0);
            state.problem_code = Some(problem.0);
        }
        let mut property_type = DEVPROPTYPE::default();
        let mut length = 0;
        let result = unsafe {
            CM_Get_DevNode_PropertyW(
                devinst,
                &DEVPKEY_Device_DriverInfPath,
                &mut property_type,
                None,
                &mut length,
                0,
            )
        };
        state.driver_inf_configret = Some(cr(result));
        if result == CR_BUFFER_SMALL && length <= 65_536 {
            let mut data = vec![0u8; length as usize];
            let result = unsafe {
                CM_Get_DevNode_PropertyW(
                    devinst,
                    &DEVPKEY_Device_DriverInfPath,
                    &mut property_type,
                    Some(data.as_mut_ptr()),
                    &mut length,
                    0,
                )
            };
            state.driver_inf_configret = Some(cr(result));
            if result == CR_SUCCESS
                && property_type == DEVPROP_TYPE_STRING
                && length as usize <= data.len()
            {
                let text: Vec<u16> = data[..length as usize]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .take_while(|&v| v != 0)
                    .collect();
                state.driver_inf_path = Some(String::from_utf16_lossy(&text));
            }
        }
    }
    state
}
fn wait_absent(id: &str, phantom: bool) -> NodeState {
    let start = Instant::now();
    loop {
        let state = node_state(id, phantom);
        if state.locate_configret == cr(CR_NO_SUCH_DEVNODE)
            || start.elapsed() >= Duration::from_secs(5)
        {
            return state;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
struct DeviceInfo(HDEVINFO);
impl Drop for DeviceInfo {
    fn drop(&mut self) {
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}
fn uninstall_owned(actual: &str, instance: &str) -> Result<bool> {
    if !owned_identity(actual, instance) {
        return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
    }
    let info = DeviceInfo(unsafe { SetupDiCreateDeviceInfoList(None, None)? });
    let mut data = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    unsafe {
        SetupDiOpenDeviceInfoW(info.0, &HSTRING::from(actual), None, 0, Some(&mut data))?;
    }
    let mut reboot = windows::core::BOOL::default();
    unsafe {
        DiUninstallDevice(HWND::default(), info.0, &data, 0, Some(&mut reboot))?;
    }
    Ok(reboot.as_bool())
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
        let handle = created.lease.0.ok_or_else(unexpected)?;
        report.lifetime = Some(unsafe { SwDeviceGetLifetime(handle)? }.0);
        unsafe {
            SwDeviceSetLifetime(handle, SWDeviceLifetimeHandle)?;
        }
        stage = "duplicate SwDeviceCreate";
        match create(instance, driver_required) {
            Err(error) => report.duplicate_create_hresult = Some(hr(error.code())),
            Ok(mut duplicate) => {
                report.duplicate_create_hresult = Some(hr(HRESULT(0)));
                duplicate.lease.close();
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
    created.lease.close();
    report.close_returned = true;
    // Close joins callbacks; recover an identity even if the bounded wait timed out.
    if report.callback_instance.is_none()
        && let Some(completion) = created
            .context
            .completion
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
pub fn run(iterations: u32, observe_pid: Option<u32>) -> Result<Report> {
    if !(1..=10).contains(&iterations) {
        return Err(unexpected());
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uninstall_guard_accepts_only_exact_created_identity() {
        assert!(owned_identity("SWD\\MOIRENW00\\W00-abc", "W00-abc"));
        for id in [
            "ROOT\\MEDIA\\0000",
            "SWD\\MMDEVAPI\\W00-abc",
            "SWD\\MOIRENW000\\W00-abc",
            "SWD\\MOIRENW00\\W00-other",
        ] {
            assert!(!owned_identity(id, "W00-abc"));
        }
        assert!(!owned_identity("SWD\\MOIRENW00\\other", "other"));
        assert!(!owned_identity("SWD\\MOIRENW00\\W00-a\\b", "W00-a\\b"));
    }
    #[test]
    fn callback_records_identity_before_create_returns_without_real_device() {
        let context = Context {
            event: OwnedHandle::event().unwrap(),
            completion: Mutex::new(None),
            panicked: AtomicBool::new(false),
        };
        unsafe {
            completed(
                HSWDEVICE::default(),
                HRESULT(0),
                (&context as *const Context).cast(),
                w!("SWD\\MOIRENW00\\W00-abc"),
            );
        }
        assert_eq!(
            unsafe { WaitForSingleObject(context.event.0, 100) },
            WAIT_OBJECT_0
        );
        assert_eq!(
            context
                .completion
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .id
                .as_deref(),
            Some("SWD\\MOIRENW00\\W00-abc")
        );
    }
}
