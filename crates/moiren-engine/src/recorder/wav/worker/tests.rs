use super::*;
use crate::recorder::{
    WavFormat, recorder,
    tests::{observe, settings},
};
use std::io::Cursor;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
struct Destination {
    bytes: Arc<Mutex<Cursor<Vec<u8>>>>,
    fail_write: Arc<AtomicBool>,
    fail_flush: Arc<AtomicBool>,
}

impl Write for Destination {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_write.load(Ordering::Acquire) {
            return Err(io::Error::other("injected recording data failure"));
        }
        self.bytes.lock().unwrap().write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush.load(Ordering::Acquire) {
            return Err(io::Error::other("injected recording flush failure"));
        }
        Ok(())
    }
}

impl Seek for Destination {
    fn seek(&mut self, position: std::io::SeekFrom) -> io::Result<u64> {
        self.bytes.lock().unwrap().seek(position)
    }
}

fn wait_for(session: &RecordingSession, predicate: impl Fn(&RecordingStatus) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = session.status();
        if predicate(&status) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out: {status:?}");
        std::thread::yield_now();
    }
}

fn assert_wav(destination: &Destination, expected: &[f32]) {
    let storage = destination.bytes.lock().unwrap();
    let bytes = storage.get_ref();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
        bytes.len() - 8
    );
    assert_eq!(
        u32::from_le_bytes(bytes[46..50].try_into().unwrap()) as usize,
        expected.len()
    );
    assert_eq!(
        u32::from_le_bytes(bytes[54..58].try_into().unwrap()) as usize,
        expected.len() * 4
    );
    let samples: Vec<f32> = bytes[58..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| f32::from_le_bytes(*sample))
        .collect();
    assert_eq!(samples, expected);
}

