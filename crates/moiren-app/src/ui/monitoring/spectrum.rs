//! Live spectrum observations behind the analyzer card's body: the engine's
//! FFT windows, relayed onto the application bus by the managed bridge, are
//! subscribed here, projected onto an ISO 266 fractional-octave grid in
//! dBFS, and decayed in the dB domain — so an idle card parks at the floor
//! exactly like the meters do, and silence stands the frame loop down.

use std::collections::BTreeMap;
use std::time::Instant;

use moiren_core::graph::NodeId;
use moiren_engine::spectrum::SpectrumWindow;
use moiren_signal::{PublisherStatus, Subscriber, SubscriptionOptions, Topic, TopicLifecycle};
use rust_i18n::t;

use super::{MeterStatus, REST_DB, STALE_AFTER, SUBSCRIPTION_GRACE};
use crate::service::{AppHandle, AppSnapshot, SessionGeneration, SessionPhase, SpectrumTopic};

/// The analyzer's projection knobs, gathered as the one seam future
/// parameter wiring plugs into at the code level (the UI does not expose
/// them yet): band resolution, grid anchor and extent, the window's
/// effective-noise bandwidth, and the display's dBFS bounds. Defaults are
/// the shipping analyzer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectrumParams {
    /// Band width in octaves — 1/12, the ISO 266 fractional-octave grid.
    pub band_octaves: f64,
    /// The base-2 grid's anchor and extent.
    pub reference_hz: f64,
    pub low_hz: f64,
    pub high_hz: f64,
    /// Hann effective-noise bandwidth in bins (N·Σw²/Σw² = 1.5).
    pub enbw_bins: f64,
    /// Display floor and ceiling in dBFS.
    pub floor_db: f32,
    pub ceiling_db: f32,
}

impl Default for SpectrumParams {
    fn default() -> Self {
        Self {
            band_octaves: 1. / 12.,
            reference_hz: 1000.,
            low_hz: 19.686_266_405,
            high_hz: 20_158.736_798,
            enbw_bins: 1.5,
            floor_db: super::REST_DB,
            ceiling_db: 0.,
        }
    }
}

/// The nominal axis span before any window lands: the default grid's own
/// bounds at a rate whose Nyquist clears 20 kHz (a test pins the two
/// together).
pub const SPECTRUM_NOMINAL_SPAN_HZ: (f64, f64) = (19.686_266_405, 20_158.736_798);

/// Band level release in dB per second; the display falls at this rate
/// between windows and parks at the floor, where frames stop.
const RELEASE_DB_PER_SEC: f32 = 30.;

/// Depth for spectrum subscriptions, matching the app topic registration.
const SPECTRUM_STREAM_DEPTH: usize = 4;

/// Motion under this many dB is sub-pixel on a 200 px body: not a change.
const CHANGED_EPSILON_DB: f32 = 0.05;

/// The analyzer's frequency axis: ISO 266 base-2 centers at 1/12-octave
/// spacing anchored on 1 kHz, kept only up to the running sample rate's
/// Nyquist. Points are equally spaced in log2 frequency — equally spaced on
/// the card's logarithmic axis, at whatever span survives the clamp.
#[derive(Debug, Clone, PartialEq)]
pub struct OctaveGrid {
    /// Center frequencies, ascending, 1 kHz-anchored.
    pub centers: Box<[f64]>,
    band_octaves: f64,
}

impl OctaveGrid {
    pub fn twelfth(sample_rate: f64) -> Self {
        Self::new(SpectrumParams::default(), sample_rate)
    }

    pub fn new(params: SpectrumParams, sample_rate: f64) -> Self {
        // Grid steps count fractional octaves from the reference: a point's
        // k is log2(f/reference)/band_octaves, so the extent's ks are that
        // of the bounds, clamped to whole steps inside them. The epsilon
        // keeps an exactly-on-a-point bound (the default grid's own edges)
        // from falling off to the neighboring step through float rounding.
        const K_EPSILON: f64 = 1e-9;
        let steps_of = |hz: f64| (hz / params.reference_hz).log2() / params.band_octaves;
        let low_k = (steps_of(params.low_hz) - K_EPSILON).ceil() as i32;
        let high_k = (steps_of(params.high_hz) + K_EPSILON).floor() as i32;
        let nyquist = (sample_rate / 2.).max(params.reference_hz);
        let centers = (low_k..=high_k)
            .map(|k| params.reference_hz * 2f64.powf(f64::from(k) * params.band_octaves))
            .filter(|center| *center <= nyquist)
            .collect::<Vec<_>>();
        Self {
            centers: centers.into(),
            band_octaves: params.band_octaves,
        }
    }

