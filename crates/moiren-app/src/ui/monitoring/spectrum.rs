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

/// ISO 266 base-2 grid around the 1 kHz reference: a twelfth of an octave
/// per point (the resolution display analyzers conventionally smooth to),
/// spanning the nominal 20 Hz .. 20 kHz audio band.
const GRID_REFERENCE_HZ: f64 = 1000.;
const GRID_LOW_K: i32 = -68;
const GRID_HIGH_K: i32 = 52;
const GRID_STEP_OCTAVES: f64 = 1. / 12.;
/// Band edges sit a twenty-fourth of an octave either side of a center.
const GRID_HALF_BAND_OCTAVES: f64 = 1. / 24.;

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
}

impl OctaveGrid {
    pub fn twelfth(sample_rate: f64) -> Self {
        let nyquist = (sample_rate / 2.).max(GRID_REFERENCE_HZ);
        let centers = (GRID_LOW_K..=GRID_HIGH_K)
            .map(|k| GRID_REFERENCE_HZ * 2f64.powf(f64::from(k) * GRID_STEP_OCTAVES))
            .filter(|center| *center <= nyquist)
            .collect::<Vec<_>>();
        Self {
            centers: centers.into(),
        }
    }

    /// The point's band edges in Hz, a twenty-fourth of an octave around it.
    fn band(&self, index: usize) -> (f64, f64) {
        let factor = 2f64.powf(GRID_HALF_BAND_OCTAVES);
        (self.centers[index] / factor, self.centers[index] * factor)
    }

    pub fn lo_hz(&self) -> f64 {
        self.centers.first().copied().unwrap_or(GRID_REFERENCE_HZ)
    }

