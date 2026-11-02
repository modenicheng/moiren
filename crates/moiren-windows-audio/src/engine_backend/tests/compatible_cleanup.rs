//! Deferred Compatible preparation must unwind the already parked native owner.
use super::*;

fn compatible_provider_failure_cleanup<S: ProcessingSample>() {
    for case in 0..3 {
        let mut project = virtual_project(2, 48_000);
        project.processing.engine.max_block_frames = 64;
        let physical = add_physical_output(&mut project, 2);
        let NodeConfig::Output(output) = project.nodes.get_mut(&physical).unwrap() else {
            unreachable!()
        };
        output.device.hardware_sample_rate = Some(48_000);
        output.device.period_frames = Some(512);

        let probe = Arc::new(Probe::default());
        let mut provider = FakeProvider::new(probe.clone());
        provider.fail_microphone = case == 0;
        provider.short_carrier = case == 1;
        provider.cancel_after_playback = case == 2;
        let backend = ProfessionalBackend::<S>::new()
            .unwrap()
            .with_virtual_provider(Arc::new(provider))
            .with_virtual_clock_factory(Arc::new(FakeClockFactory(probe.clone())));
        let native = native_devices(Default::default());
        let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = bus.create_host(8).unwrap();
        let cancel = CancellationToken::default();
        let error = backend
            .prepare_with_native(request(&project, &host), &cancel, &native)
            .err()
            .expect("injected virtual preparation failure must not return a transport");
        match case {
            0 => assert!(
                matches!(error, BackendError::ResourceUnavailable(ref message)
                if message == "injected microphone preparation")
            ),
            1 => assert!(matches!(error, BackendError::Unsupported(_))),
            2 => assert_eq!(error, BackendError::Cancelled),
            _ => unreachable!(),
        }
        assert_eq!(cancel.is_cancelled(), case == 2);

        // Exactly the successful Playback preparation is owned by the local
        // VirtualSessions guard, even when binding or the next step fails.
        assert_sessions_joined(&probe, 1);
        let requests = probe.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].max_demand_frames, 704);
        assert_eq!(requests[0].buffer, Default::default());
        assert_eq!(probe.activated.load(Ordering::Acquire), 0);
        assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 0);
        assert!(probe.microphones.lock().unwrap().is_empty());

        // The worker has negotiated but never crossed await_activation.
        // Drop before prepare returns demonstrates cancellation wakes and joins
        // that parked owner; zero Start/write is not a successful audio run.
        assert_eq!(native.probe.opened.load(Ordering::Acquire), 1);
        assert_eq!(native.probe.master_groups.load(Ordering::Acquire), 1);
        assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
        assert_eq!(native.probe.written.load(Ordering::Acquire), 0);
        assert_eq!(native.probe.dropped.load(Ordering::Acquire), 1);
    }
}

#[test]
fn compatible_virtual_prepare_failures_join_parked_physical_owner_f32() {
    compatible_provider_failure_cleanup::<f32>();
}

#[test]
fn compatible_virtual_prepare_failures_join_parked_physical_owner_f64() {
    compatible_provider_failure_cleanup::<f64>();
}