    /// The point's band edges in Hz, half a band either side of it.
    fn band(&self, index: usize) -> (f64, f64) {
        let factor = 2f64.powf(self.band_octaves / 2.);
        (self.centers[index] / factor, self.centers[index] * factor)
    }

    pub fn lo_hz(&self) -> f64 {
        self.centers.first().copied().unwrap_or(1000.)
    }

    pub fn hi_hz(&self) -> f64 {
        self.centers.last().copied().unwrap_or(1000.)
    }
}

/// One curve ready for presentation: the decayed dB points plus the
/// logarithmic axis span they are spread over.
#[derive(Debug, Clone, Copy)]
pub struct SpectrumCurve<'a> {
    pub points: &'a [f32],
    pub lo_hz: f64,
    pub hi_hz: f64,
}

/// Project one window's one-sided magnitudes onto the grid, as a
/// fractional-octave band-power spectrum with effective-noise-bandwidth
/// normalization — the standard FFT-analyzer band level (Heinzel et al.,
/// *Spectrum and spectral density estimation with the DFT*, ENBW; IEC
/// 61260 fractional-octave banding):
///
/// `L_band = 10·log10( (Σ_band wₖ·aₖ²) / ENBW )`
///
/// The engine's magnitudes `aₖ` are Hann coherent-gain normalized to
/// sinusoid amplitudes, and the Hann window spreads a bin-centered tone
/// over exactly ENBW (1.5 bins) of power, so a tone reads its true
/// amplitude at 0 dBFS while broadband power accumulates across every bin
/// the band covers — the band grows by 10·log10(bins) as bands widen with
/// frequency, instead of a single-bin maximum reading small at the top
/// end. The result is independent of the FFT size. Each bin joins the
/// bands it overlaps with its overlap fraction (a bin covers half a bin
/// width either side of its center). A band narrower than one bin is below
/// the transform's resolution and inherits the level of the bin nearest
/// its center — the standard resolution-limited display, with no invented
/// interpolation. DC never joins a band, and channels combine as the
/// maximum of their band powers.
pub fn project_spectrum(
    bins: &[f32],
    header: &SpectrumWindow,
    grid: &OctaveGrid,
    params: &SpectrumParams,
    out: &mut [f32],
) {
    let bins_per_channel = header.fft_size / 2 + 1;
    let bin_hz = header.sample_rate / header.fft_size as f64;
    let half_bin = bin_hz / 2.;
    let last_bin = bins_per_channel - 1;
    let channel = |index: usize| -> Box<[f32]> {
        bins[index * bins_per_channel..][..bins_per_channel]
            .iter()
            .copied()
            .collect()
    };
    let channels: Vec<Box<[f32]>> = (0..header.channels).map(channel).collect();
    // One band's power for one channel; bins are amplitudes squared, the
    // DC bin never joins.
    let band_power = |amplitudes: &[f32], low: f64, high: f64| -> f64 {
        let first = (((low + half_bin) / bin_hz).floor() as usize).max(1);
        let mut power = 0.;
        for (bin, amplitude) in amplitudes.iter().enumerate().skip(first) {
            let bin_lo = bin as f64 * bin_hz - half_bin;
            if bin_lo >= high {
                break;
            }
            let bin_hi = bin_lo + bin_hz;
            let overlap = (bin_hi.min(high) - bin_lo.max(low)) / bin_hz;
            if overlap > 0. {
                power += overlap * f64::from(*amplitude).powi(2);
            }
        }
        power
    };
    let level_of =
        |power: f64| -> f32 { (10. * (power.max(1e-18) / params.enbw_bins).log10()) as f32 };
    for (index, point) in out.iter_mut().enumerate() {
        let (low, high) = grid.band(index);
        let level = if high - low <= bin_hz {
            // Sub-resolution band: the nearest bin's own ENBW band level.
            let nearest = ((grid.centers[index] / bin_hz).round() as usize).clamp(1, last_bin);
            channels
                .iter()
                .map(|amplitudes| level_of(f64::from(amplitudes[nearest]).powi(2)))
                .fold(f32::MIN, f32::max)
        } else {
            channels
                .iter()
                .map(|amplitudes| level_of(band_power(amplitudes, low, high)))
                .fold(f32::MIN, f32::max)
        };
        *point = level.clamp(params.floor_db, params.ceiling_db);
    }
}

