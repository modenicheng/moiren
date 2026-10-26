use anyhow::{Context, Result, ensure};
use moiren_windows_audio::latency::{
    MarkerAnalysisConfig, MarkerCorrelation, MarkerGenerator, analyze_marker,
    qualification::{MarkerDistribution, marker_distribution},
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PcmSchedule {
    pub schema_version: u32,
    pub sample_rate: u32,
    pub reference_channel: usize,
    pub processed_channel: usize,
    pub marker_frames: usize,
    pub marker_amplitude: f32,
    pub reference_search_frames: usize,
    pub max_delay_frames: usize,
    pub max_marker_frames: usize,
    pub max_window_frames: usize,
    pub max_markers: usize,
    pub max_pcm_frames: u64,
    pub max_correlation_samples_per_marker: usize,
    pub max_total_correlation_samples: usize,
    pub max_total_window_frames: usize,
    pub min_correlation: f64,
    pub min_peak_margin: f64,
    pub min_rms: f64,
    pub windows: Vec<MarkerWindow>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerWindow {
    pub start_frame: u64,
}

#[derive(Debug, Serialize)]
pub struct PcmAnalysis {
    pub schema_version: u32,
    pub mode: &'static str,
    pub qualification: &'static str,
    pub gaps: [&'static str; 3],
    pub schedule: PcmSchedule,
    pub capture_frames: u64,
    pub failed_markers: usize,
    pub results: Vec<WindowResult>,
    pub distribution: Option<MarkerDistribution>,
}

#[derive(Debug, Serialize)]
pub struct Alignment {
    pub absolute_frame: u64,
    pub confidence: f64,
    pub peak_margin: f64,
    pub inverted: bool,
}

impl Alignment {
    fn new(origin: u64, detection: MarkerCorrelation) -> Self {
        Self {
            absolute_frame: origin + detection.delay_frames as u64,
            confidence: detection.confidence,
            peak_margin: detection.peak_margin,
            inverted: detection.inverted,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WindowResult {
    pub index: usize,
    pub start_frame: u64,
    pub reference: Option<Alignment>,
    pub processed: Option<Alignment>,
    pub delay_frames: Option<usize>,
    pub error: Option<String>,
}

impl PcmSchedule {
    fn validate(&self, bytes: u64) -> Result<usize> {
        ensure!(self.schema_version == 1, "unsupported schedule schema");
        ensure!(
            self.sample_rate > 0,
            "sample rate must be explicit and nonzero"
        );
        ensure!(
            self.reference_channel < 2
                && self.processed_channel < 2
                && self.reference_channel != self.processed_channel,
            "select distinct stereo channels 0 and 1"
        );
        ensure!(
            bytes.is_multiple_of(8),
            "PCM must contain complete little-endian interleaved stereo f32 frames"
        );
        ensure!(
            self.max_pcm_frames > 0 && bytes / 8 <= self.max_pcm_frames,
            "PCM frame limit exceeded"
        );
        ensure!(
            !self.windows.is_empty() && self.windows.len() <= self.max_markers,
            "marker count limit or empty schedule"
        );
        ensure!(
            self.marker_frames <= self.max_marker_frames,
            "marker frame limit exceeded"
        );
        MarkerGenerator::new(self.marker_frames, self.marker_amplitude)?;
        ensure!(
            [self.min_correlation, self.min_peak_margin, self.min_rms]
                .iter()
                .all(|value| value.is_finite() && *value > 0.0 && *value <= 1.0),
            "invalid analysis thresholds"
        );
        let window = self
            .marker_frames
            .checked_add(self.reference_search_frames)
            .and_then(|value| value.checked_add(self.max_delay_frames))
            .context("window overflow")?;
        ensure!(
            window <= self.max_window_frames,
            "window frame limit exceeded"
        );
        window.checked_mul(8).context("window byte overflow")?;
        let pairs = self
            .reference_search_frames
            .checked_add(1)
            .and_then(|value| value.checked_add(self.max_delay_frames))
            .and_then(|value| value.checked_add(1))
            .and_then(|value| value.checked_mul(self.marker_frames))
            .context("correlation work overflow")?;
        ensure!(
            pairs <= self.max_correlation_samples_per_marker,
            "per-marker work limit exceeded"
        );
        ensure!(
            pairs
                .checked_mul(self.windows.len())
                .is_some_and(|value| value <= self.max_total_correlation_samples),
            "total correlation work limit exceeded"
        );
        ensure!(
            window
                .checked_mul(self.windows.len())
                .is_some_and(|value| value <= self.max_total_window_frames),
            "total read limit exceeded"
        );
        let mut previous_end = 0;
        for entry in &self.windows {
            ensure!(
                entry.start_frame >= previous_end,
                "marker windows must be ordered and nonoverlapping"
            );
            previous_end = entry
                .start_frame
                .checked_add(window as u64)
                .context("window offset overflow")?;
            previous_end
                .checked_mul(8)
                .context("PCM seek offset overflow")?;
        }
        Ok(window)
    }

    fn config(&self, delay: usize) -> MarkerAnalysisConfig {
        MarkerAnalysisConfig {
            max_marker_frames: self.max_marker_frames,
            max_recording_frames: self.max_window_frames,
            max_delay_frames: delay,
            max_correlation_samples: self.max_correlation_samples_per_marker,
            min_correlation: self.min_correlation,
            min_peak_margin: self.min_peak_margin,
            min_rms: self.min_rms,
        }
    }
}

pub fn analyze(
    reader: &mut (impl Read + Seek),
    bytes: u64,
    schedule: PcmSchedule,
) -> Result<PcmAnalysis> {
    let window_frames = schedule.validate(bytes)?;
    let mut marker = vec![0.0; schedule.marker_frames];
    MarkerGenerator::new(schedule.marker_frames, schedule.marker_amplitude)?.fill(&mut marker);
    let mut raw = vec![0; window_frames * 8];
    let mut reference = vec![0.0; window_frames];
    let mut processed = vec![0.0; window_frames];
    let mut results = Vec::with_capacity(schedule.windows.len());
    let mut delays = Vec::with_capacity(schedule.windows.len());
    for (index, entry) in schedule.windows.iter().enumerate() {
        let mut result = WindowResult {
            index,
            start_frame: entry.start_frame,
            reference: None,
            processed: None,
            delay_frames: None,
            error: None,
        };
        let detected = (|| -> Result<MarkerCorrelation> {
            ensure!(
                entry.start_frame + window_frames as u64 <= bytes / 8,
                "insufficient observation window"
            );
            reader
                .seek(SeekFrom::Start(entry.start_frame * 8))
                .context("seek marker window")?;
            reader.read_exact(&mut raw).context("read marker window")?;
            for (index, frame) in raw.as_chunks::<8>().0.iter().enumerate() {
                let channel = |channel: usize| {
                    f32::from_le_bytes(
                        frame[channel * 4..channel * 4 + 4]
                            .try_into()
                            .expect("stereo frame"),
                    )
                };
                reference[index] = channel(schedule.reference_channel);
                processed[index] = channel(schedule.processed_channel);
            }
            let reference_length = schedule.marker_frames + schedule.reference_search_frames;
            let reference_detection = analyze_marker(
                &marker,
                &reference[..reference_length],
                schedule.config(schedule.reference_search_frames),
            )
            .context("reference channel")?;
            result.reference = Some(Alignment::new(entry.start_frame, reference_detection));
            let origin = reference_detection.delay_frames;
            let end = origin + schedule.marker_frames + schedule.max_delay_frames;
            // Search from the observed reference origin so submit/schedule time
            // cannot substitute for source sample generation on the common clock.
            let processed_detection = analyze_marker(
                &marker,
                &processed[origin..end],
                schedule.config(schedule.max_delay_frames),
            )
            .context("processed channel")?;
            result.processed = Some(Alignment::new(
                entry.start_frame + origin as u64,
                processed_detection,
            ));
            result.delay_frames = Some(processed_detection.delay_frames);
            Ok(MarkerCorrelation {
                confidence: reference_detection
                    .confidence
                    .min(processed_detection.confidence),
                peak_margin: reference_detection
                    .peak_margin
                    .min(processed_detection.peak_margin),
                ..processed_detection
            })
        })();
        match detected {
            Ok(delay) => delays.push(delay),
            Err(error) => result.error = Some(format!("{error:#}")),
        }
        results.push(result);
    }
    let distribution = if delays.is_empty() {
        None
    } else {
        Some(marker_distribution(
            &delays,
            schedule.sample_rate,
            schedule.max_markers,
        )?)
    };
    Ok(PcmAnalysis {
        schema_version: 1,
        mode: "offline_stereo_f32le_marker_windows",
        qualification: "not_evaluated",
        gaps: [
            "Marker windows do not verify full-waveform continuity or runtime quality counters.",
            "Calibration, physical measurement boundary and artifact provenance require independent evidence.",
            "No live recording/source mode, controlled-load soak or complete Stage 7 harness is implemented.",
        ],
        capture_frames: bytes / 8,
        failed_markers: results.len() - delays.len(),
        schedule,
        results,
        distribution,
    })
}
