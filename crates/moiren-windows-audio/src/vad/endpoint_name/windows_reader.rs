//! Bounded read-only MMDevice/topology/PnP identity resolution.

use super::{Candidate, EndpointNameReader, ObserverClock, select};
use crate::{owner::Apartment, vad::DeviceInterface};
use std::mem::size_of;
use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_Get_Device_ID_Size, CM_Get_Device_IDW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL,
                CM_Locate_DevNodeW, CONFIGRET, CR_SUCCESS, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
                SP_DEVINFO_DATA, SetupDiCreateDeviceInfoList, SetupDiDestroyDeviceInfoList,
                SetupDiGetDeviceInterfaceDetailW, SetupDiOpenDeviceInterfaceW,
            },
            FunctionDiscovery::{PKEY_Device_FriendlyName, PKEY_Device_InstanceId},
        },
        Foundation::PROPERTYKEY,
        Media::Audio::{
            DEVICE_STATE, DEVICE_STATEMASK_ALL, IDeviceTopology, IMMDevice, IMMDeviceEnumerator,
            MMDeviceEnumerator, eCapture, eRender,
        },
        System::{
            Com::{
                CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ,
                StructuredStorage::PROPVARIANT,
            },
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            Variant::VT_LPWSTR,
        },
        UI::Shell::PropertiesSystem::IPropertyStore,
    },
    core::{Error as WindowsError, HSTRING, PWSTR},
};

const MAX_INSTANCE_ID_UNITS: usize = 4096;
const MAX_MMDEVICE_ENDPOINTS: u32 = 512;
const MAX_PROPERTY_STRING_UNITS: usize = 4096;

struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns the successful SetupAPI set.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

struct TaskString(PWSTR);

impl Drop for TaskString {
    fn drop(&mut self) {
        // SAFETY: every constructor wraps exactly one COM task allocation.
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

struct OwnedProperty(PROPVARIANT);

pub struct WindowsEndpointNameReader {
    interface: DeviceInterface,
}

impl WindowsEndpointNameReader {
    pub fn new(interface: DeviceInterface) -> Self {
        Self { interface }
    }
}

impl EndpointNameReader for WindowsEndpointNameReader {
    fn read(
        &mut self,
        requests: &[(
            moiren_engine::node::VirtualEndpointId,
            super::super::StableEndpointId,
            super::super::Intent,
        )],
    ) -> Vec<Result<(String, String), String>> {
        let _apartment = match Apartment::new() {
            Ok(apartment) => apartment,
            Err(error) => {
                let error = hresult("CoInitializeEx(name observation)", &error);
                return requests.iter().map(|_| Err(error.clone())).collect();
            }
        };
        let enumerator: IMMDeviceEnumerator =
            match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                Ok(enumerator) => enumerator,
                Err(error) => {
                    let error = hresult("CoCreateInstance(MMDeviceEnumerator)", &error);
                    return requests.iter().map(|_| Err(error.clone())).collect();
                }
            };
        let root = match root_instance(&self.interface) {
            Ok(root) => root,
            Err(error) => return requests.iter().map(|_| Err(error.clone())).collect(),
        };
        let (candidates, candidate_errors) = match enumerate_candidates(&enumerator) {
            Ok(result) => result,
            Err(error) => return requests.iter().map(|_| Err(error.clone())).collect(),
        };

        requests
            .iter()
            .map(|(_, stable_id, intent)| {
                read_name(
                    &enumerator,
                    &candidates,
                    &candidate_errors,
                    *stable_id,
                    *intent,
                    &root,
                )
            })
            .collect()
    }
}

pub(crate) struct WindowsObserverClock {
    frequency: Option<u64>,
    origin: std::time::Instant,
}

impl Default for WindowsObserverClock {
    fn default() -> Self {
        Self::new()
    }
}

struct CandidateError {
    device: IMMDevice,
    intent: super::super::Intent,
    index: u32,
    error: String,
}

fn enumerate_candidates(
    enumerator: &IMMDeviceEnumerator,
) -> Result<(Vec<super::Candidate>, Vec<CandidateError>), String> {
    let mut candidates = Vec::new();
    let mut candidate_errors = Vec::new();

    for (flow, intent) in [
        (eRender, super::super::Intent::Playback),
        (eCapture, super::super::Intent::Microphone),
    ] {
        // A VAD MMDevice may be present but inactive between recreation and a
        // client opening it. Enumerate all states, then require the exact root,
        // stable ID and flow below instead of treating Active as existence.
        let devices =
            unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE(DEVICE_STATEMASK_ALL)) }
                .map_err(|error| hresult(&format!("EnumAudioEndpoints({intent:?})"), &error))?;
        let count = unsafe { devices.GetCount() }.map_err(|error| {
            hresult(
                &format!("IMMDeviceCollection::GetCount({intent:?})"),
                &error,
            )
        })?;
        if count > MAX_MMDEVICE_ENDPOINTS {
            return Err(format!(
                "MMDevice endpoint count out of bounds: {count} > {MAX_MMDEVICE_ENDPOINTS}"
            ));
        }

