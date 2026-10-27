//! Service-owned recording intent and immutable file-worker observations.
//! Destinations are session-local, not project-file or UI-owned resources.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::recorder::{RecorderStats, WavFormat};

use super::GraphSnapshot;

mod discard;
mod disk;
mod preflight;
mod runtime;
pub(crate) use runtime::Runtime;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecordingFormat {
    #[default]
    WavFloat32,
    WavPcm24,
    WavPcm16,
}

impl RecordingFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::WavFloat32 => "WAV (RIFF, float32)",
            Self::WavPcm24 => "WAV (RIFF, PCM24)",
            Self::WavPcm16 => "WAV (RIFF, PCM16)",
        }
    }

    pub const fn bytes_per_sample(self) -> u16 {
        self.wav_format().bytes_per_sample()
    }

    pub const fn header_bytes(self) -> u32 {
        self.wav_format().header_bytes()
    }

    pub(crate) const fn wav_format(self) -> WavFormat {
        match self {
            Self::WavFloat32 => WavFormat::Float32,
            Self::WavPcm24 => WavFormat::Pcm24,
            Self::WavPcm16 => WavFormat::Pcm16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingPhase {
    Idle,
    Starting,
    Recording,
    Stopping,
    /// Canceled before this attempt created a destination file; nothing was saved.
    Canceled,
    /// This attempt successfully finalized a WAV, which may contain zero frames.
    Finished,
    Failed,
}

impl RecordingPhase {
    pub fn is_busy(self) -> bool {
        matches!(self, Self::Starting | Self::Recording | Self::Stopping)
    }
}

const MIN_DISK_RESERVE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecordingEstimateBasis {
    #[default]
    Unknown,
    RiffOnly,
    DiskAndRiff,
}

impl RecordingEstimateBasis {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Capacity unknown",
            Self::RiffOnly => "RIFF only (disk space unknown)",
            Self::DiskAndRiff => "Disk reserve and RIFF limits",
        }
    }
}

/// Advisory limits, not promised recording time. Disk space is sampled on the
/// read-only capacity worker and may change externally. None means unknown, not zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordingEstimate {
    pub basis: RecordingEstimateBasis,
    /// Some only for a pre-recording estimate from candidate settings. These
    /// never populate RecordingSnapshot's confirmed-only format observations.
    pub proposed_sample_rate: Option<u32>,
    pub proposed_channels: Option<usize>,
    /// Disk probe is queued/running; the RIFF-only estimate is already usable.
    pub preflight_pending: bool,
    pub bytes_per_second: Option<u64>,
    pub free_disk_bytes: Option<u64>,
    /// Stable volume identity when available; unknown volumes are conservatively
    /// included in every active recording's aggregate rate.
    pub disk_volume: Option<String>,
    pub aggregate_bytes_per_second: Option<u64>,
    /// 64 MiB plus two seconds of aggregate writes for buffering/query lag.
    pub disk_reserve_bytes: u64,
    pub riff_remaining_frames: Option<u64>,
    pub riff_remaining_seconds: Option<f64>,
    pub disk_remaining_seconds: Option<f64>,
    /// The minimum known bound; with unknown disk space this is RIFF-only.
    pub remaining_seconds: Option<f64>,
}

impl RecordingEstimate {
    pub fn float_wav(
        sample_rate: u32,
        channels: usize,
        written_frames: u64,
        free_disk_bytes: Option<u64>,
    ) -> Self {
        Self::wav(
            RecordingFormat::WavFloat32,
            sample_rate,
            channels,
            written_frames,
            free_disk_bytes,
        )
    }

    pub fn wav(
        format: RecordingFormat,
        sample_rate: u32,
        channels: usize,
        written_frames: u64,
        free_disk_bytes: Option<u64>,
    ) -> Self {
        let mut estimate = Self {
            free_disk_bytes,
            ..Self::default()
        };
        // Recorder uses the engine builtin width budget (currently 256).
        if sample_rate == 0 || !(1..=256).contains(&channels) {
            return estimate;
        }
        let frame_bytes = channels as u64 * u64::from(format.bytes_per_sample());
        let byte_rate = u64::from(sample_rate) * frame_bytes;
        if byte_rate > u64::from(u32::MAX) {
            return estimate;
        }
        // Reuse the writer's header/padding bound, including mono PCM24's odd
        // data sizes. Estimates and file admission must agree at the RIFF edge.
        let max_frames = format
            .wav_format()
            .max_frames(channels)
            .expect("validated channels");
        let frames = max_frames.saturating_sub(written_frames);
        let riff_seconds = frames as f64 / f64::from(sample_rate);
        estimate.bytes_per_second = Some(byte_rate);
        estimate.riff_remaining_frames = Some(frames);
        estimate.riff_remaining_seconds = Some(riff_seconds);
        estimate.apply_disk_budget(byte_rate);
        estimate
    }

