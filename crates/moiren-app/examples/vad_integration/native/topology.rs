//! Optional bounded evidence, never rename/ownership admission authority.
use super::super::rename::{
    AdapterDeviceEvidence, SetupInterfaceEvidence, TopologyEvidence, TopologyRead,
    TopologyReadError, TypedInterfaceName,
};
use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_Get_Device_IDW, CM_Get_Parent, CR_SUCCESS, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
                SP_DEVINFO_DATA, SetupDiCreateDeviceInfoList, SetupDiDestroyDeviceInfoList,
                SetupDiGetDeviceInterfaceDetailW, SetupDiGetDeviceInterfacePropertyW,
                SetupDiOpenDeviceInterfaceW,
            },
            FunctionDiscovery::{
                PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName, PKEY_DeviceInterface_FriendlyName,
            },
            Properties::{DEVPKEY_DeviceInterface_FriendlyName, DEVPROP_TYPE_STRING, DEVPROPTYPE},
        },
        Foundation::ERROR_INSUFFICIENT_BUFFER,
        Media::Audio::{
            IConnector, IDeviceTopology, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
        },
        System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree},
    },
    core::{Error, HSTRING, PWSTR},
};

const MAX_UNITS: usize = 4096;
const PROPERTY_BYTES: usize = (MAX_UNITS + 1) * 2;

#[path = "topology/connected.rs"]
mod connected;

fn bounded_utf16(
    field: &'static str,
    mut read: impl FnMut(usize) -> Option<u16>,
) -> Result<String, TopologyReadError> {
    let mut units = Vec::with_capacity(MAX_UNITS);
    for index in 0..=MAX_UNITS {
        match read(index) {
            Some(0) => {
                return String::from_utf16(&units)
                    .map_err(|_| TopologyReadError::InvalidUtf16 { field });
            }
            Some(unit) if index < MAX_UNITS => units.push(unit),
            None if index == 0 => return Err(TopologyReadError::NullString { field }),
            _ => break,
        }
    }
    Err(TopologyReadError::UnterminatedString {
        field,
        max_units: MAX_UNITS,
    })
}

fn collect_with(
    read: impl FnOnce() -> Result<String, TopologyReadError>,
    setup: impl FnOnce(&str) -> SetupInterfaceEvidence,
    resolve: impl FnOnce(&str) -> Result<AdapterDeviceEvidence, TopologyReadError>,
) -> TopologyEvidence {
    match read() {
        Ok(value) => {
            let adapter_mmdevice = Some(readback(resolve(&value)));
            let setup_interface = Some(setup(&value));
            TopologyEvidence {
                adapter_id: TopologyRead::Value { value },
                setup_interface,
                adapter_mmdevice,
                connected_part: None,
            }
        }
        Err(error) => TopologyEvidence {
            adapter_id: TopologyRead::Error { error },
            setup_interface: None,
            adapter_mmdevice: None,
            connected_part: None,
        },
    }
}

fn collect_endpoint_with<C>(
    connector: Result<C, TopologyReadError>,
    adapter: impl FnOnce(&C) -> Result<String, TopologyReadError>,
    setup: impl FnOnce(&str) -> SetupInterfaceEvidence,
    resolve: impl FnOnce(&str) -> Result<AdapterDeviceEvidence, TopologyReadError>,
    connected: impl FnOnce(&C, &TopologyRead<String>) -> super::super::rename::ConnectedPartEvidence,
) -> TopologyEvidence {
    let mut evidence = collect_with(
        || connector.as_ref().map_err(Clone::clone).and_then(adapter),
        setup,
        resolve,
    );
    evidence.connected_part = Some(match connector {
        // Adapter-ID failure must not hide a successfully reached peer/part.
        Ok(connector) => connected(&connector, &evidence.adapter_id),
        // The original endpoint error is retained in adapter_id; only these
        // dependent operations are marked not attempted.
        Err(_) => connected::unavailable(),
    });
    evidence
}

