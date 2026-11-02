use super::*;

fn compatible_physical_demand<S: ProcessingSample>() {
    use crate::virtual_owner::VirtualBufferOptions;

    // Both below and above the provisional block*8 estimate must use the
    // negotiated master's complete wake, not just agree in diagnostics.
    for (rate, block, native_rate, native_period) in [
        (48_000, 64, 48_000, 512),
        (48_000, 256, 48_000, 128),
        (44_100, 64, 96_000, 128),
        (96_000, 64, 48_000, 512),
    ] {
        let mut project = virtual_project(2, rate);
        project.processing.engine.max_block_frames = block;
        let physical = add_physical_output(&mut project, 2);
        let NodeConfig::Output(output) = project.nodes.get_mut(&physical).unwrap() else {
            unreachable!()
        };
        output.device.hardware_sample_rate = Some(native_rate);
        output.device.period_frames = Some(native_period);
        let probe = Arc::new(Probe::default());
        let native = native_devices(Default::default());
        let backend = backend::<S>(&probe);
        let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = bus.create_host(8).unwrap();
        let cancel = CancellationToken::default();
        let prepared = backend
            .prepare_with_native(request(&project, &host), &cancel, &native)
            .unwrap();
        let demand = (native_period as u64 * u64::from(rate)).div_ceil(u64::from(native_rate))
            as u32
            + 64
            + 2 * block as u32;
        let expected = VirtualBufferOptions::default()
            .prepare_budget(rate, block as u32, demand)
            .unwrap();
        let requests = probe.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
        for request in &requests {
            assert_eq!(
                request.max_demand_frames, demand,
                "Compatible provider must receive the negotiated whole wake"
            );
            assert_eq!(request.prepared_budget().unwrap(), expected);
            assert_eq!(request.buffer, VirtualBufferOptions::default());
            let reprime = request
                .prepared_budget()
                .unwrap()
                .microphone_reprime(480, 4800)
                .unwrap();
            assert_eq!(reprime, expected.microphone_reprime(480, 4800).unwrap());
            if rate == 48_000 && block == 64 {
                assert_eq!(demand, 704);
                assert_eq!(reprime.required_frames, 1218);
                let provisional = VirtualBufferOptions::default()
                    .prepare_budget(rate, block as u32, block as u32 * 8)
                    .unwrap()
                    .microphone_reprime(480, 4800)
                    .unwrap();
                assert_eq!(provisional.required_frames, 1026);
                assert!(reprime.max_wait_100ns > provisional.max_wait_100ns);
            }
        }
        // Attaching publishes the same admitted contract the provider owns.
        let owner = attach(&project, prepared, cancel);
        owner.activate().unwrap();
        // Publication happens at attach; wait for actual fake Start before
        // requesting Stop so this lifecycle assertion is not a scheduling race.
        wait_until(|| native.probe.started.load(Ordering::Acquire) == 1);
        let snapshot = backend.native_observations().snapshot().unwrap();
        for kind in [
            NativeStreamKind::VirtualPlayback,
            NativeStreamKind::VirtualMicrophone,
        ] {
            let stream = snapshot.streams.iter().find(|s| s.kind == kind).unwrap();
            assert_eq!(stream.virtual_budget, Some(expected));
            assert_eq!(stream.input_queue, None);
        }
        assert_eq!(owner.stop_and_join().error, None);
        assert_sessions_joined(&probe, 2);
        assert_eq!(native.probe.started.load(Ordering::Acquire), 1);
        assert_eq!(native.probe.dropped.load(Ordering::Acquire), 1);
        assert_eq!(probe.clock_prepared.load(Ordering::Acquire), 0);
    }
}

#[test]
fn compatible_physical_master_provider_and_observation_share_whole_wake_f32() {
    compatible_physical_demand::<f32>();
}

#[test]
fn compatible_physical_master_provider_and_observation_share_whole_wake_f64() {
    compatible_physical_demand::<f64>();
}