    pub fn hi_hz(&self) -> f64 {
        self.centers.last().copied().unwrap_or(GRID_REFERENCE_HZ)
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

/// Project one window's one-sided magnitudes onto the grid in dBFS. Each
/// band takes the maximum bin magnitude across channels — the engine's
/// magnitudes are Hann-coherent-gain normalized to sinusoid amplitudes, so
/// a tone reads its true amplitude at its peak bin and a band maximum
/// cannot miss it. A band narrower than a bin interpolates on the
/// dB-versus-log2-frequency line between the two bins bracketing its
/// center — never linearly in frequency or amplitude — and a band below
/// the first non-DC bin holds that bin's level.
pub fn project_spectrum(bins: &[f32], header: &SpectrumWindow, grid: &OctaveGrid, out: &mut [f32]) {
    let bins_per_channel = header.fft_size / 2 + 1;
    let bin_hz = header.sample_rate / header.fft_size as f64;
    // Per-bin maxima across channels, in dB; DC never enters a band.
    let mut bin_db = vec![REST_DB; bins_per_channel];
    for channel in 0..header.channels {
        let slice = &bins[channel * bins_per_channel..][..bins_per_channel];
        for (bin, amplitude) in bin_db.iter_mut().zip(slice).skip(1) {
            let level = 20. * f64::from(*amplitude).max(1e-9).log10();
            *bin = (*bin).max(level as f32);
        }
    }
    let last_bin = bins_per_channel - 1;
    let hz_of = |bin: usize| bin as f64 * bin_hz;
    for (index, (point, &center)) in out.iter_mut().zip(&grid.centers).enumerate() {
        let (low, high) = grid.band(index);
        let first = ((low / bin_hz).ceil() as usize).clamp(1, last_bin);
        let last = ((high / bin_hz).floor() as usize).clamp(1, last_bin);
        let level = if low <= hz_of(first) && hz_of(last) <= high && first <= last {
            bin_db[first..=last].iter().copied().fold(REST_DB, f32::max)
        } else {
            let left = ((center / bin_hz).floor() as usize).clamp(1, last_bin.max(2) - 1);
            let right = (left + 1).min(last_bin);
            let span = hz_of(right).log2() - hz_of(left).log2();
            let position = if span > 0. {
                ((center.log2() - hz_of(left).log2()) / span).clamp(0., 1.)
            } else {
                0.
            };
            bin_db[left] + (bin_db[right] - bin_db[left]) * position as f32
        };
        *point = level.clamp(REST_DB, 0.);
    }
}

pub struct NodeSpectra {
    handle: AppHandle,
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
            let subscription = self
                .subscriptions
                .entry(*id)
                .or_insert_with(|| SpectrumSubscription::new(self.handle.clone(), topic.clone()));
            if subscription.topic != *topic {
                *subscription = SpectrumSubscription::new(self.handle.clone(), topic.clone());
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
    fn new(handle: AppHandle, topic: Topic<SpectrumTopic>) -> Self {
        let mut this = Self::unsubscribed(topic);
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
                self.grid = Some((header.sample_rate, OctaveGrid::twelfth(header.sample_rate)));
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
            project_spectrum(bins, &header, grid, &mut projected);
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
        let one_k = 68;
        let mut out = vec![REST_DB; grid.centers.len()];
        // 0.5 amplitude at bin 21 (984.4 Hz) of the second channel only:
        // the 1 kHz band holds that bin and reads -6.02 dBFS.
        let mut bins = silence(1024, 2);
        bins[per_channel + 21] = 0.5;
        project_spectrum(&bins, &head, &grid, &mut out);
        assert!((out[one_k] - (-6.0206)).abs() < 0.01, "{}", out[one_k]);
        // The neighboring bands hold other, silent bins: floor.
        assert_eq!(out[one_k - 1], REST_DB);
        assert_eq!(out[one_k + 1], REST_DB);
        // The band takes the louder channel, never a sum.
        bins[21] = 0.25;
        project_spectrum(&bins, &head, &grid, &mut out);
        assert!((out[one_k] - (-6.0206)).abs() < 0.01, "{}", out[one_k]);
        // Full scale clamps at 0 dBFS; DC never reaches a band.
        bins[per_channel + 21] = 1.5;
        bins[0] = 1.;
        project_spectrum(&bins, &head, &grid, &mut out);
        assert_eq!(out[one_k], 0.);
        assert_eq!(out[0], REST_DB);
    }

    #[test]
    fn sparse_lows_interpolate_in_db_over_log_frequency() {
        // 16-point FFT at 48 kHz: bins sit 3 kHz apart, so the 1/12-octave
        // bands between them hold no bin and must walk the dB-vs-log2-f
        // line between their bracketing bins.
        let grid = OctaveGrid::twelfth(48_000.);
        let head = header(16, 48_000., 1);
        let mut bins = silence(16, 1);
        bins[1] = 1.; // 3000 Hz, 0 dBFS
        bins[2] = 0.1; // 6000 Hz, -20 dBFS
        let mut out = vec![REST_DB; grid.centers.len()];
        project_spectrum(&bins, &head, &grid, &mut out);
        let index = 68 + 21;
        let center = grid.centers[index];
        assert!((center - 3363.6).abs() < 0.1, "{center}");
        let expected =
            -20. * ((center.log2() - 3000f64.log2()) / (6000f64.log2() - 3000f64.log2()));
        assert!(
            (f64::from(out[index]) - expected).abs() < 0.05,
            "log-log interpolation: {} expected {expected}",
            out[index]
        );
        // A linear-in-frequency amplitude interpolation would read higher.
        let linear = 20. * (1. + (center - 3000.) / 3000. * (0.1 - 1.)).log10();
        assert!((f64::from(out[index]) - linear).abs() > 1., "{linear}");
        // Below the first bin the curve holds that bin's level.
        assert_eq!(out[0], 0.);
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
                    bins[21] = 0.5;
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
        // The display sits at the tone less the one decay step a poll takes
        // after pinning (0.6 dB over 20 ms).
        let shown = subscription.points.as_deref().unwrap()[68];
        assert!((-6.7..=-6.0).contains(&shown), "{shown}");
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
