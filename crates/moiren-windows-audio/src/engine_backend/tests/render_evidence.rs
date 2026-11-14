//! Backend attachment and owner retirement, without opening native devices.
use super::*;
use crate::{
    native::{NativeStream, StereoFrame},
    render_evidence::{
        RenderEvidenceConfig, RenderEvidenceError, RenderEvidenceFormat, RenderEvidenceWriter,
    },
};
use std::{
    sync::{Mutex, atomic::AtomicUsize},
    time::Duration,
};

const CAPACITY: usize = 256;
const NATIVE_PACKET_CAP: usize = 128;

#[derive(Default)]
struct EvidenceProbe {
    format_queries: AtomicUsize,
    attachments: Mutex<Vec<(RenderEvidenceFormat, RenderEvidenceConfig)>>,
    packets: AtomicUsize,
    retired: AtomicUsize,
}

// Opt in only these tests: the shared FakeNativeDevices must remain unsupported.
struct EvidenceDevices {
    inner: FakeNativeDevices,
    probe: Arc<EvidenceProbe>,
    format: Option<RenderEvidenceFormat>,
    reject_attachment: bool,
    packet_byte: u8,
}

impl EvidenceDevices {
    fn new(format: RenderEvidenceFormat, packet_byte: u8) -> Self {
        Self {
            inner: native_devices(Default::default()),
            probe: Arc::default(),
            format: Some(format),
            reject_attachment: false,
            packet_byte,
        }
    }
}

impl NativeDevices for EvidenceDevices {
    fn resolve(
        &self,
        selector: &EndpointSelector,
        device: DeviceOptions,
        input: bool,
    ) -> Result<Resolved, BackendError> {
        self.inner.resolve(selector, device, input)
    }

    fn factory(&self, group: &Group, rate: u32) -> Result<StreamFactory, BackendError> {
        let factory = self.inner.factory(group, rate)?;
        let probe = self.probe.clone();
        let format = self.format;
        let reject_attachment = self.reject_attachment;
        let packet_byte = self.packet_byte;
        Ok(Box::new(move || {
            Ok(Box::new(EvidenceStream {
                inner: factory()?,
                probe,
                format,
                reject_attachment,
                packet_byte,
                writer: None,
                emitted: false,
            }))
        }))
    }
}

struct EvidenceStream {
    inner: Box<dyn NativeStream>,
    probe: Arc<EvidenceProbe>,
    format: Option<RenderEvidenceFormat>,
    reject_attachment: bool,
    packet_byte: u8,
    writer: Option<RenderEvidenceWriter>,
    emitted: bool,
}

impl NativeStream for EvidenceStream {
    fn info(&self) -> NativeStreamInfo {
        self.inner.info()
    }

    fn render_evidence_format(&self) -> Option<RenderEvidenceFormat> {
        self.probe.format_queries.fetch_add(1, Ordering::AcqRel);
        self.format
    }

    fn attach_render_evidence(
        &mut self,
        writer: RenderEvidenceWriter,
    ) -> Result<(), Box<(BackendError, RenderEvidenceWriter)>> {
        let summary = writer.snapshot();
        self.probe
            .attachments
            .lock()
            .unwrap()
            .push((summary.format, summary.config));
        if self.reject_attachment {
            return Err(Box::new((attachment_error(), writer)));
        }
        assert!(self.writer.replace(writer).is_none());
        Ok(())
    }

    fn take_render_evidence(&mut self) -> Option<RenderEvidenceWriter> {
        let writer = self.writer.take();
        if writer.is_some() {
            self.probe.retired.fetch_add(1, Ordering::AcqRel);
        }
        writer
    }

    fn start(&mut self) -> Result<(), BackendError> {
        self.inner.start()
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.inner.stop()
    }

    fn release(&mut self) -> Result<(), BackendError> {
        self.inner.release()
    }

    fn wait(&mut self, timeout: Duration) -> Result<bool, BackendError> {
        self.inner.wait(timeout)
    }

