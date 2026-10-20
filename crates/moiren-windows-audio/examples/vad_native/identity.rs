//! Read-only MMDevice -> topology adapter -> PnP parent identity checks.
use crate::validation::Candidate;
use anyhow::{Context, ensure};
use moiren_windows_audio::vad::{DeviceInterface, Intent};
use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_Get_Device_ID_Size, CM_Get_Device_IDW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL,
                CM_Locate_DevNodeW, CONFIGRET, CR_SUCCESS, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
                SP_DEVINFO_DATA, SetupDiCreateDeviceInfoList, SetupDiDestroyDeviceInfoList,
                SetupDiGetDeviceInterfaceDetailW, SetupDiOpenDeviceInterfaceW,
            },
            FunctionDiscovery::PKEY_Device_InstanceId,
        },
        Foundation::ERROR_INSUFFICIENT_BUFFER,
        Media::Audio::{
            DEVICE_STATE_ACTIVE, IDeviceTopology, IMMDevice, IMMDeviceEnumerator, eCapture, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoTaskMemFree, STGM_READ},
            Variant::VT_LPWSTR,
        },
    },
    core::{HSTRING, PWSTR},
};

struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        // SAFETY: the wrapper exclusively owns this successful SetupAPI set.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

struct TaskString(PWSTR);
impl Drop for TaskString {
    fn drop(&mut self) {
        // SAFETY: all callers transfer a COM task-allocated string exactly once.
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

pub(super) fn take_string(pointer: PWSTR) -> anyhow::Result<String> {
    let owned = TaskString(pointer);
    ensure!(!owned.0.is_null(), "COM returned a null identity");
    // SAFETY: the successful COM method returned a NUL-terminated allocation,
    // retained by owned until the strict UTF-16 copy has finished.
    Ok(String::from_utf16(unsafe { owned.0.as_wide() })?)
}

fn config(result: CONFIGRET, stage: &str) -> anyhow::Result<()> {
    ensure!(result == CR_SUCCESS, "{stage}: CONFIGRET={}", result.0);
    Ok(())
}

fn instance(node: u32) -> anyhow::Result<String> {
    let mut length = 0;
    // SAFETY: this only obtains a length for a live Configuration Manager node.
    config(
        unsafe { CM_Get_Device_ID_Size(&mut length, node, 0) },
        "CM_Get_Device_ID_Size",
    )?;
    ensure!(
        (1..=4096).contains(&length),
        "PnP instance ID length out of bounds"
    );
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: writable buffer includes room for the terminating NUL.
    config(
        unsafe { CM_Get_Device_IDW(node, &mut buffer, 0) },
        "CM_Get_Device_IDW",
    )?;
    ensure!(buffer[length as usize] == 0, "unterminated PnP instance ID");
    Ok(String::from_utf16(&buffer[..length as usize])?)
}

pub(super) fn root_instance(interface: &DeviceInterface) -> anyhow::Result<String> {
    let path = interface.display_path();
    // The public display form is lossy for arbitrary UTF-16. This product's
    // interface is ASCII; refuse anything else rather than open a substituted path.
    ensure!(
        path.is_ascii(),
        "control interface cannot be represented losslessly"
    );
    // SAFETY: new empty owned SetupAPI set, no class filter or parent window.
    let set = DeviceSet(unsafe { SetupDiCreateDeviceInfoList(None, None)? });
    let mut data = SP_DEVICE_INTERFACE_DATA {
        cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
        ..Default::default()
    };
    // SAFETY: live set, NUL-terminated path and initialized output structure.
    unsafe {
        SetupDiOpenDeviceInterfaceW(set.0, &HSTRING::from(path), 0, Some(&mut data))?;
    }
    let mut info = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    // A size query also returns the associated devnode; no path-tail allocation
    // is needed. ERROR_INSUFFICIENT_BUFFER is expected for the absent detail.
    let result =
        unsafe { SetupDiGetDeviceInterfaceDetailW(set.0, &data, None, 0, None, Some(&mut info)) };
    ensure!(
        result.is_err_and(|error| error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult()),
        "unexpected SetupDiGetDeviceInterfaceDetailW identity result"
    );
    instance(info.DevInst)
}

pub(super) fn candidate(
    enumerator: &IMMDeviceEnumerator,
    device: &IMMDevice,
    intent: Intent,
) -> anyhow::Result<Candidate> {
    // SAFETY: all COM objects are used in their owning apartment. The topology
    // connector supplies an opaque adapter ID for GetDevice, never string parsing.
    unsafe {
        let id = take_string(device.GetId()?)?;
        let topology: IDeviceTopology = device.Activate(CLSCTX_ALL, None)?;
        ensure!(
            topology.GetConnectorCount()? == 1,
            "endpoint has no unique connector"
        );
        let adapter_id = take_string(topology.GetConnector(0)?.GetDeviceIdConnectedTo()?)?;
        let adapter = enumerator.GetDevice(&HSTRING::from(adapter_id))?;
        let properties = adapter.OpenPropertyStore(STGM_READ)?;
        let property = properties.GetValue(&PKEY_Device_InstanceId)?;
        // PROPVARIANT owns its contents and clears them after this copy.
        ensure!(
            property.Anonymous.Anonymous.vt == VT_LPWSTR,
            "adapter lacks a string PnP instance identity"
        );
        let pointer = property.Anonymous.Anonymous.Anonymous.pwszVal;
        ensure!(!pointer.is_null(), "adapter returned a null PnP identity");
        let raw_instance = String::from_utf16(pointer.as_wide())?;
        let mut node = 0;
        config(
            CM_Locate_DevNodeW(
                &mut node,
                &HSTRING::from(&raw_instance),
                CM_LOCATE_DEVNODE_NORMAL,
            ),
            "CM_Locate_DevNodeW(adapter)",
        )?;
        let mut parent = 0;
        config(
            CM_Get_Parent(&mut parent, node, 0),
            "CM_Get_Parent(adapter)",
        )?;
        Ok(Candidate {
            id,
            raw_instance,
            parent_instance: instance(parent)?,
            intent,
        })
    }
}

pub(super) fn candidates(
    enumerator: &IMMDeviceEnumerator,
) -> anyhow::Result<(Vec<Candidate>, Vec<String>)> {
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (flow, intent) in [(eRender, Intent::Playback), (eCapture, Intent::Microphone)] {
        // SAFETY: enumeration is read-only and apartment-local.
        let devices = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
        for index in 0..unsafe { devices.GetCount()? } {
            let device = unsafe { devices.Item(index)? };
            match candidate(enumerator, &device, intent)
                .with_context(|| format!("topology identity {intent:?} index={index}"))
            {
                Ok(row) => rows.push(row),
                Err(error) => {
                    // An unrelated endpoint may not support topology identity.
                    // Keep bounded diagnostics; only exact owned matches can pass.
                    if errors.len() < 8 {
                        errors.push(format!("{error:#}"));
                    }
                }
            }
        }
    }
    Ok((rows, errors))
}