pub struct NodeSpectra {
    handle: AppHandle,
    /// The projection's knobs — today the shipping defaults, tomorrow the
    /// seam parameter wiring sets (the UI does not expose them yet).
    params: SpectrumParams,
    generation: Option<SessionGeneration>,
    subscriptions: BTreeMap<NodeId, SpectrumSubscription>,
    /// Nodes seen visible recently; subscriptions outlive the visible set by
    /// [`SUBSCRIPTION_GRACE`] instead of flapping with per-frame visibility.
    seen: BTreeMap<NodeId, Instant>,
    /// Curves placed without a session, for captures and tests.
    #[cfg(any(test, feature = "gpui-test"))]
    injected: BTreeMap<NodeId, Box<[f32]>>,
}

struct SpectrumSubscription {
    topic: Topic<SpectrumTopic>,
    params: SpectrumParams,
    ticket: Option<crate::service::SubscriptionTicket<SpectrumTopic>>,
    subscriber: Option<Subscriber<SpectrumTopic>>,
    sequence_fence: u64,
    last_delivery: Option<Instant>,
    last_window: Option<(u64, u64)>,
    /// The grid the projection sits on, rebuilt when the running sample
    /// rate changes (an FFT size change arrives as a new topic instead).
    grid: Option<(f64, OctaveGrid)>,
    /// Latest projection: the wake-bearing raw observation.
    targets: Option<Box<[f32]>>,
    /// A spare projection buffer so steady-state windows never allocate.
    scratch: Option<Box<[f32]>>,
    /// Decayed presentation reading, what the card paints.
    points: Option<Box<[f32]>>,
    decayed_at: Option<Instant>,
    status: MeterStatus,
}

