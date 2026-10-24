//! Present control-interface discovery; this does not open a driver session.

use super::{
    ClientError,
    interface_path::{MAX_DETAIL_BYTES, decode_detail_path},
};
use std::mem::size_of;
use windows::{
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
            SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList,
            SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
        },
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS},
    },
    core::{GUID, PCWSTR},
};

pub const CONTROL_INTERFACE_GUID: GUID =
    GUID::from_u128(super::protocol::CONTROL_INTERFACE_GUID_U128);

/// A NUL-terminated interface path obtained from a present SetupAPI interface.
///
/// Discovery does not imply that the interface is still present when opened.
/// Paths cannot be constructed from arbitrary strings through this API.
#[derive(Debug, Clone)]
pub struct DeviceInterface {
    path: Vec<u16>,
}

impl DeviceInterface {
    pub fn display_path(&self) -> String {
        String::from_utf16_lossy(&self.path[..self.path.len() - 1])
    }

    pub(super) fn wide_path(&self) -> PCWSTR {
        PCWSTR(self.path.as_ptr())
    }
}

struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns a successful SetupDiGetClassDevsW result.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

pub fn discover_present_interfaces() -> Result<Vec<DeviceInterface>, ClientError> {
    // SAFETY: the GUID is live for the call; no enumerator or parent is supplied.
    let devices = DeviceInfoSet(
        unsafe {
            SetupDiGetClassDevsW(
                Some(&CONTROL_INTERFACE_GUID),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        }
        .map_err(|error| ClientError::windows("SetupDiGetClassDevsW", error))?,
    );
    let mut interfaces = Vec::new();
    for index in 0..u32::MAX {
        let mut data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: the set is owned for the whole loop and data is writable.
        match unsafe {
            SetupDiEnumDeviceInterfaces(devices.0, None, &CONTROL_INTERFACE_GUID, index, &mut data)
        } {
            Ok(()) => interfaces.push(interface_detail(&devices, &data)?),
            Err(error) if error.code() == ERROR_NO_MORE_ITEMS.to_hresult() => {
                return Ok(interfaces);
            }
            Err(error) => {
                return Err(ClientError::windows("SetupDiEnumDeviceInterfaces", error));
            }
        }
    }
    Err(ClientError::Protocol(super::ProtocolError::Inconsistent(
        "interface enumeration index exhausted",
    )))
}

fn interface_detail(
    devices: &DeviceInfoSet,
    data: &SP_DEVICE_INTERFACE_DATA,
) -> Result<DeviceInterface, ClientError> {
    let mut required = 0;
    // SAFETY: this first pass only obtains the required byte count.
    match unsafe {
        SetupDiGetDeviceInterfaceDetailW(devices.0, data, None, 0, Some(&mut required), None)
    } {
        Err(error) if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {}
        Err(error) => {
            return Err(ClientError::windows(
                "SetupDiGetDeviceInterfaceDetailW(size)",
                error,
            ));
        }
        Ok(()) => {
            return Err(super::ProtocolError::Inconsistent(
                "SetupAPI accepted an absent detail buffer",
            )
            .into());
        }
    }
    // Retry only a bounded number of size races. Hot unplug and other errors are
    // reported; a partially populated interface list is never treated as complete.
    for _ in 0..4 {
        let capacity = required as usize;
        if !(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>()..=MAX_DETAIL_BYTES).contains(&capacity)
        {
            return Err(super::ProtocolError::Inconsistent(
                "SetupAPI detail size is outside the bounded allocation",
            )
            .into());
        }
        // u32 storage provides the SDK structure's alignment. Only the FFI call
        // sees an SDK pointer; the driver ABI never uses native struct layouts.
        let mut storage = vec![0_u32; capacity.div_ceil(size_of::<u32>())];
        storage[0] = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        let mut returned = required;
        // SAFETY: storage is aligned, initialized, and at least required bytes;
        // data and devices remain live. cbSize uses the architecture's SDK size.
        let result = unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                devices.0,
                data,
                Some(storage.as_mut_ptr().cast()),
                required,
                Some(&mut returned),
                None,
            )
        };
        match result {
            Ok(()) => {
                if returned as usize > capacity || returned < 8 {
                    return Err(super::ProtocolError::Inconsistent(
                        "SetupAPI returned an invalid detail length",
                    )
                    .into());
                }
                // Windows targets are little-endian. Copy out only the returned
                // bytes, without dereferencing a variable-length struct tail.
                let mut bytes: Vec<_> =
                    storage.iter().flat_map(|word| word.to_le_bytes()).collect();
                bytes.truncate(returned as usize);
                return Ok(DeviceInterface {
                    path: decode_detail_path(
                        &bytes,
                        size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32,
                    )?,
                });
            }
            Err(error)
                if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult()
                    && returned > required =>
            {
                required = returned;
            }
            Err(error) => {
                return Err(ClientError::windows(
                    "SetupDiGetDeviceInterfaceDetailW(path)",
                    error,
                ));
            }
        }
    }
    Err(super::ProtocolError::Inconsistent("SetupAPI detail size kept changing").into())
}
