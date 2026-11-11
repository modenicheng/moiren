//! Runtime recording domain. Sessions live in the managed driver against
//! engine leases; this context-side state is what snapshots project. It is
//! deliberately kept out of `ServiceGraph` so recording never enters the edit
//! journal, and every continuously advancing value is quantized (whole
//! seconds) so publishing stays at 1 Hz while a recording runs.
use moiren_core::graph::NodeId;
use moiren_engine::recorder::{RecordingState, RecordingStatus};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Encoded-output preference. WAV and FLAC are lossless; MP3 is CBR 256
/// kbps, mono or stereo only. The format is resolved when a recording starts,
/// so changing it never disturbs a running session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RecordingFormat {
    #[default]
    Wav,
    Mp3,
    Flac,
}

impl RecordingFormat {
    /// Extension of the generated recording file name.
    pub fn extension(self) -> &'static str {
        match self {
            RecordingFormat::Wav => "wav",
            RecordingFormat::Mp3 => "mp3",
            RecordingFormat::Flac => "flac",
        }
    }
}

/// Discrete lifecycle of one node's recording. Terminal phases keep their
/// entry until the next start replaces it, so the file name stays visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingPhase {
    Recording,
    Paused,
    Stopping,
    Finished,
    Failed,
}

impl RecordingPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Finished | Self::Failed)
    }
}

/// Per-node projection for detail surfaces (inspector): the card variant is
/// [`RecordingCard`] because graph-page keys may only carry discrete values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSnapshot {
    pub node: NodeId,
    pub phase: RecordingPhase,
    pub file_name: String,
    pub path: PathBuf,
    pub sample_rate: u32,
    /// Whole seconds of recorded audio; pause gaps are excluded because the
    /// engine skips paused frames entirely.
    pub recorded_secs: u64,
    pub dropped_frames: u64,
    pub error: Option<String>,
}

/// Discrete per-node projection for the graph canvas: phase and file name
/// only, never the ticking duration (graph-page cache keys must stay free of
/// per-second values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingCard {
    pub node: NodeId,
    pub phase: RecordingPhase,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordingEntry {
    phase: RecordingPhase,
    file_name: String,
    path: PathBuf,
    sample_rate: u32,
    recorded_secs: u64,
    dropped_frames: u64,
    error: Option<String>,
}

impl RecordingEntry {
    fn failed(error: String) -> Self {
        Self {
            phase: RecordingPhase::Failed,
            file_name: String::new(),
            path: PathBuf::new(),
            sample_rate: 0,
            recorded_secs: 0,
            dropped_frames: 0,
            error: Some(error),
        }
    }

    fn snapshot(&self, node: NodeId) -> RecordingSnapshot {
        RecordingSnapshot {
            node,
            phase: self.phase,
            file_name: self.file_name.clone(),
            path: self.path.clone(),
            sample_rate: self.sample_rate,
            recorded_secs: self.recorded_secs,
            dropped_frames: self.dropped_frames,
            error: self.error.clone(),
        }
    }

    fn card(&self, node: NodeId) -> RecordingCard {
        RecordingCard {
            node,
            phase: self.phase,
            file_name: self.file_name.clone(),
        }
    }
}

#[derive(Default)]
pub struct RecordingService {
    entries: BTreeMap<NodeId, RecordingEntry>,
    state_epoch: u64,
}

impl RecordingService {
    /// Content token for the snapshot publisher; advanced by every mutation
    /// path below, including the 1 Hz duration tick while recording.
    pub fn state_epoch(&self) -> u64 {
        self.state_epoch
    }

    fn bump(&mut self) {
        self.state_epoch = self.state_epoch.wrapping_add(1);
    }

    pub fn phase(&self, node: NodeId) -> Option<RecordingPhase> {
        self.entries.get(&node).map(|entry| entry.phase)
    }

    /// Replace any previous entry with a fresh recording.
    pub fn begin(&mut self, node: NodeId, file_name: String, path: PathBuf, sample_rate: u32) {
        self.entries.insert(
            node,
            RecordingEntry {
                phase: RecordingPhase::Recording,
                file_name,
                path,
                sample_rate,
                recorded_secs: 0,
                dropped_frames: 0,
                error: None,
            },
        );
        self.bump();
    }

    /// A start that never opened a file; the message is a raw diagnostic.
    pub fn fail_start(&mut self, node: NodeId, error: String) {
        self.entries.insert(node, RecordingEntry::failed(error));
        self.bump();
    }

    pub fn set_paused(&mut self, node: NodeId, paused: bool) {
        self.update(node, |entry| {
            if entry.phase == RecordingPhase::Recording && paused
                || entry.phase == RecordingPhase::Paused && !paused
            {
                entry.phase = if paused {
                    RecordingPhase::Paused
                } else {
                    RecordingPhase::Recording
                };
                return true;
            }
            false
        });
    }

    pub fn set_stopping(&mut self, node: NodeId) {
        self.update(node, |entry| {
            if !entry.phase.is_terminal() {
                entry.phase = RecordingPhase::Stopping;
                return true;
            }
            false
        });
    }