impl NodeSpectra {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            params: SpectrumParams::default(),
            generation: None,
            subscriptions: Default::default(),
            seen: Default::default(),
            #[cfg(any(test, feature = "gpui-test"))]
            injected: Default::default(),
        }
    }

    pub fn update(
        &mut self,
        snapshot: &AppSnapshot,
        visible: impl IntoIterator<Item = NodeId>,
    ) -> bool {
        self.update_at(snapshot, visible, Instant::now())
    }

    pub fn update_at(
        &mut self,
        snapshot: &AppSnapshot,
        visible: impl IntoIterator<Item = NodeId>,
        now: Instant,
    ) -> bool {
        if self.generation != Some(snapshot.generation) {
            self.subscriptions.clear();
            self.seen.clear();
            self.generation = Some(snapshot.generation);
        }
        // Spectra belong to the running session: candidate edits and the
        // brief swap window of an automatic rebuild must never blank them.
        let active = snapshot.phase == SessionPhase::Running;
        for id in visible {
            self.seen.insert(id, now);
        }
        self.seen.retain(|id, seen| {
            snapshot.graph.node(*id).is_some()
                && now.saturating_duration_since(*seen) < SUBSCRIPTION_GRACE
        });
        let before = self.subscriptions.len();
        self.subscriptions.retain(|id, _| {
            active
                && self.seen.contains_key(id)
                && snapshot.signal_topics.node_spectra.contains_key(id)
        });
        let mut changed = before != self.subscriptions.len();
        if !active {
            return changed;
        }
        for id in self.seen.keys() {
            let Some(topic) = snapshot.signal_topics.node_spectra.get(id) else {
                continue;
            };
            let (handle, params) = (self.handle.clone(), self.params);
            let subscription = self.subscriptions.entry(*id).or_insert_with(|| {
                SpectrumSubscription::new(handle.clone(), topic.clone(), params)
            });
            if subscription.topic != *topic {
                *subscription = SpectrumSubscription::new(handle, topic.clone(), params);
            }
            let status = subscription.status.clone();
            let raw_changed = subscription.poll(now);
            // A publication alone must not wake the interface: only the
            // projected curve moving, or a status change, may — the rule
            // the level path applies.
            changed |= subscription.status != status || raw_changed;
        }
        changed
    }

    /// Whether every observed curve has decayed to the floor, so the
    /// animation loop can stand down until fresh data lifts a band.
    pub fn at_rest(&self) -> bool {
        self.subscriptions
            .values()
            .all(SpectrumSubscription::at_rest)
    }

    pub fn curve(&self, id: NodeId) -> Option<SpectrumCurve<'_>> {
        #[cfg(any(test, feature = "gpui-test"))]
        if let Some(points) = self.injected.get(&id) {
            let grid = OctaveGrid::twelfth(48_000.);
            return Some(SpectrumCurve {
                points,
                lo_hz: grid.lo_hz(),
                hi_hz: grid.hi_hz(),
            });
        }
        let subscription = self.subscriptions.get(&id)?;
        let points = subscription.points.as_deref()?;
        let (_, grid) = subscription.grid.as_ref()?;
        Some(SpectrumCurve {
            points,
            lo_hz: grid.lo_hz(),
            hi_hz: grid.hi_hz(),
        })
    }

    pub fn status(&self, id: NodeId) -> MeterStatus {
        self.subscriptions
            .get(&id)
            .map(|subscription| subscription.status.clone())
            .unwrap_or(MeterStatus::Waiting)
    }

    /// Place a curve without a running session, for captures and tests;
    /// it shadows whatever a subscription would paint.
    #[cfg(any(test, feature = "gpui-test"))]
    pub fn inject_curve(&mut self, id: NodeId, points: &[f32]) {
        self.injected.insert(id, points.to_vec().into_boxed_slice());
    }
}

impl SpectrumSubscription {
    fn new(handle: AppHandle, topic: Topic<SpectrumTopic>, params: SpectrumParams) -> Self {
        let mut this = Self::unsubscribed(topic);
        this.params = params;
        match handle.subscribe_topic(
            this.topic.clone(),
            SubscriptionOptions {
                stream_depth: Some(SPECTRUM_STREAM_DEPTH),
            },
        ) {
            Ok(ticket) => this.ticket = Some(ticket),
            Err(error) => {
                this.status = MeterStatus::Error(
                    t!(
                        "Spectrum subscription: %{error}",
                        error = format!("{error:?}")
                    )
                    .to_string(),
                );
            }
        }
        this
    }

    fn unsubscribed(topic: Topic<SpectrumTopic>) -> Self {
        Self {
            sequence_fence: topic.stats().attempts,
            params: SpectrumParams::default(),
            ticket: None,
            subscriber: None,
            topic,
            last_delivery: None,
            last_window: None,
            grid: None,
            targets: None,
            scratch: None,
            points: None,
            decayed_at: None,
            status: MeterStatus::Waiting,
        }
    }

    fn at_rest(&self) -> bool {
        self.points
            .as_deref()
            .is_none_or(|points| points.iter().all(|point| *point <= REST_DB))
    }

    /// Drain the queue and continue the fall; reports whether the projected
    /// curve moved (the decay alone never counts as a change).
    fn poll(&mut self, now: Instant) -> bool {
        let raw_changed = self.poll_windows(now);
        self.advance(now);
        raw_changed
    }

