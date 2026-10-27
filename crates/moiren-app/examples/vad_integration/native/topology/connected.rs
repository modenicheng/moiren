//! One connection only; errors at a dependency retain their failed API stage.
use super::super::super::rename::{
    ConnectedConnectorEvidence, ConnectedPartEvidence, ConnectedPartProperties,
    TopologyIdentityMatch, TopologyRead, TopologyReadError,
};
use super::readback;
use windows::{
    Win32::Media::Audio::{IConnector, IPart},
    core::Interface,
};
type Read<T> = Result<T, TopologyReadError>;

fn collect_with<C>(
    adapter_id: &TopologyRead<String>,
    connector_id: impl FnOnce() -> Read<String>,
    connection: impl FnOnce() -> Read<C>,
    read_connector: impl FnOnce(C) -> ConnectedConnectorEvidence,
) -> ConnectedPartEvidence {
    let connector_id = readback(connector_id());
    let connection = readback(connection().map(read_connector));
    let part = match &connection {
        TopologyRead::Value { value } => match &value.part {
            TopologyRead::Value { value } => Some(value),
            TopologyRead::Error { .. } => None,
        },
        TopologyRead::Error { .. } => None,
    };
    ConnectedPartEvidence {
        adapter_id_match: identity_match(adapter_id, part.map(|p| &p.topology_device_id)),
        connector_id_match: identity_match(&connector_id, part.map(|p| &p.global_id)),
        connector_id,
        connection,
    }
}

fn identity_match(
    left: &TopologyRead<String>,
    right: Option<&TopologyRead<String>>,
) -> TopologyIdentityMatch {
    match (left, right) {
        (TopologyRead::Value { value: left }, Some(TopologyRead::Value { value: right })) => {
            if left == right {
                TopologyIdentityMatch::Match
            } else {
                TopologyIdentityMatch::Mismatch
            }
        }
        _ => TopologyIdentityMatch::Unknown,
    }
}

fn connector_with<P>(
    connector_type: Read<i32>,
    data_flow: Read<i32>,
    part: Read<P>,
    read_part: impl FnOnce(P) -> ConnectedPartProperties,
) -> ConnectedConnectorEvidence {
    ConnectedConnectorEvidence {
        connector_type: readback(connector_type),
        data_flow: readback(data_flow),
        part: readback(part.map(read_part)),
    }
}

fn part_with(
    part_type: impl FnOnce() -> Read<i32>,
    local_id: impl FnOnce() -> Read<u32>,
    global_id: impl FnOnce() -> Read<String>,
    name: impl FnOnce() -> Read<String>,
    subtype: impl FnOnce() -> Read<String>,
    topology_device_id: impl FnOnce() -> Read<String>,
) -> ConnectedPartProperties {
    ConnectedPartProperties {
        part_type: readback(part_type()),
        local_id: readback(local_id()),
        global_id: readback(global_id()),
        name: readback(name()),
        subtype: readback(subtype()),
        topology_device_id: readback(topology_device_id()),
    }
}

pub(super) fn unavailable() -> ConnectedPartEvidence {
    ConnectedPartEvidence {
        connector_id: TopologyRead::Error {
            error: TopologyReadError::Dependency {
                stage: "GetConnectorIdConnectedTo",
                dependency: "endpoint_connector",
            },
        },
        connection: TopologyRead::Error {
            error: TopologyReadError::Dependency {
                stage: "GetConnectedTo",
                dependency: "endpoint_connector",
            },
        },
        adapter_id_match: TopologyIdentityMatch::Unknown,
        connector_id_match: TopologyIdentityMatch::Unknown,
    }
}