    fn read(&mut self, output: &mut [StereoFrame]) -> Result<usize, BackendError> {
        self.inner.read(output)
    }

    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        let written = self.inner.write(input)?;
        if written >= 2
            && !self.emitted
            && let Some(writer) = self.writer.as_mut()
        {
            // A single bounded encoded witness exercises the real owner path;
            // these bytes are not a claim about the fake's complete audio run.
            let format = writer.snapshot().format;
            let frame_bytes = format.channels * usize::from(format.container_bits / 8);
            let packet = [self.packet_byte; 32];
            writer.stage_packet(&packet[..2 * frame_bytes], 2).unwrap();
            assert_eq!(writer.finish_packet(0).unwrap(), 2);
            self.emitted = true;
            self.probe.packets.fetch_add(1, Ordering::Release);
        }
        Ok(written)
    }
}

fn attachment_error() -> BackendError {
    unsupported("injected render evidence attachment rejection")
}

fn format(sample_format: SampleFormat) -> RenderEvidenceFormat {
    let bits = if sample_format == SampleFormat::Float64 {
        64
    } else {
        32
    };
    RenderEvidenceFormat {
        sample_rate: 96_000,
        sample_format,
        channels: 2,
        container_bits: bits,
        valid_bits: if sample_format == SampleFormat::Pcm24 {
            24
        } else {
            bits
        },
    }
}

fn physical_project(sample_format: SampleFormat) -> (EngineProject, NodeId, NodeId) {
    let mut project = virtual_project(2, 96_000);
    let input = replace_with_physical_capture(&mut project);
    let (&output, config) = project
        .nodes
        .iter_mut()
        .find(|(_, config)| matches!(config, NodeConfig::Output(_)))
        .unwrap();
    let NodeConfig::Output(config) = config else {
        unreachable!()
    };
    config.target = OutputTarget::Physical(endpoint("native"));
    config.device = physical_device(ClockRole::Master);
    for config in project.nodes.values_mut() {
        match config {
            NodeConfig::Input(input) => input.device.sample_format = sample_format,
            NodeConfig::Output(output) => output.device.sample_format = sample_format,
            _ => unreachable!(),
        }
    }
    (project, input, output)
}

fn assert_packet(reader: &mut NativeRenderEvidenceReader, packet_byte: u8) {
    let format = reader.origin().format;
    let frame_bytes = format.channels * usize::from(format.container_bits / 8);
    for offset in 0..2 {
        let frame = reader
            .pop()
            .expect("the installed native tap must reach its reader");
        assert_eq!(&frame.bytes[..frame_bytes], vec![packet_byte; frame_bytes]);
        assert!(frame.bytes[frame_bytes..].iter().all(|&byte| byte == 0));
        assert_eq!(frame.packet_sequence, 1);
        assert_eq!(frame.packet_frame_offset, offset);
        assert_eq!(frame.packet_frames, 2);
        assert_eq!(frame.release_hresult, 0);
    }
    assert!(reader.pop().is_none());
    assert!(reader.is_finished());
}

fn disabled<S: ProcessingSample>() {
    let (project, _, output) = physical_project(SampleFormat::Pcm24);
    for explicitly_disabled in [false, true] {
        let backend = ProfessionalBackend::<S>::new().unwrap();
        let backend = if explicitly_disabled {
            backend
                .with_render_evidence(Some(CAPACITY))
                .unwrap()
                .with_render_evidence(None)
                .unwrap()
        } else {
            backend
        };
        let native = EvidenceDevices::new(format(SampleFormat::Pcm24), 42);
        let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = bus.create_host(8).unwrap();
        let cancel = CancellationToken::default();
        let prepared = backend
            .prepare_with_native(request(&project, &host), &cancel, &native)
            .unwrap();
        let owner = attach(&project, prepared, cancel);
        owner.activate().unwrap();
        let retained = backend.native_observations().retain_session().unwrap();
        assert!(retained.render_evidence_origins().is_empty());
        assert_eq!(
            retained.render_evidence_reader(output, 7).err(),
            Some("render evidence output identity mismatch")
        );
        wait_until(|| native.inner.probe.written.load(Ordering::Acquire) != 0);
        assert_eq!(owner.stop_and_join().error, None);
        assert_eq!(native.probe.format_queries.load(Ordering::Acquire), 0);
        assert!(native.probe.attachments.lock().unwrap().is_empty());
        assert_eq!(native.probe.packets.load(Ordering::Acquire), 0);
        assert_eq!(native.probe.retired.load(Ordering::Acquire), 0);
        assert_eq!(native.inner.probe.dropped.load(Ordering::Acquire), 1);
    }
}