        for index in 0..count {
            let device = unsafe { devices.Item(index) }
                .map_err(|error| hresult(&format!("IMMDeviceCollection::Item({index})"), &error))?;
            match candidate(enumerator, &device, intent) {
                Ok(candidate) => candidates.push(candidate),
                Err(error) => candidate_errors.push(CandidateError {
                    device,
                    intent,
                    index,
                    error,
                }),
            }
        }
    }

    Ok((candidates, candidate_errors))
}

fn read_name(
    enumerator: &IMMDeviceEnumerator,
    candidates: &[super::Candidate],
    candidate_errors: &[CandidateError],
    stable_id: super::super::StableEndpointId,
    intent: super::super::Intent,
    root: &str,
) -> Result<(String, String), String> {
    // Attribute enumeration failures only when ownership is confirmed. A hint
    // that itself fails (for example a removed endpoint whose connected-to
    // device no longer resolves through GetDevice) stays undecidable: reporting
    // the hint's own HRESULT here would mask both the candidate error and the
    // exact-endpoint name read below with an error about the attribution path.
    let hints: Vec<(u32, String, super::CandidateOwnership)> = candidate_errors
        .iter()
        .filter(|candidate_error| candidate_error.intent == intent)
        .map(|candidate_error| {
            let ownership =
                match owned_identity_hint(enumerator, &candidate_error.device, stable_id) {
                    Ok(owned) if owned => super::CandidateOwnership::Owned,
                    Ok(_) => super::CandidateOwnership::NotOwned,
                    Err(_) => super::CandidateOwnership::Undecidable,
                };
            (
                candidate_error.index,
                candidate_error.error.clone(),
                ownership,
            )
        })
        .collect();
    let owned_candidate_errors =
        super::owned_candidate_errors(&hints, super::MAX_REPORTED_OWNED_ERRORS);
    if !owned_candidate_errors.is_empty() {
        return Err(owned_candidate_errors.join("; "));
    }
    let selected = select(candidates, stable_id, intent, root)?;
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(selected.mmdevice_id.as_str())) }
        .map_err(|error| hresult("IMMDeviceEnumerator::GetDevice(exact ID)", &error))?;
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }
        .map_err(|error| hresult("IMMDevice::OpenPropertyStore(exact endpoint)", &error))?;
    let name = property_string(&store, &PKEY_Device_FriendlyName)
        .map_err(|error| format!("PKEY_Device_FriendlyName(exact endpoint): {error}"))?;
    Ok((selected.mmdevice_id.clone(), name))
}

fn instance_has_stable_suffix(instance: &str, stable_id: super::super::StableEndpointId) -> bool {
    let stable_text = stable_id.to_string();
    instance
        .rsplit('&')
        .next()
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&stable_text))
}

fn owned_identity_hint(
    enumerator: &IMMDeviceEnumerator,
    device: &IMMDevice,
    stable_id: super::super::StableEndpointId,
) -> Result<bool, String> {
    let topology: IDeviceTopology = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| hresult("IMMDevice::Activate(error attribution)", &error))?;
    let count = unsafe { topology.GetConnectorCount() }.map_err(|error| {
        hresult(
            "IDeviceTopology::GetConnectorCount(error attribution)",
            &error,
        )
    })?;
    if count != 1 {
        return Ok(false);
    }
    let connector = unsafe { topology.GetConnector(0) }
        .map_err(|error| hresult("IDeviceTopology::GetConnector(error attribution)", &error))?;
    let adapter_id = take_string(
        unsafe { connector.GetDeviceIdConnectedTo() }.map_err(|error| {
            hresult(
                "IConnector::GetDeviceIdConnectedTo(error attribution)",
                &error,
            )
        })?,
        "IConnector::GetDeviceIdConnectedTo(error attribution)",
    )?;
    let adapter = unsafe { enumerator.GetDevice(&HSTRING::from(adapter_id)) }
        .map_err(|error| hresult("GetDevice(error attribution)", &error))?;
    let store = unsafe { adapter.OpenPropertyStore(STGM_READ) }
        .map_err(|error| hresult("OpenPropertyStore(error attribution)", &error))?;
    let raw_instance = property_string(&store, &PKEY_Device_InstanceId)
        .map_err(|error| format!("PKEY_Device_InstanceId(error attribution): {error}"))?;
    Ok(instance_has_stable_suffix(&raw_instance, stable_id)
        && raw_instance
            .rsplit_once('\\')
            .is_some_and(|(hardware_id, _)| hardware_id.eq_ignore_ascii_case(r"MOIREN\VAD_RAW")))
}

