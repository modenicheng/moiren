use super::*;
use crate::vad::{Capabilities, EndpointSnapshot};

fn capabilities() -> Capabilities {
    let mut bytes = [0; 64];
    for (offset, value) in [
        (0, 64_u32),
        (4, 2),
        (8, 31),
        (12, 64),
        (16, 1),
        (20, 48_000),
        (24, 3),
        (28, 6),
        (32, 6),
        (36, 4),
        (48, 4800),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[40..48].copy_from_slice(&7_u64.to_le_bytes());
    Capabilities::decode(&bytes).unwrap()
}

fn snapshot(stream: StreamState, frames: u32, generation: u64, gate: u64) -> EndpointSnapshot {
    let mut bytes = [0; 128];
    for (offset, value) in [
        (0, 128_u32),
        (4, 2),
        (16, PdoState::Present as u32),
        (20, Intent::Microphone as u32),
        (24, 2),
        (28, 1),
        (32, 1),
        (36, 1),
        (88, SampleFormat::Float32 as u32),
        (92, frames),
        (100, stream as u32),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [(8, 11_u64), (40, 7), (48, 11), (56, generation), (64, gate)] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[72..88].fill(0x31);
    EndpointSnapshot::decode(&bytes, capabilities()).unwrap()
}

fn patch(snapshot: EndpointSnapshot, offset: usize, value: u32) -> EndpointSnapshot {
    // Keep test snapshots protocol-validated, including malformed-but-decodable
    // native readiness (e.g. Paused with no current packet shape).
    let mut bytes = [0; 128];
    for (offset, value) in [
        (0, 128_u32),
        (4, 2),
        (16, snapshot.state() as u32),
        (20, snapshot.intent() as u32),
        (24, snapshot.channels() as u32),
        (28, u32::from(snapshot.gate_open())),
        (32, u32::from(snapshot.policy_prepared())),
        (36, u32::from(snapshot.audio_ready())),
        (
            88,
            snapshot.sample_format().map_or(0, |format| format as u32),
        ),
        (92, snapshot.packet_frames()),
        (100, snapshot.stream_state() as u32),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (8, snapshot.pdo_token().get()),
        (40, snapshot.key().session_epoch()),
        (48, snapshot.endpoint_generation()),
        (56, snapshot.stream_generation()),
        (64, snapshot.gate_epoch()),
    ] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[72..88].copy_from_slice(&snapshot.stable_id().to_le_bytes());
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    EndpointSnapshot::decode(&bytes, capabilities()).unwrap()
}

struct FakeControl {
    current: EndpointSnapshot,
    queries: usize,
}

impl lifecycle::EndpointControl for FakeControl {
    fn create(&mut self, _: &ExportSpec) -> Result<Option<EndpointSnapshot>, BackendError> {
        unreachable!("startup polling must not create endpoints")
    }

    fn query(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot, BackendError> {
        self.queries += 1;
        self.current
            .require_progress_after(previous)
            .map_err(|error| client_error(error.into()))
    }

    fn gate(&mut self, _: EndpointSnapshot, open: bool) -> Result<EndpointSnapshot, BackendError> {
        assert!(!open, "the worker never opens gates");
        // A revoked worker cannot proceed to PCM even if the fake QUERY still
        // observes an open gate. Native close behavior is tested by lifecycle.
        Ok(self.current)
    }

    fn remove(&mut self, _: EndpointSnapshot) -> Result<EndpointSnapshot, BackendError> {
        unreachable!("startup polling must not remove endpoints")
    }
}

struct Startup {
    control: FakeControl,
    current: EndpointSnapshot,
    io: FakeIo,
    clock: FakeClock,
    state: SessionState,
    reader: Option<AudioReader<f32>>,
    writer: AudioWriter<f32>,
    started: bool,
}

impl Startup {
    fn new(frames: u32) -> Self {
        let current = snapshot(StreamState::Paused, frames, 1, 1);
        let state = state();
        state.active.store(true, Ordering::Release);
        let ticket = state.admission.set(Some(9));
        state.admitted_ticket.store(ticket, Ordering::Release);
        state.admitted_revision.store(9, Ordering::Release);
        let (writer, reader) = audio_bridge(2, 4800, 1 << 20).unwrap();
        Self {
            control: FakeControl {
                current,
                queries: 0,
            },
            current,
            io: FakeIo::default(),
            clock: FakeClock::default(),
            state,
            reader: Some(reader),
            writer,
            started: false,
        }
    }

    fn poll(&mut self) -> Result<(), BackendError> {
        self.current = transport::poll_with_started_transport(
            &mut self.control,
            self.current,
            &self.state,
            self.started,
        )?
        .0;
        Ok(())
    }

    fn wait(&self) {
        transport::wait_for_microphone_poll(
            self.current,
            capabilities(),
            &self.state,
            self.started,
            0,
            |duration| self.clock.sleep(duration),
        );
    }

    fn first_write(&mut self) -> Result<(), BackendError> {
        if !transport::microphone_running(self.current, &self.state) {
            return Ok(());
        }
        let shape = self
            .current
            .pcm_shape(capabilities())
            .map_err(|error| client_error(error.into()))?;
        let mut bridge = PcmMicrophoneBridge::new(
            Channels::Stereo,
            shape.format(),
            shape.frames() as usize,
            1 << 20,
            self.reader.take().unwrap(),
        )
        .unwrap();
        let mut pump = MicrophonePump::new(shape.frames());
        microphone(
            &mut self.io,
            &mut bridge,
            &mut vec![],
            &mut pump,
            &self.state,
            0,
            || self.clock.now(),
        )?;
        self.reader = Some(bridge.into_carrier());
        Ok(())
    }
}

#[test]
fn first_microphone_startup_poll_does_not_delay_first_submit_by_ten_ms() {
    let mut f = Startup::new(144);
    f.writer.write_interleaved_exact(&vec![0.25; 288]).unwrap();
    let start = f.clock.now();
    f.poll().unwrap();
    f.first_write().unwrap();
    assert!(f.io.written.is_empty(), "Paused never encodes or WRITEs");
    assert_eq!(f.reader.as_ref().unwrap().available_frames(), 144);

    // Running begins just after the Paused QUERY. Only the next real QUERY
    // may authorize pumping; the wait must not inherit the old 10 ms delay.
    f.control.current = snapshot(StreamState::Running, 144, 1, 1);
    f.wait();
    assert!(f.io.written.is_empty());
    f.poll().unwrap();
    f.first_write().unwrap();
    assert_eq!(f.control.queries, 2);
    assert_eq!(f.io.written, [0.25_f32.to_le_bytes().repeat(288)]);
    assert_eq!(
        f.clock.now().duration_since(start),
        Duration::from_micros(1500),
        "144 frames at 48 kHz: re-query after half a packet, not 10 ms"
    );
}

#[test]
fn first_microphone_startup_wait_is_positive_packet_aware_and_bounded() {
    for (frames, expected) in [
        (1, Duration::from_nanos(10_417)),
        (144, Duration::from_micros(1500)),
        (480, Duration::from_millis(5)),
        (4800, POLL_INTERVAL),
    ] {
        let mut f = Startup::new(frames);
        f.poll().unwrap();
        f.wait();
        assert_eq!(*f.clock.sleeps.borrow(), [expected]);
        assert!(!expected.is_zero() && expected <= POLL_INTERVAL);
        assert!(f.io.written.is_empty());
    }
}

#[test]
fn first_microphone_startup_wait_does_not_change_inactive_invalid_or_recovery_polling() {
    for case in 0..12 {
        let mut f = Startup::new(144);
        match case {
            0 => f.started = true,
            1 => f.state.active.store(false, Ordering::Release),
            2 => f.state.cancel.store(true, Ordering::Release),
            3 => f.state.drained.store(true, Ordering::Release),
            4 => f.state.revoke(),
            5 => f.current = patch(f.current, 28, 0),
            6 => f.current = patch(f.current, 92, 0),
            7 => f.current = patch(f.current, 20, Intent::Playback as u32),
            8 => f.current = patch(f.current, 100, StreamState::Running as u32),
            9 => f.state.reset_epoch.store(1, Ordering::Release),
            10 => f.state.producer_quiesced_epoch.store(1, Ordering::Release),
            11 => f.state.consumer_quiesced_epoch.store(1, Ordering::Release),
            _ => unreachable!(),
        }
        f.wait();
        assert_eq!(*f.clock.sleeps.borrow(), [POLL_INTERVAL], "case {case}");
        assert!(f.io.written.is_empty());
    }
    let f = Startup::new(144);
    transport::wait_for_microphone_poll(f.current, capabilities(), &f.state, false, 1, |wait| {
        assert_eq!(wait, POLL_INTERVAL, "a completed reset is not first start")
    });
}

#[test]
fn first_microphone_startup_requeries_and_accepts_legal_initial_generation_and_shape() {
    let mut f = Startup::new(144);
    f.writer.write_interleaved_exact(&vec![0.5; 960]).unwrap();
    f.poll().unwrap();
    f.wait();
    f.first_write().unwrap();
    assert!(f.io.written.is_empty(), "a sleep cannot authorize Running");
    assert_eq!(f.reader.as_ref().unwrap().available_frames(), 480);
    // No owner exists yet: retain the existing initial-transition exception,
    // rather than interpreting a new native shape/generation as stale IO.
    f.control.current = snapshot(StreamState::Running, 480, 2, 2);
    f.poll().unwrap();
    f.first_write().unwrap();
    assert_eq!(f.control.queries, 2);
    assert_eq!(f.io.written, [0.5_f32.to_le_bytes().repeat(960)]);
    assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 0);
    assert_eq!(f.state.telemetry.snapshot().recovery_events, 0);
}

#[test]
fn first_microphone_startup_cancel_gate_and_reset_after_wait_do_not_write() {
    for case in 0..6 {
        let mut f = Startup::new(144);
        f.writer.write_interleaved_exact(&vec![0.5; 288]).unwrap();
        f.poll().unwrap();
        f.wait();
        f.control.current = snapshot(StreamState::Running, 144, 1, 1);
        match case {
            0 => f.state.cancel.store(true, Ordering::Release),
            1 => f.control.current = patch(f.control.current, 28, 0),
            2 => f.state.revoke(),
            3 => f.state.reset_epoch.store(1, Ordering::Release),
            4 => f.state.producer_quiesced_epoch.store(1, Ordering::Release),
            5 => f.state.consumer_quiesced_epoch.store(1, Ordering::Release),
            _ => unreachable!(),
        }
        f.poll().unwrap();
        f.first_write().unwrap();
        assert!(f.io.written.is_empty(), "case {case}");
        assert_eq!(f.reader.as_ref().unwrap().available_frames(), 144);
    }
}

#[test]
fn first_microphone_startup_invalid_fresh_shape_does_not_write() {
    let mut f = Startup::new(144);
    f.poll().unwrap();
    f.wait();
    f.control.current = snapshot(StreamState::Running, 0, 1, 1);
    f.poll().unwrap();
    assert!(f.first_write().is_err());
    assert!(f.io.written.is_empty());
}

#[test]
fn first_microphone_startup_does_not_exempt_old_owner_generation_or_gate_reset() {
    for (generation, gate) in [(2, 1), (1, 2)] {
        let mut f = Startup::new(144);
        f.started = true;
        f.control.current = snapshot(StreamState::Running, 144, generation, gate);
        f.poll().unwrap();
        f.first_write().unwrap();
        assert_eq!(f.state.reset_epoch.load(Ordering::Acquire), 1);
        assert_eq!(f.state.telemetry.snapshot().recovery_events, 1);
        assert!(f.io.written.is_empty());
        f.wait();
        assert_eq!(*f.clock.sleeps.borrow(), [POLL_INTERVAL]);
    }
}
