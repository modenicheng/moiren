use super::*;
use crate::professional::native::{NativeRecoveryRequest, NativeStream, StereoFrame};
use crate::virtual_owner::{
    BoundaryProbe, ClockWake, PreparedVirtualMicrophone, PreparedVirtualPlayback,
    VirtualBoundaryRequest, VirtualRealtimeClock, VirtualResourceSession,
};
use moiren_engine::{
    boundary::{AudioReader, AudioWriter, audio_bridge},
    host::OwnerStatus,
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Probe {
    pub prepared: AtomicUsize,
    pub activated: AtomicUsize,
    pub cancelled: AtomicUsize,
    pub joined: AtomicUsize,
    pub dropped: AtomicUsize,
    pub clock_prepared: AtomicUsize,
    pub requests: Mutex<Vec<VirtualBoundaryRequest>>,
    pub playback: Mutex<Vec<AudioWriter<f32>>>,
    pub microphones: Mutex<Vec<AudioReader<f32>>>,
    pub fail_session: AtomicBool,
}

pub(super) struct FakeProvider {
    pub probe: Arc<Probe>,
    pub fail_microphone: bool,
    pub short_carrier: bool,
    pub cancel_after_playback: bool,
}
impl FakeProvider {
    pub fn new(probe: Arc<Probe>) -> Self {
        Self {
            probe,
            fail_microphone: false,
            short_carrier: false,
            cancel_after_playback: false,
        }
    }
    fn session(&self, request: VirtualBoundaryRequest) -> Box<dyn VirtualResourceSession> {
        self.probe.prepared.fetch_add(1, Ordering::AcqRel);
        self.probe.requests.lock().unwrap().push(request);
        Box::new(FakeSession {
            probe: self.probe.clone(),
            active: AtomicBool::new(false),
        })
    }
    fn carrier(&self) -> (AudioWriter<f32>, AudioReader<f32>) {
        audio_bridge(2, if self.short_carrier { 1 } else { 16_384 }, 1 << 20).unwrap()
    }
}
impl VirtualBoundaryProvider for FakeProvider {
    fn prepare_playback(
        &self,
        request: VirtualBoundaryRequest,
        cancel: &CancellationToken,
    ) -> Result<PreparedVirtualPlayback, BackendError> {
        let (mut writer, reader) = self.carrier();
        if !self.short_carrier {
            let samples: Vec<_> = (0..8192).flat_map(|_| [0.25, -0.5]).collect();
            writer.write_interleaved(&samples).unwrap();
        }
        self.probe.playback.lock().unwrap().push(writer);
        if self.cancel_after_playback {
            cancel.cancel();
        }
        Ok(PreparedVirtualPlayback {
            carrier: reader,
            reset_epoch: Arc::new(AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            probe: Arc::new(BoundaryProbe::default()),
            session: self.session(request),
        })
    }
    fn prepare_microphone(
        &self,
        request: VirtualBoundaryRequest,
        _: &CancellationToken,
    ) -> Result<PreparedVirtualMicrophone, BackendError> {
        if self.fail_microphone {
            return Err(BackendError::ResourceUnavailable(
                "injected microphone preparation".into(),
            ));
        }
        let (writer, reader) = self.carrier();
        self.probe.microphones.lock().unwrap().push(reader);
        Ok(PreparedVirtualMicrophone {
            carrier: writer,
            reset_epoch: Arc::new(AtomicU64::new(0)),
            producer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            consumer_quiesced_epoch: Arc::new(AtomicU64::new(0)),
            probe: Arc::new(BoundaryProbe::default()),
            session: self.session(request),
        })
    }
}

struct FakeSession {
    probe: Arc<Probe>,
    active: AtomicBool,
}
impl VirtualResourceSession for FakeSession {
    fn activate(&self) -> Result<(), BackendError> {
        self.probe.activated.fetch_add(1, Ordering::AcqRel);
        self.active.store(true, Ordering::Release);
        Ok(())
    }
    fn cancel(&self) {
        self.probe.cancelled.fetch_add(1, Ordering::AcqRel);
        self.active.store(false, Ordering::Release);
    }
    fn status(&self) -> OwnerStatus {
        if self.probe.fail_session.load(Ordering::Acquire) {
            OwnerStatus::Failed(session_error())
        } else if self.active.load(Ordering::Acquire) {
            OwnerStatus::Running
        } else {
            OwnerStatus::Starting
        }
    }
    fn stop_and_join(self: Box<Self>) -> Result<(), BackendError> {
        self.probe.joined.fetch_add(1, Ordering::AcqRel);
        if self.probe.fail_session.load(Ordering::Acquire) {
            Err(session_error())
        } else {
            Ok(())
        }
    }
}
impl Drop for FakeSession {
    fn drop(&mut self) {
        self.probe.dropped.fetch_add(1, Ordering::AcqRel);
    }
}
pub(super) fn session_error() -> BackendError {
    BackendError::ResourceUnavailable("injected VAD session failure".into())
}

pub(super) struct FakeClockFactory(pub Arc<Probe>);
impl VirtualClockFactory for FakeClockFactory {
    fn prepare(&self, _: crate::StopSignal) -> Result<Box<dyn VirtualRealtimeClock>, BackendError> {
        self.0.clock_prepared.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(FakeClock { now: 0, waits: 0 }))
    }
}
struct FakeClock {
    now: u64,
    waits: usize,
}
impl VirtualRealtimeClock for FakeClock {
    fn frequency(&self) -> u64 {
        96_000
    }
    fn now_ticks(&mut self) -> Result<u64, BackendError> {
        Ok(self.now)
    }
    fn wait_ticks(&mut self, ticks: u64) -> Result<ClockWake, BackendError> {
        // Finite even after an assertion unwinds; defensive owner Drop can join.
        if self.waits == 16 {
            return Ok(ClockWake::Stopped);
        }
        self.waits += 1;
        self.now += ticks;
        std::thread::sleep(Duration::from_millis(1));
        Ok(ClockWake::Timer)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Faults {
    pub open: bool,
    pub wait: bool,
    pub stop: bool,
    pub release: bool,
    pub drop: bool,
    pub recover: bool,
    pub wrong_rate: bool,
    pub wrong_format: bool,
    pub wrong_period: bool,
}
#[derive(Default)]
pub(super) struct NativeProbe {
    pub opened: AtomicUsize,
    pub started: AtomicUsize,
    pub stopped: AtomicUsize,
    pub released: AtomicUsize,
    pub dropped: AtomicUsize,
    pub written: AtomicUsize,
    pub master_groups: AtomicUsize,
}
pub(super) struct FakeNativeDevices {
    pub probe: Arc<NativeProbe>,
    pub faults: Faults,
}
impl NativeDevices for FakeNativeDevices {
    fn resolve(
        &self,
        selector: &EndpointSelector,
        _: DeviceOptions,
        _: bool,
    ) -> Result<String, BackendError> {
        match selector {
            EndpointSelector::PinnedEndpoint { endpoint_id, .. } => Ok(endpoint_id.clone()),
            _ => Err(unsupported("fake requires pinned identity")),
        }
    }
    fn factory(&self, group: &Group, rate: u32) -> Result<StreamFactory, BackendError> {
        // Keep duplex validation in the fake path as well.
        if group.backend == AudioBackend::Asio {
            asio_session_options(
                group.input.as_ref().map(|d| d.device),
                group.output.as_ref().map(|d| d.device),
                rate,
            )?;
        }
        self.probe
            .master_groups
            .fetch_add(usize::from(group.master()), Ordering::AcqRel);
        let probe = self.probe.clone();
        let faults = self.faults;
        let device = group
            .input
            .as_ref()
            .or(group.output.as_ref())
            .unwrap()
            .device;
        let info = NativeStreamInfo {
            sample_rate: if faults.wrong_rate {
                32_000
            } else {
                device.hardware_sample_rate.unwrap_or(rate)
            },
            format: if faults.wrong_format {
                SampleFormat::Pcm16
            } else {
                match device.sample_format {
                    SampleFormat::Native => SampleFormat::Float32,
                    format => format,
                }
            },
            buffer_frames: if faults.wrong_period {
                512
            } else {
                device.period_frames.unwrap_or(128) as u32
            },
            channels: 2,
        };
        Ok(Box::new(move || {
            probe.opened.fetch_add(1, Ordering::AcqRel);
            if faults.open {
                return Err(native_error("open"));
            }
            Ok(Box::new(FakeNative {
                info,
                probe,
                faults,
            }))
        }))
    }
}
struct FakeNative {
    info: NativeStreamInfo,
    probe: Arc<NativeProbe>,
    faults: Faults,
}
impl NativeStream for FakeNative {
    fn info(&self) -> NativeStreamInfo {
        self.info
    }
    fn start(&mut self) -> Result<(), BackendError> {
        self.probe.started.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        self.probe.stopped.fetch_add(1, Ordering::AcqRel);
        if self.faults.stop {
            Err(native_error("stop"))
        } else {
            Ok(())
        }
    }
    fn release(&mut self) -> Result<(), BackendError> {
        self.probe.released.fetch_add(1, Ordering::AcqRel);
        if self.faults.release {
            Err(native_error("release"))
        } else {
            Ok(())
        }
    }
    fn recovery_request(&self) -> Option<NativeRecoveryRequest> {
        self.faults.recover.then_some(NativeRecoveryRequest::Reset)
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        if self.faults.wait {
            return Err(native_error("wait"));
        }
        std::thread::sleep(Duration::from_millis(1));
        Ok(true)
    }
    fn read(&mut self, output: &mut [StereoFrame]) -> Result<usize, BackendError> {
        output.fill([0.25, -0.5]);
        Ok(output.len())
    }
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        self.probe.written.fetch_add(input.len(), Ordering::AcqRel);
        Ok(input.len())
    }
}
impl Drop for FakeNative {
    fn drop(&mut self) {
        self.probe.dropped.fetch_add(1, Ordering::AcqRel);
        assert!(!self.faults.drop, "injected native drop panic");
    }
}
pub(super) fn native_error(phase: &str) -> BackendError {
    BackendError::Failed(format!("injected native {phase} failure"))
}
pub(super) fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "fake owner watchdog");
        std::thread::yield_now();
    }
}
