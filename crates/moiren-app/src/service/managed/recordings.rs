//! Recording orchestration: engine leases and worker sessions live here, the
//! context-side domain mirrors them for snapshots. File IO, finalization and
//! session drops always run on the recording worker or the cleanup lane, never
//! on the service loop or UI thread.
use super::*;
use crate::service::recording::{local_date_time, recording_file_path};
use moiren_core::graph::NodeKind;
use moiren_engine::recorder::{RecordingSession, RecordingState};

/// One live recording: the worker session plus the cleanup slot reserved for
/// its eventual drop, mirroring how the lifecycle owner reserves before it
/// opens a host, so disposal never needs additional storage.
pub(super) struct OwnedRecording {
    session: RecordingSession,
    cleanup: DeferredPermit,
}

impl ManagedDriver {
    pub(super) fn start_recording(
        &mut self,
        c: &mut ServiceContext,
        node: NodeId,
        directory: std::path::PathBuf,
        format: crate::service::RecordingFormat,
        mp3: moiren_engine::recorder::Mp3Encoding,
        flac: moiren_engine::recorder::FlacEncoding,
    ) {
        if c.recordings
            .phase(node)
            .is_some_and(|phase| !phase.is_terminal())
        {
            // Double start; the UI hides active controls, and a second lease
            // would fail anyway. Keep the first file running.
            return;
        }
        let Some(owner) = self.owner.as_ref() else {
            c.recordings
                .fail_start(node, "audio engine is not running".into());
            return;
        };
        if c.core.phase() != SessionPhase::Running {
            c.recordings
                .fail_start(node, "start audio before recording".into());
            return;
        }
        if c.graph.snapshot_ref().kind_of(node) != Some(NodeKind::Recorder) {
            c.recordings
                .fail_start(node, "node is not a recorder".into());
            return;
        }
        let Ok(cleanup) = c.try_reserve_cleanup() else {
            c.recordings
                .fail_start(node, "too many pending cleanups".into());
            return;
        };
        if let Err(error) = std::fs::create_dir_all(&directory) {
            c.recordings
                .fail_start(node, format!("cannot create recording directory: {error}"));
            return;
        }
        // `record_wav_now` discards the preroll queued since graph start and
        // waits out any in-flight observer block, which can park this thread
        // for up to one audio block. This is a user-initiated one-off on the
        // service loop, never a per-poll cost.
        let sample_rate = owner.audio_settings.processing.sample_rate;
        let reader = match owner.observer.recorder_reader(node) {
            Ok(reader) => reader,
            Err(error) => {
                c.recordings.fail_start(node, error);
                return;
            }
        };
        let path = recording_file_path(&directory, local_date_time(), format, |p| p.exists());
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Both entry points claim the lease, create the sink first (format
        // errors return the reader without changing capture), discard the
        // preroll queued since graph start, then spawn the worker.
        let started = match format {
            crate::service::RecordingFormat::Wav => reader.record_wav_now(&path, sample_rate),
            crate::service::RecordingFormat::Mp3 => reader.record_mp3_now(&path, sample_rate, mp3),
            crate::service::RecordingFormat::Flac => {
                reader.record_flac_now(&path, sample_rate, flac)
            }
        };
        match started {
            Ok(session) => {
                c.recordings
                    .begin(node, file_name, path.clone(), sample_rate);
                self.recording_sessions
                    .insert(node, OwnedRecording { session, cleanup });
                log::info!(
                    target: "moiren::audio::service",
                    "recording started: node={node:?} path={}",
                    path.display(),
                );
            }
            Err(error) => {
                c.recordings
                    .fail_start(node, format!("cannot open recording file: {error}"));
            }
        }
    }

    pub(super) fn pause_recording(&mut self, c: &mut ServiceContext, node: NodeId) {
        if let Some(recording) = self.recording_sessions.get(&node) {
            recording.session.pause();
            c.recordings.set_paused(node, true);
        }
    }

    pub(super) fn resume_recording(&mut self, c: &mut ServiceContext, node: NodeId) {
        if let Some(recording) = self.recording_sessions.get(&node) {
            recording.session.resume();
            c.recordings.set_paused(node, false);
        }
    }

    pub(super) fn stop_recording(&mut self, c: &mut ServiceContext, node: NodeId) {
        if let Some(recording) = self.recording_sessions.get(&node) {
            recording.session.stop();
            c.recordings.set_stopping(node);
        }
    }

    /// Fold worker statuses into the domain and retire terminal sessions.
    /// Runs every poll; `status()` is a mutex-guarded clone of a small
    /// struct, and there is one session per recorder node at most.
    pub(super) fn poll_recordings(&mut self, c: &mut ServiceContext) {
        let mut terminal = Vec::new();
        for (node, recording) in &self.recording_sessions {
            let status = recording.session.status();
            c.recordings.observe(*node, &status);
            if matches!(
                status.state,
                RecordingState::Finished | RecordingState::Failed
            ) {
                terminal.push(*node);
            }
        }
        for node in terminal {
            if let Some(recording) = self.recording_sessions.remove(&node) {
                self.dispose_recording(c, recording);
            }
        }
    }

    /// Engine ownership is going away: stop every live session (their workers
    /// finalize independently of the host) and settle the domain. Drops run on
    /// the cleanup lane like the owner's.
    pub(super) fn retire_recordings(&mut self, c: &mut ServiceContext) {
        for (_, recording) in std::mem::take(&mut self.recording_sessions) {
            recording.session.stop();
            self.dispose_recording(c, recording);
        }
        c.recordings.settle_all();
    }

    /// Dropping a session joins its worker, so it belongs on the cleanup
    /// lane. The reserved permit guarantees admission; a terminal session's
    /// join is immediate (its worker already exited).
    fn dispose_recording(&mut self, c: &mut ServiceContext, recording: OwnedRecording) {
        let OwnedRecording { session, cleanup } = recording;
        let job: Job = Box::new(move || drop(session));
        if c.defer_reserved_cleanup(cleanup, job).is_err() {
            unreachable!("recording cleanup permit belongs to context");
        }
    }
}