pub(super) fn collect(
    connector: &IConnector,
    adapter_id: &TopologyRead<String>,
) -> ConnectedPartEvidence {
    collect_with(
        adapter_id,
        || {
            super::task_text(
                "GetConnectorIdConnectedTo",
                "connected_connector_id",
                unsafe { connector.GetConnectorIdConnectedTo() },
            )
        },
        || {
            unsafe { connector.GetConnectedTo() }
                .map_err(|error| super::hresult("GetConnectedTo", error))
        },
        |peer| {
            // Only the returned adapter-side connector is cast, never the
            // synthetic endpoint connector. Each getter has its own result.
            connector_with(
                unsafe { peer.GetType() }
                    .map(|value| value.0)
                    .map_err(|error| super::hresult("IConnector::GetType", error)),
                unsafe { peer.GetDataFlow() }
                    .map(|value| value.0)
                    .map_err(|error| super::hresult("IConnector::GetDataFlow", error)),
                peer.cast::<IPart>()
                    .map_err(|error| super::hresult("QueryInterface(IPart)", error)),
                |part| {
                    part_with(
                        || {
                            unsafe { part.GetPartType() }
                                .map(|value| value.0)
                                .map_err(|error| super::hresult("IPart::GetPartType", error))
                        },
                        || {
                            unsafe { part.GetLocalId() }
                                .map_err(|error| super::hresult("IPart::GetLocalId", error))
                        },
                        || {
                            super::task_text("IPart::GetGlobalId", "part_global_id", unsafe {
                                part.GetGlobalId()
                            })
                        },
                        || {
                            super::task_text("IPart::GetName", "part_name", unsafe {
                                part.GetName()
                            })
                        },
                        || {
                            unsafe { part.GetSubType() }
                                .map(|value| format!("{value:?}"))
                                .map_err(|error| super::hresult("IPart::GetSubType", error))
                        },
                        || {
                            let topology =
                                unsafe { part.GetTopologyObject() }.map_err(|error| {
                                    super::hresult("IPart::GetTopologyObject", error)
                                })?;
                            super::task_text(
                                "IDeviceTopology::GetDeviceId",
                                "part_topology_device_id",
                                unsafe { topology.GetDeviceId() },
                            )
                        },
                    )
                },
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADAPTER: &str = r"{2}.\\?\Mixed#Opaque Adapter ";
    const CONNECTOR: &str = "Exact Mixed Connector ";
    const SUBTYPE: &str = "DFF21CE1-F70F-11D0-B917-00A0C9223196";

    fn failure(stage: &'static str) -> TopologyReadError {
        TopologyReadError::Hresult {
            stage,
            hresult: -2147467259,
        }
    }

    fn collect_fixture(
        adapter_id: Read<String>,
        connector_id: Read<String>,
        connection: Read<()>,
        part: Read<()>,
        global_id: Read<String>,
        owner_id: Read<String>,
    ) -> ConnectedPartEvidence {
        collect_with(
            &readback(adapter_id),
            || connector_id,
            || connection,
            |_| {
                connector_with(Ok(2), Ok(1), part, |_| {
                    part_with(
                        || Ok(0),
                        || Ok(0xF123_ABCD),
                        || global_id,
                        || Ok("Bridge name ".into()),
                        || Ok(SUBTYPE.into()),
                        || owner_id,
                    )
                })
            },
        )
    }

    #[test]
    fn connected_part_collects_actual_peer_fields_and_full_local_identity() {
        let result = collect_fixture(
            Ok(ADAPTER.into()),
            Ok(CONNECTOR.into()),
            Ok(()),
            Ok(()),
            Ok(CONNECTOR.into()),
            Ok(ADAPTER.into()),
        );
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["connection"]["status"], "value");
        let peer = &json["connection"]["value"];
        assert_eq!(peer["connector_type"]["value"], 2);
        assert_eq!(peer["data_flow"]["value"], 1);
        let part = &peer["part"]["value"];
        assert_eq!(part["part_type"]["value"], 0);
        assert_eq!(part["local_id"]["value"], 0xF123_ABCDu32);
        assert_eq!(part["global_id"]["value"], CONNECTOR);
        assert_eq!(part["name"]["value"], "Bridge name ");
        assert_eq!(part["subtype"]["value"], SUBTYPE);
        assert_eq!(part["topology_device_id"]["value"], ADAPTER);
        assert_eq!(json["connector_id"]["value"], CONNECTOR);
        assert_eq!(json["adapter_id_match"], "match");
        assert_eq!(json["connector_id_match"], "match");
    }

    #[test]
    fn connected_part_identity_distinguishes_exact_mismatch_and_unknown() {
        for (adapter, connector, global, owner, expected_adapter, expected_connector) in [
            (
                Ok(ADAPTER.into()),
                Ok(CONNECTOR.into()),
                Ok(CONNECTOR.to_lowercase()),
                Ok(ADAPTER.trim().into()),
                "mismatch",
                "mismatch",
            ),
            (
                Err(failure("GetDeviceIdConnectedTo")),
                Ok(CONNECTOR.into()),
                Ok(CONNECTOR.into()),
                Ok(ADAPTER.into()),
                "unknown",
                "match",
            ),
            (
                Ok(ADAPTER.into()),
                Err(failure("GetConnectorIdConnectedTo")),
                Ok(CONNECTOR.into()),
                Err(failure("GetTopologyObject")),
                "unknown",
                "unknown",
            ),
        ] {
            let json = serde_json::to_value(collect_fixture(
                adapter,
                connector,
                Ok(()),
                Ok(()),
                global,
                owner,
            ))
            .unwrap();
            assert_eq!(json["adapter_id_match"], expected_adapter);
            assert_eq!(json["connector_id_match"], expected_connector);
            assert_eq!(
                json["connection"]["value"]["part"]["value"]["name"]["value"],
                "Bridge name "
            );
        }
    }

    #[test]
    fn connected_part_connection_is_followed_once_and_failure_blocks_dependents() {
        let calls = std::cell::Cell::new(0);
        let result = collect_with(
            &readback(Ok(ADAPTER.into())),
            || Ok(CONNECTOR.into()),
            || {
                calls.set(calls.get() + 1);
                Err::<(), _>(failure("GetConnectedTo"))
            },
            |_| panic!("unavailable peer must not be read"),
        );
        assert_eq!(calls.get(), 1);
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["connection"]["error"]["stage"], "GetConnectedTo");
        assert_eq!(json["connection"]["error"]["hresult"], -2147467259i32);
        assert_eq!(json["connector_id"]["value"], CONNECTOR);
        assert_eq!(json["adapter_id_match"], "unknown");
    }

    #[test]
    fn connected_part_query_interface_failure_retains_connector_siblings() {
        let result = collect_fixture(
            Ok(ADAPTER.into()),
            Ok(CONNECTOR.into()),
            Ok(()),
            Err(failure("QueryInterface(IPart)")),
            Ok(CONNECTOR.into()),
            Ok(ADAPTER.into()),
        );
        let json = serde_json::to_value(result).unwrap();
        let peer = &json["connection"]["value"];
        assert_eq!(peer["connector_type"]["value"], 2);
        assert_eq!(peer["data_flow"]["value"], 1);
        assert_eq!(peer["part"]["error"]["stage"], "QueryInterface(IPart)");
        assert_eq!(json["adapter_id_match"], "unknown");
        assert_eq!(json["connector_id_match"], "unknown");
    }

    #[test]
    fn connected_part_partial_getter_failures_do_not_short_circuit_siblings() {
        let calls = std::cell::Cell::new(0);
        let part = part_with(
            || {
                calls.set(calls.get() + 1);
                Err(failure("IPart::GetPartType"))
            },
            || {
                calls.set(calls.get() + 1);
                Ok(u32::MAX)
            },
            || {
                calls.set(calls.get() + 1);
                Ok(CONNECTOR.into())
            },
            || {
                calls.set(calls.get() + 1);
                Err(failure("IPart::GetName"))
            },
            || {
                calls.set(calls.get() + 1);
                Ok(SUBTYPE.into())
            },
            || {
                calls.set(calls.get() + 1);
                Err(failure("IPart::GetTopologyObject"))
            },
        );
        assert_eq!(calls.get(), 6);
        let connector =
            connector_with(Err(failure("IConnector::GetType")), Ok(1), Ok(()), |_| part);
        let result = collect_with(
            &readback(Ok(ADAPTER.into())),
            || Ok(CONNECTOR.into()),
            || Ok(()),
            |_| connector,
        );
        let json = serde_json::to_value(result).unwrap();
        let peer = &json["connection"]["value"];
        assert_eq!(
            peer["connector_type"]["error"]["stage"],
            "IConnector::GetType"
        );
        assert_eq!(peer["data_flow"]["value"], 1);
        let part = &peer["part"]["value"];
        assert_eq!(part["part_type"]["error"]["stage"], "IPart::GetPartType");
        assert_eq!(part["name"]["error"]["stage"], "IPart::GetName");
        assert_eq!(part["local_id"]["value"], u32::MAX);
        assert_eq!(part["subtype"]["value"], SUBTYPE);
        assert_eq!(
            part["topology_device_id"]["error"]["stage"],
            "IPart::GetTopologyObject"
        );
        assert_eq!(json["connector_id_match"], "match");
        assert_eq!(json["adapter_id_match"], "unknown");
    }

    #[test]
    fn connected_part_connector_getter_errors_do_not_block_the_actual_part() {
        let calls = std::cell::Cell::new(0);
        let result = collect_with(
            &readback(Ok(ADAPTER.into())),
            || Ok(CONNECTOR.into()),
            || {
                calls.set(calls.get() + 1);
                Ok(71u32)
            },
            |peer| {
                assert_eq!(peer, 71, "read the returned peer, not the endpoint");
                connector_with(
                    Err(failure("IConnector::GetType")),
                    Err(failure("IConnector::GetDataFlow")),
                    Ok(peer + 1),
                    |part| {
                        assert_eq!(part, 72);
                        part_with(
                            || Ok(0),
                            || Ok(u32::MAX),
                            || Ok(CONNECTOR.into()),
                            || Ok("Part despite connector errors".into()),
                            || Ok(SUBTYPE.into()),
                            || Ok(ADAPTER.into()),
                        )
                    },
                )
            },
        );
        assert_eq!(calls.get(), 1);
        let json = serde_json::to_value(result).unwrap();
        let peer = &json["connection"]["value"];
        assert_eq!(
            peer["connector_type"]["error"]["stage"],
            "IConnector::GetType"
        );
        assert_eq!(
            peer["data_flow"]["error"]["stage"],
            "IConnector::GetDataFlow"
        );
        assert_eq!(
            peer["part"]["value"]["name"]["value"],
            "Part despite connector errors"
        );
        assert_eq!(json["adapter_id_match"], "match");
        assert_eq!(json["connector_id_match"], "match");
    }

    #[test]
    fn connected_part_query_interface_failure_never_invokes_part_getters() {
        let result = connector_with::<()>(
            Ok(2),
            Err(failure("IConnector::GetDataFlow")),
            Err(failure("QueryInterface(IPart)")),
            |_| panic!("no part, no part getters"),
        );
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["connector_type"]["value"], 2);
        assert_eq!(
            json["data_flow"]["error"]["stage"],
            "IConnector::GetDataFlow"
        );
        assert_eq!(json["part"]["error"]["stage"], "QueryInterface(IPart)");
    }
}
