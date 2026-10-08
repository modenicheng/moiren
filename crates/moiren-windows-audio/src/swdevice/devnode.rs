//! Only an exact probe-owned PnP identity may reach the uninstall API.
use super::{ENUMERATOR, cr};
use serde::Serialize;
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_DEVNODE_STATUS_FLAGS, CM_Get_DevNode_PropertyW, CM_Get_DevNode_Status,
                CM_LOCATE_DEVNODE_NORMAL, CM_LOCATE_DEVNODE_PHANTOM, CM_Locate_DevNodeW, CM_PROB,
                CR_BUFFER_SMALL, CR_NO_SUCH_DEVNODE, CR_SUCCESS, DiUninstallDevice, HDEVINFO,
                SP_DEVINFO_DATA, SetupDiCreateDeviceInfoList, SetupDiDestroyDeviceInfoList,
                SetupDiOpenDeviceInfoW,
            },
            Properties::{DEVPKEY_Device_DriverInfPath, DEVPROP_TYPE_STRING, DEVPROPTYPE},
        },
        Foundation::{E_ACCESSDENIED, HWND},
    },
    core::{HSTRING, Result},
};

// This is deliberately stricter than the API's possible name decoration. A
// surprising callback identity is recorded, but never passed to uninstallation.
pub(super) fn owned_identity(actual: &str, instance: &str) -> bool {
    actual.eq_ignore_ascii_case(&format!("SWD\\{ENUMERATOR}\\{instance}"))
        && instance.starts_with("W00-")
        && !instance.contains(['\\', '/', '\0'])
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
pub(super) fn node_state(id: &str, phantom: bool) -> NodeState {
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
pub(super) fn wait_absent(id: &str, phantom: bool) -> NodeState {
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
pub(super) fn uninstall_owned(actual: &str, instance: &str) -> Result<bool> {
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
}