    /// Fold one worker status refresh into the entry. Only quantized or
    /// discrete values are stored, so unchanged seconds do not republish.
    pub fn observe(&mut self, node: NodeId, status: &RecordingStatus) {
        self.update(node, |entry| {
            let phase = match status.state {
                RecordingState::Recording => {
                    if status.paused {
                        RecordingPhase::Paused
                    } else {
                        RecordingPhase::Recording
                    }
                }
                RecordingState::Stopping => RecordingPhase::Stopping,
                RecordingState::Finished => RecordingPhase::Finished,
                RecordingState::Failed => RecordingPhase::Failed,
            };
            let recorded_secs = if entry.sample_rate > 0 {
                status.written_frames / u64::from(entry.sample_rate)
            } else {
                0
            };
            let error = status.error.as_ref().map(|error| error.message.clone());
            let changed = entry.phase != phase
                || entry.recorded_secs != recorded_secs
                || entry.dropped_frames != status.stats.dropped_frames
                || entry.error != error;
            entry.phase = phase;
            entry.recorded_secs = recorded_secs;
            entry.dropped_frames = status.stats.dropped_frames;
            entry.error = error;
            changed
        });
    }

    /// Engine ownership went away (stop/retire): the workers finalize on
    /// their own, so every live entry settles without further observations.
    pub fn settle_all(&mut self) {
        self.update_all(|entry| {
            if entry.phase.is_terminal() {
                return false;
            }
            entry.phase = RecordingPhase::Finished;
            true
        });
    }

    pub fn snapshots(&self) -> Vec<RecordingSnapshot> {
        self.entries
            .iter()
            .map(|(node, entry)| entry.snapshot(*node))
            .collect()
    }

    pub fn cards(&self) -> Vec<RecordingCard> {
        self.entries
            .iter()
            .map(|(node, entry)| entry.card(*node))
            .collect()
    }

    fn update(&mut self, node: NodeId, change: impl FnOnce(&mut RecordingEntry) -> bool) {
        if let Some(entry) = self.entries.get_mut(&node)
            && change(entry)
        {
            self.bump();
        }
    }

    fn update_all(&mut self, change: impl Fn(&mut RecordingEntry) -> bool) {
        let mut changed = false;
        for entry in self.entries.values_mut() {
            changed |= change(entry);
        }
        if changed {
            self.bump();
        }
    }
}

/// Wall-clock date and time for the file-name stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeParts {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// The Win32 clock call is unsafe by nature; it is isolated in this module
/// behind the same `allow` pattern as `crate::console`.
#[cfg(windows)]
#[allow(unsafe_code)]
mod clock {
    use super::DateTimeParts;
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::SystemInformation::GetLocalTime;

    pub fn local_date_time() -> DateTimeParts {
        // SAFETY: `GetLocalTime` takes no handles and cannot fail.
        let now: SYSTEMTIME = unsafe { GetLocalTime() };
        DateTimeParts {
            year: u32::from(now.wYear),
            month: u32::from(now.wMonth),
            day: u32::from(now.wDay),
            hour: u32::from(now.wHour),
            minute: u32::from(now.wMinute),
            second: u32::from(now.wSecond),
        }
    }
}

#[cfg(windows)]
pub use clock::local_date_time;

#[cfg(not(windows))]
pub fn local_date_time() -> DateTimeParts {
    // UTC fallback for non-Windows builds: the stamp is a file-name label,
    // not a timezone claim.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    DateTimeParts {
        year: year.max(0) as u32,
        month,
        day,
        hour: (tod / 3_600) as u32,
        minute: (tod % 3_600 / 60) as u32,
        second: (tod % 60) as u32,
    }
}