fn candidate(
    enumerator: &IMMDeviceEnumerator,
    device: &IMMDevice,
    intent: super::super::Intent,
) -> Result<super::Candidate, String> {
    let id = take_string(
        unsafe { device.GetId() }.map_err(|error| hresult("IMMDevice::GetId", &error))?,
        "IMMDevice::GetId",
    )?;
    let topology: IDeviceTopology = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| hresult("IMMDevice::Activate(IDeviceTopology)", &error))?;
    let connectors = unsafe { topology.GetConnectorCount() }
        .map_err(|error| hresult("IDeviceTopology::GetConnectorCount", &error))?;
    if connectors != 1 {
        return Err(format!(
            "topology connector count is {connectors}, expected 1"
        ));
    }
    let connector = unsafe { topology.GetConnector(0) }
        .map_err(|error| hresult("IDeviceTopology::GetConnector(0)", &error))?;
    let adapter_id = take_string(
        unsafe { connector.GetDeviceIdConnectedTo() }
            .map_err(|error| hresult("IConnector::GetDeviceIdConnectedTo", &error))?,
        "IConnector::GetDeviceIdConnectedTo",
    )?;
    let adapter = unsafe { enumerator.GetDevice(&HSTRING::from(adapter_id)) }
        .map_err(|error| hresult("IMMDeviceEnumerator::GetDevice(opaque adapter ID)", &error))?;
    let store = unsafe { adapter.OpenPropertyStore(STGM_READ) }
        .map_err(|error| hresult("IMMDevice::OpenPropertyStore(adapter)", &error))?;
    let raw_instance = property_string(&store, &PKEY_Device_InstanceId)
        .map_err(|error| format!("PKEY_Device_InstanceId(adapter): {error}"))?;
    let adapter_node = locate_devnode(&raw_instance, "CM_Locate_DevNodeW(adapter)")?;
    let mut parent = 0;
    config(
        unsafe { CM_Get_Parent(&mut parent, adapter_node, 0) },
        "CM_Get_Parent(adapter)",
    )?;
    Ok(Candidate {
        mmdevice_id: id,
        raw_instance,
        root_instance: instance(parent)?,
        intent,
    })
}

fn property_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Result<String, String> {
    let property = OwnedProperty(
        unsafe { store.GetValue(key) }
            .map_err(|error| hresult("IPropertyStore::GetValue", &error))?,
    );
    let value = unsafe { &property.0.Anonymous.Anonymous };
    if value.vt != VT_LPWSTR {
        return Err(format!("property type {:?}, expected VT_LPWSTR", value.vt));
    }
    let pointer = unsafe { value.Anonymous.pwszVal };
    if pointer.is_null() {
        return Err("property string pointer is null".into());
    }
    // SAFETY: PROPVARIANT owns a valid LPWSTR until OwnedProperty is dropped.
    // Scan explicitly so malformed property data cannot trigger an unbounded read.
    let mut length = 0;
    while length <= MAX_PROPERTY_STRING_UNITS {
        if unsafe { *pointer.0.add(length) } == 0 {
            break;
        }
        length += 1;
    }
    if length == 0 || length > MAX_PROPERTY_STRING_UNITS {
        return Err(format!(
            "property string length out of bounds: {length} units"
        ));
    }
    let wide = unsafe { std::slice::from_raw_parts(pointer.0, length) };
    String::from_utf16(wide).map_err(|error| format!("property string is invalid UTF-16: {error}"))
}

