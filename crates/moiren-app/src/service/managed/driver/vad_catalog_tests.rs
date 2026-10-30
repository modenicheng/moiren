use super::*;
use crate::service::{
    ExportDirection, ExportIntent, ExportObservation, ExportResourceState,
    WindowsEndpointNameObservation,
};
use moiren_engine::node::VirtualEndpointId;

#[test]
fn catalog_refresh_preserves_owned_ids_until_confirmed_absence() {
    let id = VirtualEndpointId::from_bytes([0x42; 16]);
    let intent = ExportIntent {
        endpoint_id: id,
        direction: ExportDirection::Playback,
        name: "Speaker".into(),
        channels: 2,
        enabled: true,
        binding: None,
    };
    let mut context = ServiceContext::default();
    context.exports.set_available(true);
    context
        .exports
        .observe(ExportObservation {
            intent,
            generation: 1,
            resource: ExportResourceState::Removing,
            allow_default: false,
            gate_open: false,
            active_revision: None,
        })
        .unwrap();
    context.catalog = Arc::new(super::super::super::DeviceCatalog {
        vad_endpoints: [(id, "owned-mmdevice".into())].into_iter().collect(),
        ..Default::default()
    });
    let mut driver =
        ManagedDriver::new(|_| Err(host::BackendError::Unsupported("catalog-only test".into())));
    driver.completions = Some(CompletionQueue::new(std::thread::current()));
    assert!(
        driver
            .completions
            .as_ref()
            .unwrap()
            .port()
            .try_reserve()
            .unwrap()
            .send(Completion::Catalog(Ok(
                super::super::super::DeviceCatalog::default()
            )))
            .is_ok()
    );
    driver.poll_completions(&mut context, RoundBudget::default());
    update_vad_catalog(&mut context);
    assert_eq!(
        context.catalog.vad_endpoints.get(&id).map(String::as_str),
        Some("owned-mmdevice")
    );
    context.exports.observations.get_mut(&id).unwrap().resource = ExportResourceState::Absent;
    update_vad_catalog(&mut context);
    assert!(context.catalog.vad_endpoints.is_empty());
}

#[test]
fn present_observation_adds_the_exact_sampled_mmdevice_id() {
    let id = VirtualEndpointId::from_bytes([0x43; 16]);
    let mut context = ServiceContext::default();
    context.exports.set_available(true);
    context
        .exports
        .observe(ExportObservation {
            intent: ExportIntent {
                endpoint_id: id,
                direction: ExportDirection::Microphone,
                name: "Mic".into(),
                channels: 2,
                enabled: true,
                binding: None,
            },
            generation: 1,
            resource: ExportResourceState::Present,
            allow_default: false,
            gate_open: false,
            active_revision: None,
        })
        .unwrap();
    context.exports.windows_names.insert(
        id,
        WindowsEndpointNameObservation {
            native_identity: [1, 1, 1],
            observed_qpc_100ns: None,
            mmdevice_id: Some("exact-capture-id".into()),
            name: Some("Windows Mic".into()),
            error: None,
        },
    );
    update_vad_catalog(&mut context);
    assert_eq!(
        context.catalog.vad_endpoints.get(&id).map(String::as_str),
        Some("exact-capture-id")
    );
}