fn property_text(kind: u32, bytes: u32, buffer: &[u8]) -> Result<String, TopologyReadError> {
    if kind != DEVPROP_TYPE_STRING.0 {
        return Err(TopologyReadError::PropertyType {
            property_type: kind,
        });
    }
    let capacity_bytes = buffer.len().min(PROPERTY_BYTES);
    let length = bytes as usize;
    if length < 2 || !length.is_multiple_of(2) || length > capacity_bytes {
        return Err(TopologyReadError::PropertyBytes {
            bytes,
            capacity_bytes,
        });
    }
    let units: Vec<_> = buffer[..length]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if units.last() != Some(&0) || units[..units.len() - 1].contains(&0) {
        return Err(TopologyReadError::PropertyTermination);
    }
    bounded_utf16("interface_friendly_name", |index| units.get(index).copied())
}

fn readback<T>(result: Result<T, TopologyReadError>) -> TopologyRead<T> {
    match result {
        Ok(value) => TopologyRead::Value { value },
        Err(error) => TopologyRead::Error { error },
    }
}

fn hresult(stage: &'static str, error: Error) -> TopologyReadError {
    TopologyReadError::Hresult {
        stage,
        hresult: error.code().0,
    }
}

struct TaskString(PWSTR);
impl Drop for TaskString {
    fn drop(&mut self) {
        // Exactly this call's COM allocation, freed even on bounds/UTF-16 error.
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

fn task_text(
    stage: &'static str,
    field: &'static str,
    result: windows::core::Result<PWSTR>,
) -> Result<String, TopologyReadError> {
    let owned = TaskString(result.map_err(|error| hresult(stage, error))?);
    // Successful COM string outputs are NUL-terminated. Keep their allocation
    // alive through the bounded, strict copy and free on every return path.
    bounded_utf16(field, |index| {
        if owned.0.is_null() {
            None
        } else {
            Some(unsafe { *owned.0.0.add(index) })
        }
    })
}

fn endpoint_connector(device: &IMMDevice) -> Result<IConnector, TopologyReadError> {
    // Apartment-local activation obtains metadata only, never starts a stream.
    // The returned connector is used and released by collect on this thread.
    let topology: IDeviceTopology = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| hresult("Activate(IDeviceTopology)", error))?;
    let count = unsafe { topology.GetConnectorCount() }
        .map_err(|error| hresult("GetConnectorCount", error))?;
    if count != 1 {
        return Err(TopologyReadError::ConnectorCount { count });
    }
    unsafe { topology.GetConnector(0) }.map_err(|error| hresult("GetConnector(0)", error))
}

struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        // Local enumeration container only: no device removal or registry writes.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn instance(node: u32, field: &'static str) -> Result<String, TopologyReadError> {
    let mut units = [0u16; MAX_UNITS + 1];
    let status = unsafe { CM_Get_Device_IDW(node, &mut units, 0) };
    if status != CR_SUCCESS {
        return Err(TopologyReadError::ConfigRet {
            stage: "CM_Get_Device_IDW",
            code: status.0,
        });
    }
    bounded_utf16(field, |index| units.get(index).copied())
}

fn typed_name(set: HDEVINFO, interface: &SP_DEVICE_INTERFACE_DATA) -> TypedInterfaceName {
    // Fixed, UTF-16-aligned buffer; no size probe, second read or fallback.
    let mut units = [0u16; MAX_UNITS + 1];
    let mut kind = DEVPROPTYPE::default();
    let mut required = 0;
    // SAFETY: the byte view covers exactly this exclusive fixed output buffer.
    let buffer =
        unsafe { std::slice::from_raw_parts_mut(units.as_mut_ptr().cast::<u8>(), PROPERTY_BYTES) };
    let result = unsafe {
        SetupDiGetDeviceInterfacePropertyW(
            set,
            interface,
            &DEVPKEY_DeviceInterface_FriendlyName,
            &mut kind,
            Some(&mut *buffer),
            Some(&mut required),
            0,
        )
    };
    typed_name_result(result, kind.0, required, buffer)
}

fn typed_name_result(
    result: windows::core::Result<()>,
    kind: u32,
    required: u32,
    buffer: &[u8],
) -> TypedInterfaceName {
    match result {
        Ok(()) => TypedInterfaceName {
            property_type: Some(kind),
            required_bytes: Some(required),
            readback: readback(property_text(kind, required, buffer)),
        },
        Err(error) => TypedInterfaceName {
            property_type: None,
            required_bytes: (error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult())
                .then_some(required),
            readback: readback(Err(hresult("SetupDiGetDeviceInterfacePropertyW", error))),
        },
    }
}