    fn poll_windows(&mut self, now: Instant) -> bool {
        let mut raw_changed = false;
        let status = self.topic.status();
        if status.lifecycle == TopicLifecycle::Closed
            || status.publisher == PublisherStatus::NoPublisher
        {
            raw_changed |= self.targets.is_some();
            self.targets = None;
            self.points = None;
            self.last_delivery = None;
            self.sequence_fence = self.topic.stats().attempts;
            self.status = MeterStatus::NoPublisher;
            return raw_changed;
        }
        if let Some(ticket) = self.ticket.as_mut() {
            match ticket.try_take() {
                Ok(Some(subscriber)) => {
                    self.ticket = None;
                    self.subscriber = Some(subscriber);
                }
                Ok(None) => {}
                Err(error) => {
                    self.ticket = None;
                    raw_changed |= self.targets.is_some();
                    self.targets = None;
                    self.points = None;
                    self.status = MeterStatus::Error(
                        t!("Spectrum subscription: %{error}", error = error.to_string())
                            .to_string(),
                    );
                    return raw_changed;
                }
            }
        }
        let Some(subscriber) = self.subscriber.as_mut() else {
            self.status = MeterStatus::Waiting;
            return raw_changed;
        };
        let mut published = false;
        while let Some(window) = subscriber.next() {
            let meta = window.meta();
            let header = *window.header();
            let bins = window.elements();
            if !valid_spectrum_window(
                meta.sequence,
                self.sequence_fence,
                header,
                bins,
                self.last_window,
            ) {
                continue;
            }
            self.sequence_fence = meta.sequence;
            self.last_window = Some((header.timeline_epoch, header.frame_end));
            self.last_delivery = Some(now);
            self.status = MeterStatus::Live;
            published = true;
            if self
                .grid
                .as_ref()
                .is_none_or(|(rate, _)| *rate != header.sample_rate)
            {
                self.grid = Some((
                    header.sample_rate,
                    OctaveGrid::new(self.params, header.sample_rate),
                ));
                self.targets = None;
                self.points = None;
            }
            let grid = &self.grid.as_ref().expect("grid built above").1;
            let len = grid.centers.len();
            let mut projected = self
                .scratch
                .take()
                .filter(|buffer| buffer.len() == len)
                .unwrap_or_else(|| vec![REST_DB; len].into_boxed_slice());
            project_spectrum(bins, &header, grid, &self.params, &mut projected);
            raw_changed |= self.targets.as_deref().is_none_or(|previous| {
                previous
                    .iter()
                    .zip(&projected)
                    .any(|(previous, target)| (previous - target).abs() > CHANGED_EPSILON_DB)
            });
            // Rise instantly to the new projection; the fall continues in
            // `advance`.
            let points = self
                .points
                .get_or_insert_with(|| vec![REST_DB; len].into_boxed_slice());
            for (point, target) in points.iter_mut().zip(&projected) {
                *point = (*point).max(*target);
            }
            self.scratch = self.targets.replace(projected);
        }
        if published {
            return raw_changed;
        }
        if self
            .last_delivery
            .is_some_and(|last| now.saturating_duration_since(last) >= STALE_AFTER)
        {
            raw_changed |= self.targets.is_some();
            self.targets = None;
            self.points = None;
            self.status = MeterStatus::Stale;
        } else if self.last_delivery.is_none() {
            self.status = MeterStatus::Waiting;
        }
        raw_changed
    }

    /// Continue the dB-domain fall. Runs on every pump; a poll that drained
    /// no window just keeps the curve falling toward the floor.
    fn advance(&mut self, now: Instant) {
        let Some(points) = self.points.as_deref_mut() else {
            self.decayed_at = None;
            return;
        };
        let elapsed = self
            .decayed_at
            .map_or(0., |previous| {
                now.saturating_duration_since(previous).as_secs_f32()
            })
            .min(0.25);
        self.decayed_at = Some(now);
        let release = RELEASE_DB_PER_SEC * elapsed;
        for point in points.iter_mut() {
            *point = (*point - release).max(REST_DB);
        }
    }
}