    fn apply_disk_budget(&mut self, aggregate_bytes_per_second: u64) {
        let Some(rate) = self.bytes_per_second else {
            return;
        };
        let aggregate = aggregate_bytes_per_second.max(rate);
        self.aggregate_bytes_per_second = Some(aggregate);
        self.disk_reserve_bytes =
            MIN_DISK_RESERVE_BYTES.saturating_add(aggregate.saturating_mul(2));
        self.disk_remaining_seconds = self
            .free_disk_bytes
            .map(|bytes| bytes.saturating_sub(self.disk_reserve_bytes) as f64 / aggregate as f64);
        self.basis = if self.free_disk_bytes.is_some() {
            RecordingEstimateBasis::DiskAndRiff
        } else {
            RecordingEstimateBasis::RiffOnly
        };
        self.remaining_seconds = self.riff_remaining_seconds.map(|riff| {
            self.disk_remaining_seconds
                .map_or(riff, |disk| riff.min(disk))
        });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordingSnapshot {
    pub node: NodeId,
    pub path: Option<PathBuf>,
    /// Selected intent while Idle; fixed for the admitted attempt thereafter.
    /// Changing it after a terminal state creates fresh Idle intent, not a
    /// relabeling of the previous file or its statistics. PCM has no dither.
    pub format: RecordingFormat,
    pub phase: RecordingPhase,
    /// The confirmed graph processing format used to open this file, never
    /// the device mix rate or an unapplied settings draft. Unknown until start.
    pub sample_rate: Option<u32>,
    pub channels: Option<usize>,
    /// Frames accepted by the WAV writer after the fresh capture boundary.
    /// Pre-start queue contents are discarded. Not an fsync guarantee.
    pub written_frames: u64,
    /// Engine queue counters are cumulative for this endpoint, not per file.
    pub stats: Option<RecorderStats>,
    pub estimate: RecordingEstimate,
    pub error: Option<String>,
}

impl RecordingSnapshot {
    fn idle(node: NodeId) -> Self {
        Self {
            node,
            path: None,
            format: RecordingFormat::WavFloat32,
            phase: RecordingPhase::Idle,
            sample_rate: None,
            channels: None,
            written_frames: 0,
            stats: None,
            estimate: RecordingEstimate::default(),
            error: None,
        }
    }
}

pub(crate) fn validate_destination(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path.file_name().is_none()
        || !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
    {
        return Err("Choose a .wav destination for RIFF float32, PCM24 or PCM16".into());
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct RecordingService {
    entries: BTreeMap<NodeId, RecordingSnapshot>,
}

impl RecordingService {
    pub(crate) fn snapshots(&self, graph: &GraphSnapshot) -> Vec<RecordingSnapshot> {
        graph
            .graph
            .nodes()
            .iter()
            .filter(|node| node.kind() == NodeKind::Recorder)
            .map(|node| {
                self.entries
                    .get(&node.id())
                    .cloned()
                    .unwrap_or_else(|| RecordingSnapshot::idle(node.id()))
            })
            .collect()
    }

    pub(crate) fn configure(
        &mut self,
        graph: &GraphSnapshot,
        node: NodeId,
        path: PathBuf,
    ) -> Result<(), String> {
        if graph.kind_of(node) != Some(NodeKind::Recorder) {
            return Err("Recording destination requires a Recorder node".into());
        }
        validate_destination(&path)?;
        if self
            .entries
            .get(&node)
            .is_some_and(|entry| entry.phase.is_busy())
        {
            return Err("Stop and finalize this recording before changing its destination".into());
        }
        // Resolve relative intent now; the worker must not reinterpret it after
        // the process's current directory has changed. This does not open files.
        let path = std::path::absolute(path).map_err(|error| error.to_string())?;
        let mut entry = RecordingSnapshot::idle(node);
        entry.format = self
            .entries
            .get(&node)
            .map_or(RecordingFormat::default(), |entry| entry.format);
        entry.path = Some(path);
        self.entries.insert(node, entry);
        Ok(())
    }

    pub(crate) fn set_format(
        &mut self,
        graph: &GraphSnapshot,
        node: NodeId,
        format: RecordingFormat,
    ) -> Result<(), String> {
        if graph.kind_of(node) != Some(NodeKind::Recorder) {
            return Err("Recording format requires a Recorder node".into());
        }
        let entry = self
            .entries
            .entry(node)
            .or_insert_with(|| RecordingSnapshot::idle(node));
        if entry.phase.is_busy() {
            return Err("Stop and finalize this recording before changing its format".into());
        }
        if entry.format != format {
            let path = entry.path.clone();
            *entry = RecordingSnapshot::idle(node);
            entry.path = path;
            entry.format = format;
        }
        Ok(())
    }

    pub(crate) fn fail(&mut self, node: NodeId, error: impl ToString) {
        let entry = self
            .entries
            .entry(node)
            .or_insert_with(|| RecordingSnapshot::idle(node));
        entry.phase = RecordingPhase::Failed;
        entry.error = Some(error.to_string());
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}