fn enabled<S: ProcessingSample>() {
    // Pcm24 in a 32-bit container must not be reconstructed as packed 24-bit.
    for (sample_format, packet_cap, capacity) in [
        (SampleFormat::Pcm24, NATIVE_PACKET_CAP, CAPACITY),
        (SampleFormat::Float64, 512, 512),
    ] {
        let layout = format(sample_format);
        let (mut project, input, output) = physical_project(sample_format);
        let mut native = EvidenceDevices::new(layout, 42);
        if packet_cap == 512 {
            // Driver-preferred period: the cap must come from negotiation, not
            // an explicit request, engine block size, or evidence capacity.
            for config in project.nodes.values_mut() {
                match config {
                    NodeConfig::Input(input) => input.device.period_frames = None,
                    NodeConfig::Output(output) => output.device.period_frames = None,
                    _ => unreachable!(),
                }
            }
            native.inner.faults.wrong_period = true;
        }
        let backend = ProfessionalBackend::<S>::new()
            .unwrap()
            .with_render_evidence(Some(capacity))
            .unwrap();
        let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
        let (host, _rt) = bus.create_host(8).unwrap();
        let cancel = CancellationToken::default();
        let prepared = backend
            .prepare_with_native(request(&project, &host), &cancel, &native)
            .unwrap();
        let config = RenderEvidenceConfig {
            capacity_frames: capacity,
            max_packet_frames: packet_cap,
        };
        assert_eq!(
            *native.probe.attachments.lock().unwrap(),
            [(layout, config)]
        );
        assert_eq!(native.probe.format_queries.load(Ordering::Acquire), 1);
        assert_eq!(native.inner.probe.started.load(Ordering::Acquire), 0);
        let owner = attach(&project, prepared, cancel.clone());
        owner.activate().unwrap();
        let retained = backend.native_observations().retain_session().unwrap();
        let origins = retained.render_evidence_origins();
        assert_eq!(
            origins.len(),
            1,
            "duplex capture must not acquire a render tap"
        );
        let origin = &origins[0];
        assert_eq!(origin.schema_version, 1);
        assert_eq!(origin.owner_generation, 7);
        assert_eq!(origin.node, output);
        assert_eq!(origin.kind, NativeStreamKind::MasterOutput);
        assert_eq!(origin.format, layout);
        assert_eq!(origin.config, config);
        assert_eq!(
            retained.render_evidence_reader(output, 8).err(),
            Some("render evidence owner generation mismatch")
        );
        assert_eq!(
            retained.render_evidence_reader(input, 7).err(),
            Some("render evidence output identity mismatch")
        );
        let mut reader = retained.render_evidence_reader(output, 7).unwrap();
        assert_eq!(
            retained.clone().render_evidence_reader(output, 7).err(),
            Some("render evidence disabled or already claimed")
        );
        assert_eq!(
            backend
                .native_observations()
                .retain_session()
                .unwrap()
                .render_evidence_reader(output, 7)
                .err(),
            Some("render evidence disabled or already claimed")
        );
        assert!(reader.final_summary().is_none());
        wait_until(|| native.probe.packets.load(Ordering::Acquire) == 1);
        cancel.cancel();
        wait_until(|| native.inner.probe.dropped.load(Ordering::Acquire) == 1);
        assert_eq!(native.probe.retired.load(Ordering::Acquire), 1);
        assert_packet(&mut reader, 42);
        assert!(
            reader.final_summary().is_none(),
            "native destruction is not control join"
        );
        assert_eq!(owner.stop_and_join().error, None);
        let summary = reader
            .final_summary()
            .expect("control join must publish final evidence");
        assert_eq!(summary.format, layout);
        assert_eq!(summary.config, config);
        assert_eq!(summary.attempts, 1);
        assert_eq!(summary.accepted_frames, 2);
        assert_eq!(summary.stored_frames, 2);
        assert_eq!(summary.rejected_frames, 0);
        assert_eq!(summary.dropped_frames, 0);
        assert_eq!(summary.overflow_packets, 0);
        assert_eq!(summary.reader_abandoned_packets, 0);
        assert_eq!(summary.invalid_attempts, 0);
        assert_eq!(summary.unobserved_packets, 0);
        assert_eq!(summary.unobserved_frames, 0);
        assert_eq!(summary.pending_abandoned_packets, 0);
        assert_eq!(summary.pending_abandoned_frames, 0);
        assert!(!summary.counters_saturated);
        assert!(!summary.sequence_exhausted);
        assert!(reader.is_finished());
        assert!(backend.native_observations().retain_session().is_none());
        assert_eq!(native.inner.probe.started.load(Ordering::Acquire), 1);
        assert_eq!(native.inner.probe.stopped.load(Ordering::Acquire), 1);
        assert_eq!(native.inner.probe.released.load(Ordering::Acquire), 1);
    }
}

