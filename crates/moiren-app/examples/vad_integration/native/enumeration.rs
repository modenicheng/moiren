//! One bounded ALL-state snapshot after an ACTIVE enumeration failure.
use super::*;
use crate::rename::enumeration::{Endpoint, Failure, Identity, Read, budgeted, collect_endpoints};
use windows::Win32::{
    Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
    Media::Audio::{DEVICE_STATE, DEVICE_STATEMASK_ALL, eCapture, eRender},
};

fn read<T>(stage: &'static str, result: anyhow::Result<T>) -> Read<T> {
    match result {
        Ok(value) => Read::Value { value },
        Err(error) => Read::Error {
            stage,
            hresult: error
                .chain()
                .find_map(|cause| cause.downcast_ref::<windows::core::Error>())
                .map(|error| error.code().0),
            message: format!("{error:#}"),
        },
    }
}

pub(super) fn pending(
    stable: StableEndpointId,
    intent: Intent,
    root: &str,
    original: &anyhow::Error,
) -> Failure {
    Failure {
        original_error: format!("{original:#}"),
        direction: format!("{intent:?}"),
        requested_stable_id: stable.to_string(),
        requested_root: root.into(),
        enumeration_mask: "DEVICE_STATEMASK_ALL; diagnostic only",
        elapsed_ms: 0,
        total_endpoints: None,
        collection_error: Some(Read::NotAttempted {
            stage: "all-state collection",
            reason: "original failure published before diagnostic calls",
        }),
        truncated: false,
        collection_completed: false,
        endpoints: Vec::new(),
    }
}

pub(super) fn collect(
    enumerator: &IMMDeviceEnumerator,
    stable: StableEndpointId,
    intent: Intent,
    root: &str,
    deadline: Instant,
    original: &anyhow::Error,
) -> Failure {
    let started = Instant::now();
    // Check between stages, not merely endpoints. An individual COM call can
    // still stall; the unchanged process watchdog remains the ultimate bound.
    let deadline = deadline.min(started + Duration::from_secs(1));
    let mut evidence = pending(stable, intent, root, original);
    evidence.collection_error = None;
    let result = (|| -> anyhow::Result<()> {
        ensure!(
            Instant::now() < deadline,
            "no remaining metadata work budget"
        );
        let flow = if intent == Intent::Playback {
            eRender
        } else {
            eCapture
        };
        // Read-only metadata in the existing apartment. ALL-state entries never
        // participate in client selection, initialization or a success result.
        let devices =
            unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE(DEVICE_STATEMASK_ALL))? };
        ensure!(
            Instant::now() < deadline,
            "metadata budget expired before count"
        );
        let count = unsafe { devices.GetCount()? };
        evidence.total_endpoints = Some(count);
        let (endpoints, truncated, error) = collect_endpoints(
            count,
            || Instant::now() < deadline,
            |index| {
                let device = unsafe { devices.Item(index)? };
                let identity = budgeted(
                    "topology/PnP identity",
                    || Instant::now() < deadline,
                    || {
                        read(
                            "topology/PnP identity",
                            identity::candidate(enumerator, &device, intent).and_then(
                                |candidate| {
                                    let exact_requested_identity = validation::select(
                                        std::slice::from_ref(&candidate),
                                        stable,
                                        intent,
                                        root,
                                    )?
                                    .is_some();
                                    Ok(Identity {
                                        raw_instance: candidate.raw_instance,
                                        parent_instance: candidate.parent_instance,
                                        direction: format!("{intent:?}"),
                                        exact_requested_identity,
                                    })
                                },
                            ),
                        )
                    },
                );
                Ok(Endpoint {
                    index,
                    mmdevice_id: budgeted(
                        "GetId",
                        || Instant::now() < deadline,
                        || {
                            read(
                                "GetId",
                                unsafe { device.GetId() }
                                    .map_err(Into::into)
                                    .and_then(identity::take_string),
                            )
                        },
                    ),
                    state: budgeted(
                        "GetState",
                        || Instant::now() < deadline,
                        || {
                            read(
                                "GetState",
                                unsafe { device.GetState() }
                                    .map(|state| state.0)
                                    .map_err(Into::into),
                            )
                        },
                    ),
                    identity,
                    friendly_name: budgeted(
                        "FriendlyName",
                        || Instant::now() < deadline,
                        || Read::Value {
                            value: optional_property(&device, &PKEY_Device_FriendlyName),
                        },
                    ),
                    topology: budgeted(
                        "topology",
                        || Instant::now() < deadline,
                        || Read::Value {
                            value: topology::collect(&device),
                        },
                    ),
                })
            },
        );
        evidence.endpoints = endpoints;
        evidence.truncated = truncated;
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    })();
    if let Err(error) = result {
        evidence.collection_error = Some(read("all-state collection", Err(error)));
    }
    evidence.elapsed_ms = started.elapsed().as_millis();
    evidence.collection_completed = true;
    evidence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contextual_hresult_and_partial_fields_serialize_without_becoming_success() {
        let error = anyhow::Error::new(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80004005u32 as i32,
        )))
        .context("context");
        let value = read::<u32>("GetState", Err(error));
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["hresult"], 0x80004005u32 as i32);
        assert!(json["message"].as_str().unwrap().contains("context"));
        let value = budgeted::<u32>("GetState", || false, || unreachable!());
        assert_eq!(
            serde_json::to_value(value).unwrap()["status"],
            "not_attempted"
        );
    }
}
