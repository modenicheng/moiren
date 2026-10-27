//! Validate engine publications, never generate replacement spectrum data.
use moiren_engine::spectrum::{SpectrumSettings, SpectrumWindow};

#[derive(Debug, serde::Serialize)]
pub(super) struct PeakEvidence {
    pub channel: usize,
    pub bin: usize,
    pub frequency_hz: f64,
    pub magnitude: f32,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct SpectrumEvidence {
    pub fft_size: usize,
    pub active_hop_frames: usize,
    pub channels: usize,
    pub sample_rate: f64,
    pub frame_start: u64,
    pub frame_end: u64,
    pub bin_hz: f64,
    pub tone_hz: f64,
    pub peaks: Vec<PeakEvidence>,
}

pub(super) fn verify_spectrum(
    window: SpectrumWindow,
    magnitudes: &[f32],
    active: SpectrumSettings,
    channels: usize,
    tone_hz: f64,
) -> anyhow::Result<SpectrumEvidence> {
    anyhow::ensure!(
        active.valid()
            && window.fft_size == active.fft_size
            && (1..=256).contains(&channels)
            && window.channels == channels
            && window.sample_rate.is_finite()
            && window.sample_rate > 0.
            && window.frame_end.checked_sub(window.frame_start) == Some(active.fft_size as u64)
            && magnitudes.len() == channels * active.bins_per_channel(),
        "spectrum header/payload disagrees with active binding: window={window:?}, active={active:?}, channels={channels}, samples={}",
        magnitudes.len()
    );
    anyhow::ensure!(
        tone_hz.is_finite() && tone_hz > 0. && tone_hz < window.sample_rate / 2.,
        "capture tone is outside the actual processing-rate domain"
    );
    anyhow::ensure!(
        magnitudes
            .iter()
            .all(|value| value.is_finite() && *value >= 0.),
        "spectrum contains nonfinite or negative magnitudes"
    );
    let bin_hz = window.sample_rate / active.fft_size as f64;
    let mut peaks = Vec::with_capacity(channels);
    for (channel, values) in magnitudes
        .chunks_exact(active.bins_per_channel())
        .enumerate()
    {
        let (bin, &magnitude) = values
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .expect("validated nonempty bins");
        let frequency_hz = bin as f64 * bin_hz;
        anyhow::ensure!(
            bin > 0 && magnitude > 1e-5 && (frequency_hz - tone_hz).abs() <= bin_hz,
            "channel {} peak is not the real source tone: bin={bin}, frequency={frequency_hz}, magnitude={magnitude}, tone={tone_hz}, bin_hz={bin_hz}",
            channel + 1
        );
        peaks.push(PeakEvidence {
            channel: channel + 1,
            bin,
            frequency_hz,
            magnitude,
        });
    }
    Ok(SpectrumEvidence {
        fft_size: active.fft_size,
        active_hop_frames: active.hop_frames,
        channels,
        sample_rate: window.sample_rate,
        frame_start: window.frame_start,
        frame_end: window.frame_end,
        bin_hz,
        tone_hz,
        peaks,
    })
}
