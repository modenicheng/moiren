use super::*;

fn compatible_physical_demand<S: ProcessingSample>() {
    use crate::virtual_owner::VirtualBufferOptions;

    // Both below and above the provisional block*8 estimate must use the
    // negotiated master's complete wake, not just agree in diagnostics.
    for (rate, block, native_rate, native_period) in [
        (48_000, 64, 48_000, 512),
        (48_000, 256, 48_000, 128),
        (48_000, 4096, 48_000, 64),
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
        let demand =
            (native_period as u64 * u64::from(rate)).div_ceil(u64::from(native_rate)) as u32 + 16;
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
                assert_eq!(demand, 528);
                assert_eq!(reprime.required_frames, 1042);
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
            if block == 4096 {
                // Compatible keeps its legacy full-ceiling servo geometry:
                // transfer4137, residual6123, playback10260/MIC6124.
                let boundary = snapshot
                    .boundary_probes
                    .iter()
                    .find(|probe| probe.kind == kind)
                    .unwrap();
                assert_eq!(
                    boundary.target_frames,
                    Some(if kind == NativeStreamKind::VirtualPlayback {
                        10260
                    } else {
                        6124
                    })
                );
            }
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

fn exact_physical_target<S: ProcessingSample>() {
    for (block, period, burst, transfer, minimum) in
        [(64, 512, 528, 530, 1138), (4096, 64, 80, 81, 689)]
    {
        check_physical_target::<S>(block, period, burst, transfer, minimum);
    }
}

fn check_physical_target<S: ProcessingSample>(
    block: usize,
    period: usize,
    burst: u32,
    transfer: u32,
    minimum: u32,
) {
    use crate::latency::transport::{
        DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy,
    };
    use crate::virtual_owner::VirtualBufferOptions;

    // Charge the complete corrected transfer, input FIR32, delivery gap480
    // and jitter96 even when the entire wake is below the Engine ceiling.
    for target in [minimum - 1, minimum] {
        let mut project = virtual_project(2, 48_000);
        project.processing.engine.max_block_frames = block;
        let physical = add_physical_output(&mut project, 2);
        let NodeConfig::Output(output) = project.nodes.get_mut(&physical).unwrap() else {
            unreachable!()
        };
        output.device.hardware_sample_rate = Some(48_000);
        output.device.period_frames = Some(period);
        let probe = Arc::new(Probe::default());
        let buffer = VirtualBufferOptions {
            input: TransportInputPolicy::Target {
                capacity_frames: 16384,
                target_frames: target,
                jitter_frames: 96,
            },
            delivery: Some(InputDeliveryTiming {
                sample_rate: 48_000,
                max_packet_frames: 480,
                max_gap_frames: 480,
                source: DeliveryTimingSource::Declared,
            }),
            ..Default::default()
        };
        let backend = backend::<S>(&probe).with_virtual_buffer(buffer).unwrap();
        let native = native_devices(Default::default());
        let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = bus.create_host(8).unwrap();
        let prepared = backend.prepare_with_native(
            request(&project, &host),
            &CancellationToken::default(),
            &native,
        );
        if target < minimum {
            assert!(
                prepared.is_err(),
                "a one-frame input reserve deficit must refuse"
            );
            assert!(probe.requests.lock().unwrap().is_empty());
        } else {
            let prepared = prepared.unwrap();
            let requests = probe.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            for request in requests.iter() {
                assert_eq!(request.buffer, buffer);
                let budget = request.prepared_budget().unwrap();
                assert_eq!(budget.engine_block_frames, block as u32);
                assert_eq!(budget.engine_burst_frames, burst);
                assert_eq!(budget.native_transfer_frames, transfer);
                assert_eq!(budget.queue.unwrap().prime_frames, minimum);
                assert_eq!(budget.queue.unwrap().modeled_minimum_frames, Some(minimum));
            }
            drop(requests);
            let mut engine = compile(
                &project.graph,
                prepared.bindings,
                project.processing.compile_config(1, 1),
            )
            .unwrap()
            .engine;
            crate::test_allocation::assert_allocates_nothing(|| {
                for _ in 0..2 {
                    let mut remaining = burst as usize;
                    while remaining > 0 {
                        let frames = remaining.min(block);
                        engine.render(frames).unwrap();
                        remaining -= frames;
                    }
                }
            });
            assert_eq!(engine.timeline(), u64::from(burst) * 2);
            let mut pcm = [0.0; 4096];
            let mut microphones = probe.microphones.lock().unwrap();
            let available = microphones[0].available_frames();
            let report = microphones[0]
                .read_interleaved(&mut pcm[..available * 2])
                .unwrap();
            assert!(report.transferred_frames > 0);
            assert_eq!(report.transferred_frames, available);
            assert_eq!(report.xruns, 0);
            assert!(
                pcm[..report.transferred_frames * 2]
                    .iter()
                    .any(|&v| v != 0.0)
            );
            drop(microphones);
            drop(engine);
            drop(prepared.transport);
            assert_sessions_joined(&probe, 2);
        }
        assert_eq!(native.probe.started.load(Ordering::Acquire), 0);
        assert_eq!(native.probe.dropped.load(Ordering::Acquire), 1);
    }
}

#[test]
fn exact_physical_master_target_preserves_all_input_reserves_f32() {
    exact_physical_target::<f32>();
}

#[test]
fn exact_physical_master_target_preserves_all_input_reserves_f64() {
    exact_physical_target::<f64>();
}