fn valid_spectrum_window(
    sequence: u64,
    sequence_fence: u64,
    header: SpectrumWindow,
    bins: &[f32],
    last_window: Option<(u64, u64)>,
) -> bool {
    sequence > sequence_fence
        && header.timeline_epoch != 0
        && header.frame_end > header.frame_start
        && header.sample_rate.is_finite()
        && header.sample_rate > 0.0
        && header.fft_size.is_power_of_two()
        && (16..=8192).contains(&header.fft_size)
        && header.channels > 0
        && bins.len() == header.channels * (header.fft_size / 2 + 1)
        && bins.iter().all(|bin| bin.is_finite() && *bin >= 0.0)
        && last_window
            .is_none_or(|(epoch, end)| epoch != header.timeline_epoch || header.frame_start >= end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_signal::{BlockInit, Bus, BusConfig, PumpBudget};
    use std::time::Duration;

    fn header(fft_size: usize, sample_rate: f64, channels: usize) -> SpectrumWindow {
        SpectrumWindow {
            timeline_epoch: 1,
            frame_start: 0,
            frame_end: fft_size as u64,
            sample_rate,
            fft_size,
            channels,
        }
    }

    /// Channel-major bins, all zero.
    fn silence(fft_size: usize, channels: usize) -> Vec<f32> {
        vec![0.; channels * (fft_size / 2 + 1)]
    }

    #[test]
    fn twelfth_octave_grid_is_iso_266_anchored_at_one_kilohertz() {
        let grid = OctaveGrid::twelfth(48_000.);
        assert_eq!(grid.centers.len(), 121);
        assert_eq!(grid.centers[68], 1000.);
        assert!(
            (grid.centers[69] / grid.centers[68] - 2f64.powf(1. / 12.)).abs() < 1e-12,
            "points sit a twelfth of an octave apart"
        );
        assert!((grid.lo_hz() - 19.686).abs() < 1e-3, "{}", grid.lo_hz());
        assert!((grid.hi_hz() - 20_158.7).abs() < 0.1, "{}", grid.hi_hz());
        assert!((grid.lo_hz() - SPECTRUM_NOMINAL_SPAN_HZ.0).abs() < 1e-6);
        assert!((grid.hi_hz() - SPECTRUM_NOMINAL_SPAN_HZ.1).abs() < 1e-3);
        // A slow rate clamps the top of the axis at its Nyquist only.
        let slow = OctaveGrid::twelfth(8_000.);
        assert!(slow.hi_hz() <= 4_000.);
        assert_eq!(slow.lo_hz(), grid.lo_hz());
        assert!(slow.centers.len() < grid.centers.len());
    }

    #[test]
    fn tone_reads_its_amplitude_at_its_band_across_channels() {
        let grid = OctaveGrid::twelfth(48_000.);
        let head = header(1024, 48_000., 2);
        let per_channel = 1024 / 2 + 1;
        // A band that resolves the tone: at ~4 kHz the 1/12-octave band is
        // ~4.9 bins wide, so the Hann skirt sits entirely inside it and
        // ENBW normalization reads the tone's own level — the standard's
        // tone calibration. (Near band edges a tone legitimately shares
        // its skirt with the neighbors; that under-read is inherent to
        // fractional-octave power displays.)
        let band = 92; // exactly 4 kHz on the 1 kHz-anchored grid
        assert_eq!(grid.centers[band], 4000.);
        let params = SpectrumParams::default();
        let mut out = vec![REST_DB; grid.centers.len()];
        // The amplitude spectrum a Hann window makes of a 0.5-amplitude
        // bin-centered tone, on the second channel only: 0.5 at the peak
        // bin, half that one bin either side.
        let mut bins = silence(1024, 2);
        bins[per_channel + 84] = 0.25;
        bins[per_channel + 85] = 0.5;
        bins[per_channel + 86] = 0.25;
        project_spectrum(&bins, &head, &grid, &params, &mut out);
        assert!((out[band] - (-6.0206)).abs() < 0.01, "{}", out[band]);
        // The neighboring bands' bins are silent: floor.
        assert_eq!(out[band - 1], REST_DB);
        assert_eq!(out[band + 1], REST_DB);
        // Channels combine as the maximum of band powers, never a sum.
        bins[84] = 0.25;
        bins[85] = 0.5;
        bins[86] = 0.25;
        project_spectrum(&bins, &head, &grid, &params, &mut out);
        assert!((out[band] - (-6.0206)).abs() < 0.01, "{}", out[band]);
        // Full scale clamps at 0 dBFS; DC never reaches a band.
        bins[per_channel + 85] = 1.5;
        bins[0] = 1.;
        project_spectrum(&bins, &head, &grid, &params, &mut out);
        assert_eq!(out[band], 0.);
        assert_eq!(out[0], REST_DB);
    }

    #[test]
    fn broadband_power_accumulates_across_each_band() {
        // The standard's point: broadband energy must sum over the bins a
        // band covers. A flat bin spectrum at amplitude a reads
        // 10·log10(K·a²/ENBW) for a band covering K bins, so the top of the
        // axis — where a 1/12-octave band spans dozens of bins — sits
        // 10·log10(K) above a sub-resolution band at the bottom, instead of
        // the single-bin maximum the first cut displayed.
        let grid = OctaveGrid::twelfth(48_000.);
        let head = header(1024, 48_000., 1);
        let params = SpectrumParams::default();
        let mut bins = silence(1024, 1);
        for bin in bins.iter_mut().skip(1) {
            *bin = 0.1;
        }
        let mut out = vec![REST_DB; grid.centers.len()];
        project_spectrum(&bins, &head, &grid, &params, &mut out);
        let bin_hz = 48_000. / 1024.;
        let (_low_lo, _low_hi) = grid.band(0);
        let (high_lo, high_hi) = grid.band(120);
        let expected_low = 10. * (0.1f64 * 0.1 / params.enbw_bins).log10();
        // Count the bins the top band covers with the same overlap weights
        // the projection uses.
        let half = bin_hz / 2.;
        let k_high: f64 = (1..1025)
            .map(|bin| {
                let bin_lo = bin as f64 * bin_hz - half;
                (((bin_lo + bin_hz).min(high_hi) - bin_lo.max(high_lo)) / bin_hz).max(0.)
            })
            .sum();
        let expected_high = 10. * (k_high * 0.1f64 * 0.1 / params.enbw_bins).log10();
        // The bottom band is narrower than a bin: it inherits it exactly.
        assert!((out[0] as f64 - expected_low).abs() < 0.01, "{}", out[0]);
        assert!(
            (out[120] as f64 - expected_high).abs() < 0.05,
            "{}",
            out[120]
        );
        assert!(
            (f64::from(out[120] - out[0]) - 10. * k_high.log10()).abs() < 0.06,
            "the top band accumulates 10·log10(K) = {} over the bottom: {}",
            10. * k_high.log10(),
            out[120] - out[0]
        );
        // The accumulation must be FFT-size independent: a 2048 window of
        // the same density reads within a fraction of a dB.
        let head = header(2048, 48_000., 1);
        let mut bins = silence(2048, 1);
        for bin in bins.iter_mut().skip(1) {
            *bin = (0.1 / 2f64.sqrt()) as f32;
        }
        let mut out2048 = vec![REST_DB; grid.centers.len()];
        project_spectrum(&bins, &head, &grid, &params, &mut out2048);
        assert!(
            (out2048[120] - out[120]).abs() < 0.3,
            "{} vs {}",
            out2048[120],
            out[120]
        );
    }

    #[test]
    fn sub_resolution_bands_inherit_their_nearest_bin() {
        // A 16-point FFT at 48 kHz puts bins 3 kHz apart: every 1/12-octave
        // band is narrower than one bin, so the standard display follows
        // the bins — each point reads the bin nearest its center, a
        // staircase the spline smooths, with no invented interpolation
        // between bins and no ramp at the axis edge.
        let grid = OctaveGrid::twelfth(48_000.);
        let head = header(16, 48_000., 1);
        let params = SpectrumParams::default();
        let mut bins = silence(16, 1);
        bins[1] = 1.; // 3000 Hz
        bins[2] = 0.1; // 6000 Hz
        let mut out = vec![REST_DB; grid.centers.len()];
        project_spectrum(&bins, &head, &grid, &params, &mut out);
        let bin_level = |amplitude: f32| 10. * (f64::from(amplitude).powi(2) / 1.5).log10();
        // Points nearest bin 1 (up to 4.5 kHz) read its own ENBW band
        // level, the axis edge included — the plateau real analyzers show
        // below their resolution.
        assert!((out[0] - bin_level(1.) as f32).abs() < 0.01, "{}", out[0]);
        assert_eq!(out[0], out[34], "20 Hz and 1 kHz both resolve to bin 1");
        assert_eq!(out[0], out[68]);
        // Points nearest bin 2 follow it instead.
        let near_6k = grid
            .centers
            .iter()
            .position(|center| (*center - 5_800.).abs() < 150.)
            .expect("a point near 6 kHz");
        assert!(
            (out[near_6k] - bin_level(0.1) as f32).abs() < 0.01,
            "{}",
            out[near_6k]
        );
        // The silent top of the spectrum sits at the floor.
        assert_eq!(out[120], REST_DB);
    }

    #[test]
    fn subscription_wakes_on_curve_changes_only_and_parks_at_the_floor() {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let topic = bus
            .register_stream_block(
                "test/spectrum",
                BlockInit {
                    header: header(1024, 48_000., 2),
                    element: 0.,
                    capacity: 2 * 513,
                },
                4,
            )
            .unwrap();
        let subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, mut host) = bus.create_host(1).unwrap();
        let key = control.new_binding().unwrap();
        let publisher = control.prepare(&topic, key).unwrap();
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        let operation = update.op_bindings(key).unwrap();
        drop(host.activate(update).unwrap());
        let mut subscription = SpectrumSubscription::unsubscribed(topic.clone());
        subscription.subscriber = Some(subscriber);

        let mut frame_end = 0;
        let mut publish = |bus: &mut Bus, tone: bool| {
            host.begin_block();
            {
                let mut scope = host.scope(&operation);
                let mut write = scope.try_write(&publisher.handle(), 2 * 513).unwrap();
                *write.header_mut() = SpectrumWindow {
                    timeline_epoch: 1,
                    frame_start: frame_end,
                    frame_end: frame_end + 512,
                    sample_rate: 48_000.,
                    fft_size: 1024,
                    channels: 2,
                };
                frame_end += 512;
                let bins = write.elements_mut();
                bins.fill(0.);
                if tone {
                    bins[20] = 0.25;
                    bins[21] = 0.5;
                    bins[22] = 0.25;
                }
                let _ = write.commit(2 * 513);
            }
            // Delivery is staged across pump passes: settle it.
            for _ in 0..16 {
                let report = bus.pump(PumpBudget {
                    topic_checks: 64,
                    deliveries: 16,
                    copy_bytes: 1 << 20,
                });
                if !report.has_pending {
                    break;
                }
            }
        };

        let mut now = Instant::now();
        // The first window lands: Waiting -> Live with a fresh curve wakes.
        publish(&mut bus, true);
        now += Duration::from_millis(20);
        assert!(subscription.poll(now), "the first curve wakes");
        assert_eq!(subscription.status, MeterStatus::Live);
        // Republishing the same tone must not: the curve already says this,
        // even while the decay keeps the pump busy between windows.
        for _ in 0..24 {
            publish(&mut bus, true);
            now += Duration::from_millis(20);
            assert!(!subscription.poll(now), "an unchanged curve must not wake");
        }
        assert!(!subscription.at_rest(), "a live tone is not at rest");
        // The display sits at the tone's band level — the skirt only
        // partly overlaps the 1 kHz band's edges here, so it reads just
        // under the tone's own level — less the one decay step a poll
        // takes after pinning (0.6 dB over 20 ms).
        let shown = subscription.points.as_deref().unwrap()[68];
        assert!((-9.2..=-8.2).contains(&shown), "{shown}");
        // Silence lifts the tone: that one change wakes, then the curve
        // falls at the release rate under a stream that keeps publishing
        // silence, and parks at the floor without waking again.
        publish(&mut bus, false);
        now += Duration::from_millis(20);
        assert!(subscription.poll(now), "the drop wakes once");
        let mut polls = 0;
        while !subscription.at_rest() {
            publish(&mut bus, false);
            now += Duration::from_millis(33);
            assert!(!subscription.poll(now), "the fall itself never wakes");
            polls += 1;
            assert!(polls < 400, "the curve must reach the floor");
        }
        // -6 dB to -96 dB at 30 dB/s is three seconds: ~91 polls of 33 ms.
        assert!((80..=100).contains(&polls), "{polls} polls to the floor");
    }
}