#[cfg(not(windows))]
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// `<directory>/moiren-rec-<YYYYMMDD>-<HHMMSS>.wav`, suffixed `-2`, `-3`, …
/// when the second already produced a file. No spaces by construction, and
/// custom names are not supported yet.
pub fn recording_file_path(
    directory: &std::path::Path,
    parts: DateTimeParts,
    format: RecordingFormat,
    exists: impl Fn(&std::path::Path) -> bool,
) -> PathBuf {
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second
    );
    let base = directory.join(format!("moiren-rec-{stamp}.{}", format.extension()));
    if !exists(&base) {
        return base;
    }
    for suffix in 2..=99_u32 {
        let candidate = directory.join(format!(
            "moiren-rec-{stamp}-{suffix}.{}",
            format.extension()
        ));
        if !exists(&candidate) {
            return candidate;
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use std::path::Path;

    /// `NodeId` has no public constructor; tests mint ids through a real
    /// graph the same way production does.
    fn node_ids(count: usize) -> Vec<NodeId> {
        let mut graph = LogicalGraph::default();
        (0..count)
            .map(|_| {
                graph
                    .create_node(NodeKind::Recorder, 2)
                    .expect("fresh graph accepts recorder nodes")
            })
            .collect()
    }

    fn parts() -> DateTimeParts {
        DateTimeParts {
            year: 2026,
            month: 11,
            day: 11,
            hour: 15,
            minute: 4,
            second: 5,
        }
    }

    #[test]
    fn file_name_has_no_spaces_and_stamps_start_time() {
        let path = recording_file_path(
            PathBuf::from("D:/My Recordings").as_path(),
            parts(),
            RecordingFormat::Wav,
            |_| false,
        );
        assert_eq!(
            path,
            PathBuf::from("D:/My Recordings/moiren-rec-20261111-150405.wav")
        );
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(!name.contains(' '), "file names must not contain spaces");
    }

    #[test]
    fn same_second_collisions_take_numeric_suffixes() {
        let first = recording_file_path(Path::new("out"), parts(), RecordingFormat::Wav, |_| false);
        let taken = [first];
        let second = recording_file_path(
            Path::new("out"),
            parts(),
            RecordingFormat::Wav,
            |candidate| taken.iter().any(|p| p.as_path() == candidate),
        );
        assert_eq!(
            second,
            PathBuf::from("out/moiren-rec-20261111-150405-2.wav")
        );
        let taken = [taken[0].clone(), second];
        let third = recording_file_path(
            Path::new("out"),
            parts(),
            RecordingFormat::Wav,
            |candidate| taken.iter().any(|p| p.as_path() == candidate),
        );
        assert_eq!(third, PathBuf::from("out/moiren-rec-20261111-150405-3.wav"));
    }

    #[test]
    fn zero_padded_stamp_keeps_fixed_width_fields() {
        let narrow = DateTimeParts {
            year: 2026,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        };
        let path = recording_file_path(Path::new("out"), narrow, RecordingFormat::Wav, |_| false);
        assert_eq!(path, PathBuf::from("out/moiren-rec-20260102-030405.wav"));
    }

    #[test]
    fn observe_quantizes_seconds_and_folds_pause_state() {
        let ids = node_ids(2);
        let mut service = RecordingService::default();
        service.begin(
            ids[0],
            "moiren-rec-1.wav".into(),
            PathBuf::from("out/moiren-rec-1.wav"),
            48_000,
        );
        let epoch = service.state_epoch();
        service.observe(
            ids[0],
            &RecordingStatus {
                state: RecordingState::Recording,
                paused: false,
                written_frames: 48_000 * 90 + 47_999,
                stats: moiren_engine::recorder::RecorderStats {
                    accepted_frames: 0,
                    read_frames: 0,
                    dropped_frames: 3,
                    state: moiren_engine::recorder::RecorderState::Recording,
                },
                error: None,
            },
        );
        let snapshot = &service.snapshots()[0];
        assert_eq!(
            snapshot.recorded_secs, 90,
            "sub-second precision is dropped"
        );
        assert_eq!(snapshot.dropped_frames, 3);
        assert_eq!(snapshot.phase, RecordingPhase::Recording);
        assert_ne!(service.state_epoch(), epoch, "a real change republishes");

        service.observe(
            ids[0],
            &RecordingStatus {
                state: RecordingState::Recording,
                paused: true,
                written_frames: 48_000 * 90 + 47_999,
                stats: moiren_engine::recorder::RecorderStats {
                    accepted_frames: 0,
                    read_frames: 0,
                    dropped_frames: 3,
                    state: moiren_engine::recorder::RecorderState::Recording,
                },
                error: None,
            },
        );
        assert_eq!(service.snapshots()[0].phase, RecordingPhase::Paused);

        let epoch_before = service.state_epoch();
        service.observe(
            ids[0],
            &RecordingStatus {
                state: RecordingState::Recording,
                paused: true,
                written_frames: 48_000 * 90,
                stats: moiren_engine::recorder::RecorderStats {
                    accepted_frames: 0,
                    read_frames: 0,
                    dropped_frames: 3,
                    state: moiren_engine::recorder::RecorderState::Recording,
                },
                error: None,
            },
        );
        assert_eq!(
            service.state_epoch(),
            epoch_before,
            "an unchanged quantized status does not republish"
        );
    }

    #[test]
    fn cards_carry_discrete_state_only_and_failure_keeps_the_entry() {
        let ids = node_ids(2);
        let mut service = RecordingService::default();
        service.fail_start(ids[0], "no engine".into());
        assert_eq!(service.cards().len(), 1);
        assert_eq!(service.snapshots()[0].phase, RecordingPhase::Failed);
        assert_eq!(service.snapshots()[0].error.as_deref(), Some("no engine"));
        service.settle_all();
        assert_eq!(service.cards()[0].phase, RecordingPhase::Failed);
    }

    #[test]
    fn settle_all_finalizes_live_entries_only() {
        let ids = node_ids(2);
        let mut service = RecordingService::default();
        service.begin(ids[0], "a.wav".into(), PathBuf::from("out/a.wav"), 48_000);
        service.begin(ids[1], "b.wav".into(), PathBuf::from("out/b.wav"), 48_000);
        service.set_stopping(ids[1]);
        service.settle_all();
        let phases: Vec<_> = service
            .snapshots()
            .iter()
            .map(|snapshot| (snapshot.node, snapshot.phase))
            .collect();
        assert_eq!(
            phases,
            vec![
                (ids[0], RecordingPhase::Finished),
                (ids[1], RecordingPhase::Finished),
            ]
        );
    }
}