fn pinned_replacement<S: ProcessingSample>() {
    let layout = format(SampleFormat::Pcm24);
    let (project, _, output) = physical_project(layout.sample_format);
    let backend = ProfessionalBackend::<S>::new()
        .unwrap()
        .with_render_evidence(Some(CAPACITY))
        .unwrap();
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    let old_native = EvidenceDevices::new(layout, 42);
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &old_native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    let old_session = backend.native_observations().retain_session().unwrap();
    let mut old_reader = old_session.render_evidence_reader(output, 7).unwrap();
    wait_until(|| old_native.probe.packets.load(Ordering::Acquire) == 1);
    assert_eq!(owner.stop_and_join().error, None);

    // Reuse BOTH generation and output identity, but install a distinct owner.
    let backend = backend.with_render_evidence(Some(2 * CAPACITY)).unwrap();
    let new_native = EvidenceDevices::new(layout, 99);
    let cancel = CancellationToken::default();
    let prepared = backend
        .prepare_with_native(request(&project, &host), &cancel, &new_native)
        .unwrap();
    let owner = attach(&project, prepared, cancel);
    owner.activate().unwrap();
    let new_session = backend.native_observations().retain_session().unwrap();
    assert_eq!(
        old_session.render_evidence_origins()[0]
            .config
            .capacity_frames,
        CAPACITY
    );
    assert_eq!(
        new_session.render_evidence_origins()[0]
            .config
            .capacity_frames,
        2 * CAPACITY
    );
    assert_eq!(
        old_session.render_evidence_reader(output, 7).err(),
        Some("render evidence disabled or already claimed")
    );
    let mut new_reader = new_session.render_evidence_reader(output, 7).unwrap();
    assert_eq!(
        new_session.render_evidence_reader(output, 7).err(),
        Some("render evidence disabled or already claimed")
    );
    assert_eq!(old_reader.origin().config.capacity_frames, CAPACITY);
    assert_eq!(
        old_reader.final_summary().unwrap().config.capacity_frames,
        CAPACITY
    );
    assert_eq!(new_reader.origin().config.capacity_frames, 2 * CAPACITY);
    assert!(new_reader.final_summary().is_none());
    wait_until(|| new_native.probe.packets.load(Ordering::Acquire) == 1);
    assert_eq!(owner.stop_and_join().error, None);
    assert_eq!(
        new_reader.final_summary().unwrap().config.capacity_frames,
        2 * CAPACITY
    );
    assert_packet(&mut old_reader, 42);
    assert_packet(&mut new_reader, 99);
}