#[test]
fn worker_drains_while_running_and_finish_preserves_queued_tail() {
    let (mut recorder, reader) = recorder(1, settings(16)).unwrap();
    observe::<f32>(&mut recorder, &[&[1.0, 2.0]]);
    let destination = Destination::default();
    let session = reader
        .record_with_writer(destination.clone(), 48_000)
        .unwrap();
    wait_for(&session, |status| status.written_frames == 2);
    observe::<f32>(&mut recorder, &[&[3.0, 4.0]]);
    let summary = session.finish().unwrap();
    assert_eq!(summary.written_frames, 4);
    assert_eq!(summary.stats.accepted_frames, 4);
    assert_eq!(summary.stats.read_frames, 4);
    assert_wav(&destination, &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn worker_encodes_selected_pcm_format_and_finalizes_odd_tail() {
    for (format, width) in [(WavFormat::Pcm16, 2), (WavFormat::Pcm24, 3)] {
        let (mut recorder, reader) = recorder(1, settings(8)).unwrap();
        let destination = Destination::default();
        let session = reader
            .record_with_writer_format(destination.clone(), 48_000, format)
            .unwrap();
        observe::<f64>(&mut recorder, &[&[-1.0, 1.0, 0.0]]);
        wait_for(&session, |status| status.written_frames == 3);
        let summary = session.finish().unwrap();
        assert_eq!(summary.written_frames, 3);
        assert_eq!(summary.stats.read_frames, 3);
        let storage = destination.bytes.lock().unwrap();
        let bytes = storage.get_ref();
        assert_eq!(
            u16::from_le_bytes(bytes[20..22].try_into().unwrap()),
            0xfffe
        );
        assert_eq!(
            u32::from_le_bytes(bytes[64..68].try_into().unwrap()),
            3 * width as u32
        );
        let expected = if format == WavFormat::Pcm16 {
            vec![0, 128, 255, 127, 0, 0]
        } else {
            vec![0, 0, 128, 255, 255, 127, 0, 0, 0, 0]
        };
        assert_eq!(&bytes[68..], expected);
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len() - 8
        );
    }
}

#[test]
fn worker_finalizes_after_source_disconnect_without_explicit_stop() {
    let (mut recorder, reader) = recorder(1, settings(8)).unwrap();
    observe::<f64>(&mut recorder, &[&[0.25, -0.5]]);
    let destination = Destination::default();
    let session = reader
        .record_with_writer(destination.clone(), 48_000)
        .unwrap();
    drop(recorder);
    wait_for(&session, |status| status.state == RecordingState::Finished);
    assert_eq!(session.finish().unwrap().written_frames, 2);
    assert_wav(&destination, &[0.25, -0.5]);
}

#[test]
fn worker_latches_data_failure_stops_capture_and_reports_finish_error() {
    let (mut recorder, reader) = recorder(1, settings(8)).unwrap();
    let destination = Destination::default();
    let session = reader
        .record_with_writer(destination.clone(), 48_000)
        .unwrap();
    destination.fail_write.store(true, Ordering::Release);
    observe::<f32>(&mut recorder, &[&[1.0, 2.0]]);
    wait_for(&session, |status| status.state == RecordingState::Failed);
    let status = session.status();
    assert_eq!(status.written_frames, 0);
    assert_eq!(status.stats.state, crate::recorder::RecorderState::Stopped);
    assert!(
        status
            .error
            .unwrap()
            .message
            .contains("injected recording data failure")
    );
    assert!(session.finish().is_err());
}

#[test]
fn worker_reports_finalization_failure_and_drop_joins_finalization() {
    let (recorder, reader) = recorder(1, settings(8)).unwrap();
    let destination = Destination::default();
    let session = reader
        .record_with_writer(destination.clone(), 48_000)
        .unwrap();
    destination.fail_flush.store(true, Ordering::Release);
    assert!(
        session
            .finish()
            .unwrap_err()
            .to_string()
            .contains("flush failure")
    );
    drop(recorder);

    let (mut recorder, reader) = crate::recorder::recorder(1, settings(8)).unwrap();
    observe::<f32>(&mut recorder, &[&[0.75]]);
    let destination = Destination::default();
    drop(
        reader
            .record_with_writer(destination.clone(), 48_000)
            .unwrap(),
    );
    assert_wav(&destination, &[0.75]);
}

struct RevocableSource {
    reader: RecorderReader,
    active: Arc<AtomicBool>,
    returned: Arc<AtomicBool>,
}

impl RecordingSource for RevocableSource {
    fn reader(&mut self) -> &mut RecorderReader {
        &mut self.reader
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

impl Drop for RevocableSource {
    fn drop(&mut self) {
        self.returned.store(true, Ordering::Release);
    }
}

#[test]
fn revoked_source_stops_and_returns_lease_before_terminal_publication() {
    let (mut recorder, reader) = recorder(1, settings(8)).unwrap();
    observe::<f32>(&mut recorder, &[&[0.5, -0.75]]);
    let active = Arc::new(AtomicBool::new(true));
    let returned = Arc::new(AtomicBool::new(false));
    let destination = Destination::default();
    let wav = WavRecording::from_writer(destination.clone(), 1, 48_000).unwrap();
    let source = RevocableSource {
        reader,
        active: Arc::clone(&active),
        returned: Arc::clone(&returned),
    };
    let session = spawn_recording(source, wav).unwrap();
    active.store(false, Ordering::Release);
    wait_for(&session, |status| status.state == RecordingState::Finished);
    assert!(returned.load(Ordering::Acquire));
    assert_eq!(session.finish().unwrap().written_frames, 2);
    assert_wav(&destination, &[0.5, -0.75]);
}

struct PanickingWriter(Destination);

impl Write for PanickingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.fail_write.load(Ordering::Acquire) {
            panic!("injected destination panic");
        }
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Seek for PanickingWriter {
    fn seek(&mut self, position: std::io::SeekFrom) -> io::Result<u64> {
        self.0.seek(position)
    }
}

#[test]
fn destination_panic_is_observable_and_returns_stopped_source() {
    let (mut recorder, reader) = recorder(1, settings(8)).unwrap();
    let destination = Destination::default();
    let active = Arc::new(AtomicBool::new(true));
    let returned = Arc::new(AtomicBool::new(false));
    let source = RevocableSource {
        reader,
        active,
        returned: Arc::clone(&returned),
    };
    let wav = WavRecording::from_writer(PanickingWriter(destination.clone()), 1, 48_000).unwrap();
    let session = spawn_recording(source, wav).unwrap();
    destination.fail_write.store(true, Ordering::Release);
    observe::<f32>(&mut recorder, &[&[1.0]]);
    wait_for(&session, |status| status.state == RecordingState::Failed);
    assert!(returned.load(Ordering::Acquire));
    assert_eq!(
        session.status().stats.state,
        crate::recorder::RecorderState::Stopped
    );
    assert!(
        session
            .finish()
            .unwrap_err()
            .to_string()
            .contains("panicked")
    );
}

type ConversionGate = (
    std::sync::mpsc::SyncSender<()>,
    std::sync::mpsc::Receiver<()>,
);
thread_local! {
    static CONVERSION_GATE: std::cell::RefCell<Option<ConversionGate>> = const {
        std::cell::RefCell::new(None)
    };
}

#[derive(Clone, Copy)]
struct GatedSample(f64);

impl crate::sample::ProcessingSample for GatedSample {
    const ZERO: Self = Self(0.0);

    fn from_f64(value: f64) -> Self {
        Self(value)
    }

    fn to_f64(self) -> f64 {
        CONVERSION_GATE.with(|gate| {
            if let Some((entered, release)) = gate.borrow_mut().take() {
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
        });
        self.0
    }
}

#[test]
fn stop_waits_for_inflight_publication_before_finalizing_wav_tail() {
    let (mut recorder, reader) = recorder(1, settings(8)).unwrap();
    let destination = Destination::default();
    let session = reader
        .record_with_writer(destination.clone(), 48_000)
        .unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let observer = std::thread::spawn(move || {
        CONVERSION_GATE.with(|gate| *gate.borrow_mut() = Some((entered_tx, release_rx)));
        observe::<GatedSample>(&mut recorder, &[&[0.25, -0.5]]);
        recorder
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    session.stop();
    wait_for(&session, |status| status.state != RecordingState::Recording);
    let while_inflight = session.status();
    release_tx.send(()).unwrap();
    let recorder = observer.join().unwrap();
    let summary = session.finish().unwrap();
    assert_eq!(while_inflight.state, RecordingState::Stopping);
    assert_eq!(while_inflight.written_frames, 0);
    assert_eq!(summary.written_frames, 2);
    assert_wav(&destination, &[0.25, -0.5]);
    drop(recorder);
}

#[test]
fn unstarted_worker_guard_settles_inflight_capture_before_returning_source() {
    let (mut recorder, reader) = recorder(1, settings(8)).unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let observer = std::thread::spawn(move || {
        CONVERSION_GATE.with(|gate| *gate.borrow_mut() = Some((entered_tx, release_rx)));
        observe::<GatedSample>(&mut recorder, &[&[0.25]]);
        recorder
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (returned_tx, returned_rx) = std::sync::mpsc::sync_channel(1);
    let guard_owner = std::thread::spawn(move || {
        // Thread spawn failure drops this guard without ever entering run().
        drop(SourceGuard(reader));
        returned_tx.send(()).unwrap();
    });
    let premature = returned_rx.recv_timeout(Duration::from_millis(100)).is_ok();
    release_tx.send(()).unwrap();
    guard_owner.join().unwrap();
    drop(observer.join().unwrap());
    assert!(
        !premature,
        "unstarted worker returned the source during publication"
    );
}
