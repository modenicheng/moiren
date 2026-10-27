//! Windows device enumeration: WASAPI endpoints, ASIO drivers, audio processes.
use super::backend::{BackendError, CatalogBackend};
use super::model::{DefaultEndpointRow, DeviceCatalog, DeviceRow, ProcessRow};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct WindowsBackend;

impl CatalogBackend for WindowsBackend {
    fn catalog(&self) -> Result<DeviceCatalog, BackendError> {
        let native = moiren_windows_audio::catalog_snapshot()
            .map_err(|e| BackendError::new("catalog", e))?;
        let mut result = DeviceCatalog::default();
        update_asio_catalog(&mut result, moiren_windows_audio::asio::list_drivers());
        let mut pids = BTreeSet::new();
        for endpoint in native.endpoints {
            for process in endpoint.sessions {
                if process.process_id != 0 {
                    pids.insert(process.process_id);
                }
            }
            let row = DeviceRow {
                name: endpoint.name.unwrap_or_else(|| endpoint.id.clone()),
                endpoint_id: endpoint.id,
            };
            match endpoint.flow {
                "capture" => result.inputs.push(row),
                "render" => result.outputs.push(row),
                _ => {}
            }
        }
        for default in native.defaults {
            if default.role == "multimedia" {
                match default.flow {
                    "capture" => result.default_input_endpoint_id = default.id.clone(),
                    "render" => result.default_output_endpoint_id = default.id.clone(),
                    _ => {}
                }
            }
            result.default_roles.push(DefaultEndpointRow {
                flow: default.flow,
                role: default.role,
                endpoint_id: default.id,
            });
        }
        for pid in pids {
            if let Ok(identity) = moiren_windows_audio::process_loopback::inspect_process(pid) {
                result.processes.push(ProcessRow {
                    pid: identity.pid,
                    creation_time_100ns: identity.creation_time_100ns,
                    name: identity.executable_name,
                });
            }
        }
        Ok(result)
    }
}

fn update_asio_catalog(
    catalog: &mut DeviceCatalog,
    result: Result<Vec<moiren_windows_audio::asio::AsioDriver>, moiren_engine::host::BackendError>,
) {
    match result {
        Ok(drivers) => {
            catalog.asio_drivers = drivers
                .into_iter()
                .map(|driver| DeviceRow {
                    endpoint_id: format!("asio:{}", driver.id),
                    name: driver.name,
                })
                .collect();
            catalog.asio_error = None;
        }
        Err(error) => {
            catalog.asio_drivers.clear();
            catalog.asio_error = Some(error.to_string());
        }
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn asio_discovery_failure_preserves_wasapi_catalog() {
        let endpoint = DeviceRow {
            endpoint_id: "wasapi-render".into(),
            name: "Speaker".into(),
        };
        let mut catalog = DeviceCatalog {
            outputs: vec![endpoint.clone()],
            default_output_endpoint_id: Some(endpoint.endpoint_id.clone()),
            ..Default::default()
        };
        update_asio_catalog(
            &mut catalog,
            Err(moiren_engine::host::BackendError::Unsupported(
                "ASIO requires a 64-bit process".into(),
            )),
        );
        assert_eq!(catalog.outputs, vec![endpoint]);
        assert_eq!(
            catalog.default_output_endpoint_id.as_deref(),
            Some("wasapi-render")
        );
        assert!(catalog.asio_drivers.is_empty());
        assert!(catalog.asio_error.unwrap().contains("64-bit"));
    }

    #[test]
    fn asio_discovery_uses_stable_driver_id_and_clears_previous_error() {
        let mut catalog = DeviceCatalog {
            asio_error: Some("old discovery failure".into()),
            ..Default::default()
        };
        update_asio_catalog(
            &mut catalog,
            Ok(vec![moiren_windows_audio::asio::AsioDriver {
                id: "{stable-clsid}".into(),
                name: "Driver".into(),
            }]),
        );
        assert_eq!(catalog.asio_drivers[0].endpoint_id, "asio:{stable-clsid}");
        assert_eq!(catalog.asio_drivers[0].name, "Driver");
        assert_eq!(catalog.asio_error, None);
    }
}