fn setup_interface(exact_id: &str) -> SetupInterfaceEvidence {
    let set = match unsafe { SetupDiCreateDeviceInfoList(None, None) } {
        Ok(set) => DeviceSet(set),
        Err(error) => {
            return SetupInterfaceEvidence::Error {
                error: hresult("SetupDiCreateDeviceInfoList", error),
            };
        }
    };
    let mut interface = SP_DEVICE_INTERFACE_DATA {
        cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
        ..Default::default()
    };
    // GetDeviceIdConnectedTo guarantees an opaque MMDevice ID, not a SetupAPI
    // path. Rejection is an expected diagnostic result. No trimming, splitting,
    // case folding, prefix stripping or second attempt is allowed here.
    if let Err(error) = unsafe {
        SetupDiOpenDeviceInterfaceW(set.0, &HSTRING::from(exact_id), 0, Some(&mut interface))
    } {
        return SetupInterfaceEvidence::Rejected {
            hresult: error.code().0,
        };
    }
    let mut info = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    // The documented NULL-detail form fills DeviceInfoData and returns
    // ERROR_INSUFFICIENT_BUFFER. This is one identity query, not a sizing retry.
    let detail = unsafe {
        SetupDiGetDeviceInterfaceDetailW(set.0, &interface, None, 0, None, Some(&mut info))
    };
    let identity = match detail {
        Err(error) if error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => Ok(info.DevInst),
        Err(error) => Err(hresult("SetupDiGetDeviceInterfaceDetailW", error)),
        Ok(()) => Err(TopologyReadError::UnexpectedDetailSuccess),
    };
    let pdo = identity
        .clone()
        .and_then(|node| instance(node, "pdo_instance"));
    let parent = identity.and_then(|node| {
        let mut parent = 0;
        let status = unsafe { CM_Get_Parent(&mut parent, node, 0) };
        if status != CR_SUCCESS {
            return Err(TopologyReadError::ConfigRet {
                stage: "CM_Get_Parent",
                code: status.0,
            });
        }
        instance(parent, "parent_instance")
    });
    SetupInterfaceEvidence::Opened {
        interface_class: format!("{:?}", interface.InterfaceClassGuid),
        pdo_instance: readback(pdo),
        parent_instance: readback(parent),
        friendly_name: typed_name(set.0, &interface),
    }
}

fn adapter_mmdevice(exact_id: &str) -> Result<AdapterDeviceEvidence, TopologyReadError> {
    // GetDeviceIdConnectedTo returns an opaque MMDevice ID. Its documented
    // consumer is GetDevice, even when SetupAPI rejects that same exact text.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| hresult("CoCreateInstance(MMDeviceEnumerator)", error))?;
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(exact_id)) }
        .map_err(|error| hresult("GetDevice(adapter_id)", error))?;
    let device_id = (|| {
        let owned = TaskString(
            unsafe { device.GetId() }.map_err(|error| hresult("GetId(adapter)", error))?,
        );
        bounded_utf16("adapter_mmdevice_id", |index| {
            if owned.0.is_null() {
                None
            } else {
                Some(unsafe { *owned.0.0.add(index) })
            }
        })
    })();
    Ok(AdapterDeviceEvidence {
        device_id: readback(device_id),
        device_desc: super::optional_property(&device, &PKEY_Device_DeviceDesc),
        friendly_name: super::optional_property(&device, &PKEY_Device_FriendlyName),
        interface_friendly_name: super::optional_property(
            &device,
            &PKEY_DeviceInterface_FriendlyName,
        ),
    })
}

pub(super) fn collect(device: &IMMDevice) -> TopologyEvidence {
    collect_endpoint_with(
        endpoint_connector(device),
        |connector| {
            task_text("GetDeviceIdConnectedTo", "adapter_id", unsafe {
                connector.GetDeviceIdConnectedTo()
            })
        },
        setup_interface,
        adapter_mmdevice,
        connected::collect,
    )
}