fn take_string(pointer: PWSTR, stage: &str) -> Result<String, String> {
    let owned = TaskString(pointer);
    if owned.0.is_null() {
        return Err(format!("{stage}: COM returned null"));
    }
    // SAFETY: COM returned a task-allocated UTF-16 string. The explicit scan
    // bound prevents malformed native strings from causing an unbounded read.
    let mut length = 0;
    while length <= MAX_PROPERTY_STRING_UNITS {
        if unsafe { *owned.0.0.add(length) } == 0 {
            break;
        }
        length += 1;
    }
    if length > MAX_PROPERTY_STRING_UNITS {
        return Err(format!("{stage}: string length exceeds bound"));
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(owned.0.0, length) })
        .map_err(|error| format!("{stage}: invalid UTF-16: {error}"))
}

fn root_instance(interface: &DeviceInterface) -> Result<String, String> {
    let path = interface.display_path();
    if !path.is_ascii() {
        return Err("control interface path cannot be represented losslessly".into());
    }
    let set = DeviceInfoSet(
        unsafe { SetupDiCreateDeviceInfoList(None, None) }
            .map_err(|error| hresult("SetupDiCreateDeviceInfoList", &error))?,
    );
    let mut data = SP_DEVICE_INTERFACE_DATA {
        cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
        ..Default::default()
    };
    unsafe { SetupDiOpenDeviceInterfaceW(set.0, &HSTRING::from(path), 0, Some(&mut data)) }
        .map_err(|error| hresult("SetupDiOpenDeviceInterfaceW(control)", &error))?;
    let mut info = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    let result =
        unsafe { SetupDiGetDeviceInterfaceDetailW(set.0, &data, None, 0, None, Some(&mut info)) };
    match result {
        Ok(()) => {
            return Err("SetupDiGetDeviceInterfaceDetailW(size) unexpectedly succeeded".into());
        }
        Err(error)
            if error.code()
                == windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER.to_hresult() =>
        {
            // The expected size query populated SP_DEVINFO_DATA.
        }
        Err(error) => {
            return Err(hresult("SetupDiGetDeviceInterfaceDetailW(size)", &error));
        }
    }
    instance(info.DevInst)
}

fn locate_devnode(instance_id: &str, stage: &str) -> Result<u32, String> {
    let mut node = 0;
    config(
        unsafe {
            CM_Locate_DevNodeW(
                &mut node,
                &HSTRING::from(instance_id),
                CM_LOCATE_DEVNODE_NORMAL,
            )
        },
        stage,
    )?;
    Ok(node)
}

fn instance(node: u32) -> Result<String, String> {
    let mut length = 0;
    config(
        unsafe { CM_Get_Device_ID_Size(&mut length, node, 0) },
        "CM_Get_Device_ID_Size",
    )?;
    if length == 0 || length as usize > MAX_INSTANCE_ID_UNITS {
        return Err(format!("PnP instance ID length out of bounds: {length}"));
    }
    let mut buffer = vec![0u16; length as usize + 1];
    config(
        unsafe { CM_Get_Device_IDW(node, &mut buffer, 0) },
        "CM_Get_Device_IDW",
    )?;
    if buffer[length as usize] != 0 {
        return Err("PnP instance ID is not NUL terminated".into());
    }
    String::from_utf16(&buffer[..length as usize])
        .map_err(|error| format!("PnP instance ID is invalid UTF-16: {error}"))
}

fn config(result: CONFIGRET, stage: &str) -> Result<(), String> {
    if result != CR_SUCCESS {
        return Err(format!("{stage}: CONFIGRET={}", result.0));
    }
    Ok(())
}

fn hresult(stage: &str, error: &WindowsError) -> String {
    format!("{stage}: HRESULT 0x{:08X} ({error})", error.code().0 as u32)
}

impl WindowsObserverClock {
    pub(crate) fn new() -> Self {
        let mut frequency = 0_i64;
        let frequency = unsafe { QueryPerformanceFrequency(&mut frequency) }
            .ok()
            .and_then(|()| u64::try_from(frequency).ok())
            .filter(|frequency| *frequency > 0);
        Self {
            frequency,
            origin: std::time::Instant::now(),
        }
    }
}

impl ObserverClock for WindowsObserverClock {
    fn now(&mut self) -> std::time::Duration {
        self.origin.elapsed()
    }

    fn qpc_100ns(&mut self) -> Option<u64> {
        let mut ticks = 0_i64;
        unsafe { QueryPerformanceCounter(&mut ticks) }.ok()?;
        let ticks = u64::try_from(ticks).ok()?;
        let frequency = self.frequency?;
        (u128::from(ticks).checked_mul(10_000_000)? / u128::from(frequency))
            .try_into()
            .ok()
    }
}