fn preparation_errors<S: ProcessingSample>() {
    let layout = format(SampleFormat::Pcm24);
    let (project, _, _) = physical_project(layout.sample_format);
    let mut bus = moiren_signal::Bus::new(Default::default()).unwrap();
    let (host, _rt) = bus.create_host(8).unwrap();
    for case in 0..4 {
        let mut native = EvidenceDevices::new(layout, 42);
        let capacity = if case == 2 {
            NATIVE_PACKET_CAP - 1
        } else {
            CAPACITY
        };
        let expected = match case {
            0 => {
                native.format = None;
                unsupported("native render evidence unavailable")
            }
            1 => {
                native.reject_attachment = true;
                attachment_error()
            }
            2 => unsupported(RenderEvidenceError::InvalidConfig),
            3 => {
                native.format.as_mut().unwrap().valid_bits = 16;
                unsupported(RenderEvidenceError::InvalidLayout)
            }
            _ => unreachable!(),
        };
        let backend = ProfessionalBackend::<S>::new()
            .unwrap()
            .with_render_evidence(Some(capacity))
            .unwrap();
        let error = backend
            .prepare_with_native(
                request(&project, &host),
                &CancellationToken::default(),
                &native,
            )
            .err()
            .expect("evidence preparation must not silently disable an explicit request");
        assert_eq!(error, expected, "preparation case {case}");
        assert_eq!(native.probe.format_queries.load(Ordering::Acquire), 1);
        assert_eq!(
            native.probe.attachments.lock().unwrap().len(),
            usize::from(case == 1)
        );
        assert_eq!(native.probe.retired.load(Ordering::Acquire), 0);
        assert_eq!(native.inner.probe.opened.load(Ordering::Acquire), 1);
        assert_eq!(native.inner.probe.started.load(Ordering::Acquire), 0);
        assert_eq!(native.inner.probe.dropped.load(Ordering::Acquire), 1);
        assert!(backend.native_observations().retain_session().is_none());
    }
}

#[test]
fn render_evidence_disabled_never_queries_or_installs_a_tap_f32() {
    disabled::<f32>();
}
#[test]
fn render_evidence_disabled_never_queries_or_installs_a_tap_f64() {
    disabled::<f64>();
}
#[test]
fn render_evidence_exact_layout_caps_and_control_join_f32() {
    enabled::<f32>();
}
#[test]
fn render_evidence_exact_layout_caps_and_control_join_f64() {
    enabled::<f64>();
}
#[test]
fn render_evidence_once_claim_is_pinned_across_same_generation_replacement_f32() {
    pinned_replacement::<f32>();
}
#[test]
fn render_evidence_once_claim_is_pinned_across_same_generation_replacement_f64() {
    pinned_replacement::<f64>();
}
#[test]
fn render_evidence_unsupported_attachment_and_caps_fail_explicitly_f32() {
    preparation_errors::<f32>();
}
#[test]
fn render_evidence_unsupported_attachment_and_caps_fail_explicitly_f64() {
    preparation_errors::<f64>();
}

#[test]
fn render_evidence_builder_rejects_out_of_range_capacity() {
    for capacity in [0, 131_073, usize::MAX] {
        assert_eq!(
            ProfessionalBackend::<f32>::new()
                .unwrap()
                .with_render_evidence(Some(capacity))
                .err(),
            Some(unsupported(
                "render evidence capacity must be 1..131072 frames"
            ))
        );
        assert_eq!(
            ProfessionalBackend::<f64>::new()
                .unwrap()
                .with_render_evidence(Some(capacity))
                .err(),
            Some(unsupported(
                "render evidence capacity must be 1..131072 frames"
            ))
        );
    }
}