pub(super) fn collect_once(
    pending: &std::cell::Cell<bool>,
    collect: impl FnOnce() -> TopologyEvidence,
) -> Option<TopologyEvidence> {
    pending.replace(false).then(collect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_part_is_explicit_when_endpoint_connector_is_unavailable() {
        for error in [
            TopologyReadError::ConnectorCount { count: 2 },
            TopologyReadError::Hresult {
                stage: "Activate(IDeviceTopology)",
                hresult: -2147467259,
            },
            TopologyReadError::Hresult {
                stage: "GetConnector(0)",
                hresult: -2147024891,
            },
        ] {
            let result = collect_endpoint_with::<()>(
                Err(error.clone()),
                |_| panic!("no endpoint connector, no adapter-ID getter"),
                |_| panic!("no adapter ID, no SetupAPI"),
                |_| panic!("no adapter ID, no MMDevice"),
                |_, _| panic!("no endpoint connector, no connected-part getter"),
            );
            assert_eq!(result.adapter_id, TopologyRead::Error { error });
            let json = serde_json::to_value(result).unwrap();
            assert_eq!(
                json["connected_part"]["connection"]["error"]["kind"],
                "dependency"
            );
            assert_eq!(
                json["connected_part"]["connection"]["error"]["stage"],
                "GetConnectedTo"
            );
            assert_eq!(
                json["connected_part"]["connection"]["error"]["dependency"],
                "endpoint_connector"
            );
            assert_eq!(
                json["connected_part"]["connector_id"]["error"]["stage"],
                "GetConnectorIdConnectedTo"
            );
            assert_eq!(json["connected_part"]["adapter_id_match"], "unknown");
        }
    }

    #[test]
    fn connected_part_is_not_fabricated_by_adapter_only_collection() {
        let result = collect_with(
            || Ok("adapter".into()),
            |_| SetupInterfaceEvidence::Rejected {
                hresult: -2147024809,
            },
            |_| {
                Err(TopologyReadError::Hresult {
                    stage: "GetDevice(adapter_id)",
                    hresult: -2147467259,
                })
            },
        );
        assert!(
            result.connected_part.is_none(),
            "adapter-only collector knows nothing about connector availability"
        );
    }

    #[test]
    fn connected_part_reads_survive_adapter_id_failure_and_remain_once_only() {
        let pending = std::cell::Cell::new(true);
        let connections = std::cell::Cell::new(0);
        let collect = || {
            collect_endpoint_with(
                Ok(7u32),
                |endpoint| {
                    assert_eq!(*endpoint, 7);
                    Err(TopologyReadError::Hresult {
                        stage: "GetDeviceIdConnectedTo",
                        hresult: -2147467259,
                    })
                },
                |_| panic!("no ID, no SetupAPI"),
                |_| panic!("no ID, no MMDevice"),
                |endpoint, adapter| {
                    assert_eq!(*endpoint, 7);
                    assert!(matches!(adapter, TopologyRead::Error { .. }));
                    connections.set(connections.get() + 1);
                    connected::unavailable()
                },
            )
        };
        let evidence = collect_once(&pending, collect).unwrap();
        assert!(evidence.connected_part.is_some());
        assert!(matches!(evidence.adapter_id, TopologyRead::Error { .. }));
        assert!(evidence.adapter_mmdevice.is_none());
        assert!(evidence.setup_interface.is_none());
        assert_eq!(connections.get(), 1);
        assert!(collect_once(&pending, || panic!("must not recollect connected part")).is_none());
    }

    #[test]
    fn connected_part_task_strings_keep_strict_bounds_field_errors_and_hresult() {
        use windows::Win32::System::Com::CoTaskMemAlloc;
        let read_units = |units: &[u16]| {
            // Transfer a real task allocation to the production ownership helper.
            let pointer = unsafe { CoTaskMemAlloc(units.len() * size_of::<u16>()) }.cast::<u16>();
            assert!(!pointer.is_null());
            unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), pointer, units.len()) };
            task_text("IPart::GetName", "part_name", Ok(PWSTR(pointer)))
        };
        let exact: Vec<_> = " Mixed bridge name ".encode_utf16().chain([0]).collect();
        assert_eq!(read_units(&exact).unwrap(), " Mixed bridge name ");
        let mut maximum = vec![88u16; MAX_UNITS];
        maximum.push(0);
        assert_eq!(read_units(&maximum).unwrap().len(), MAX_UNITS);
        maximum[MAX_UNITS] = 88;
        assert_eq!(
            read_units(&maximum),
            Err(TopologyReadError::UnterminatedString {
                field: "part_name",
                max_units: MAX_UNITS,
            })
        );
        assert_eq!(
            read_units(&[0xd800, 0]),
            Err(TopologyReadError::InvalidUtf16 { field: "part_name" })
        );
        assert_eq!(
            task_text("IPart::GetName", "part_name", Ok(PWSTR::null())),
            Err(TopologyReadError::NullString { field: "part_name" })
        );
        assert_eq!(
            task_text(
                "IPart::GetName",
                "part_name",
                Err(Error::from_hresult(windows::core::HRESULT(-2147024891)))
            ),
            Err(TopologyReadError::Hresult {
                stage: "IPart::GetName",
                hresult: -2147024891,
            })
        );
    }

    #[test]
    fn bounded_utf16_preserves_exact_text_and_never_scans_beyond_limit() {
        let exact = r"{2}.\\?\Mixed#Opaque Adapter ";
        let units: Vec<_> = exact.encode_utf16().chain([0]).collect();
        assert_eq!(
            bounded_utf16("adapter_id", |i| units.get(i).copied()).unwrap(),
            exact
        );
        let mut units = vec![b'X' as u16; MAX_UNITS];
        units.push(0);
        assert_eq!(
            bounded_utf16("adapter_id", |i| units.get(i).copied())
                .unwrap()
                .len(),
            MAX_UNITS
        );
        let mut max_index = 0;
        let error = bounded_utf16("adapter_id", |i| {
            max_index = i;
            Some(88)
        })
        .unwrap_err();
        assert_eq!(max_index, MAX_UNITS);
        assert_eq!(
            error,
            TopologyReadError::UnterminatedString {
                field: "adapter_id",
                max_units: MAX_UNITS
            }
        );
        assert_eq!(
            bounded_utf16("adapter_id", |i| [0xd800, 0].get(i).copied()),
            Err(TopologyReadError::InvalidUtf16 {
                field: "adapter_id"
            })
        );
        assert_eq!(
            bounded_utf16("adapter_id", |_| None),
            Err(TopologyReadError::NullString {
                field: "adapter_id"
            })
        );
    }

    #[test]
    fn actual_collection_helper_never_normalizes_or_retries_opaque_setup_rejection() {
        let opaque = r"{2}.\\?\Mixed#Opaque Adapter ";
        let mut calls = 0;
        let result = collect_with(
            || Ok(opaque.into()),
            |actual| {
                calls += 1;
                assert_eq!(actual, opaque);
                SetupInterfaceEvidence::Rejected {
                    hresult: -2147024809,
                }
            },
            |actual| {
                assert_eq!(actual, opaque);
                Err(TopologyReadError::Hresult {
                    stage: "GetDevice(adapter_id)",
                    hresult: -2147467259,
                })
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(
            result.adapter_id,
            TopologyRead::Value {
                value: opaque.into()
            }
        );
        assert_eq!(
            result.setup_interface,
            Some(SetupInterfaceEvidence::Rejected {
                hresult: -2147024809
            })
        );
        assert_eq!(
            result.adapter_mmdevice,
            Some(TopologyRead::Error {
                error: TopologyReadError::Hresult {
                    stage: "GetDevice(adapter_id)",
                    hresult: -2147467259,
                }
            })
        );
        let error = TopologyReadError::Hresult {
            stage: "GetDeviceIdConnectedTo",
            hresult: -2147467259,
        };
        let result = collect_with(
            || Err(error.clone()),
            |_| panic!("no ID, no SetupAPI attempt"),
            |_| panic!("no ID, no MMDevice attempt"),
        );
        assert_eq!(result.adapter_id, TopologyRead::Error { error });
        assert!(result.setup_interface.is_none());
        assert!(result.adapter_mmdevice.is_none());
    }

    #[test]
    fn supported_mmdevice_resolution_survives_setup_rejection_without_name_fallback() {
        use super::super::super::rename::{PropertyReadError, PropertyReadback};
        let opaque = r"{2}.\\?\Mixed#Opaque Adapter ";
        let names = AdapterDeviceEvidence {
            device_id: TopologyRead::Value {
                value: opaque.into(),
            },
            device_desc: PropertyReadback::Value {
                value: "Adapter description ".into(),
            },
            friendly_name: PropertyReadback::Error {
                error: PropertyReadError::UnexpectedType { vartype: 0 },
            },
            interface_friendly_name: PropertyReadback::Value {
                value: "Different adapter interface".into(),
            },
        };
        let result = collect_with(
            || Ok(opaque.into()),
            |_| SetupInterfaceEvidence::Rejected {
                hresult: -536870363,
            },
            |actual| {
                assert_eq!(actual, opaque);
                Ok(names.clone())
            },
        );
        assert_eq!(
            result.adapter_mmdevice,
            Some(TopologyRead::Value { value: names })
        );
        assert_eq!(
            result.setup_interface,
            Some(SetupInterfaceEvidence::Rejected {
                hresult: -536870363
            })
        );
    }

    #[test]
    fn typed_property_decoder_rejects_wrong_type_size_termination_and_invalid_utf16() {
        let bytes: Vec<_> = "Requested "
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            property_text(18, bytes.len() as u32, &bytes).unwrap(),
            "Requested "
        );
        assert_eq!(property_text(18, 2, &[0, 0]).unwrap(), "");
        assert_eq!(
            property_text(25, bytes.len() as u32, &bytes),
            Err(TopologyReadError::PropertyType { property_type: 25 })
        );
        for size in [0, 1, 3, PROPERTY_BYTES as u32 + 2] {
            assert!(matches!(
                property_text(18, size, &bytes),
                Err(TopologyReadError::PropertyBytes { .. })
            ));
        }
        assert_eq!(
            property_text(18, 2, &[88, 0]),
            Err(TopologyReadError::PropertyTermination)
        );
        assert_eq!(
            property_text(18, 4, &[0, 0, 0, 0]),
            Err(TopologyReadError::PropertyTermination)
        );
        assert_eq!(
            property_text(18, 4, &[0, 0xd8, 0, 0]),
            Err(TopologyReadError::InvalidUtf16 {
                field: "interface_friendly_name"
            })
        );
        let mut maximum = [0x58, 0].repeat(MAX_UNITS);
        maximum.extend([0, 0]);
        assert_eq!(
            property_text(18, PROPERTY_BYTES as u32, &maximum)
                .unwrap()
                .len(),
            MAX_UNITS
        );
        maximum.extend([0, 0]);
        assert!(matches!(
            property_text(18, maximum.len() as u32, &maximum),
            Err(TopologyReadError::PropertyBytes { .. })
        ));
    }

    #[test]
    fn optional_failure_is_retained_once_not_retried_or_cloned_into_held_observation() {
        let pending = std::cell::Cell::new(true);
        let result = collect_once(&pending, || {
            collect_with(
                || {
                    Err(hresult(
                        "Activate(IDeviceTopology)",
                        Error::from_hresult(windows::core::HRESULT(-2147467259)),
                    ))
                },
                |_| panic!("failed adapter read must not call SetupAPI"),
                |_| panic!("failed adapter read must not call MMDevice"),
            )
        })
        .unwrap();
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(
            json["adapter_id"]["error"]["stage"],
            "Activate(IDeviceTopology)"
        );
        assert_eq!(json["adapter_id"]["error"]["hresult"], -2147467259i32);
        assert!(collect_once(&pending, || panic!("while-held must not recollect")).is_none());
    }

    #[test]
    fn actual_property_result_helper_preserves_hresult_and_never_invents_ddi_outputs() {
        let denied = windows::core::HRESULT(-2147024891);
        let result = typed_name_result(Err(Error::from_hresult(denied)), 18, 0, &[0; 2]);
        assert!(result.property_type.is_none());
        assert!(result.required_bytes.is_none());
        assert_eq!(
            result.readback,
            TopologyRead::Error {
                error: TopologyReadError::Hresult {
                    stage: "SetupDiGetDeviceInterfacePropertyW",
                    hresult: denied.0,
                },
            }
        );
        let result = typed_name_result(
            Err(Error::from_hresult(ERROR_INSUFFICIENT_BUFFER.to_hresult())),
            18,
            PROPERTY_BYTES as u32 + 2,
            &[0; 2],
        );
        assert!(result.property_type.is_none());
        assert_eq!(result.required_bytes, Some(PROPERTY_BYTES as u32 + 2));
        assert!(matches!(result.readback, TopologyRead::Error { .. }));
        let result = typed_name_result(Ok(()), 25, 2, &[0; 2]);
        assert_eq!(result.property_type, Some(25));
        assert_eq!(result.required_bytes, Some(2));
        assert_eq!(
            result.readback,
            TopologyRead::Error {
                error: TopologyReadError::PropertyType { property_type: 25 },
            }
        );
    }
}
