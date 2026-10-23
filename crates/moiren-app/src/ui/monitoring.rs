//! Visible, session-scoped source/output Peak and RMS observations.
//!
//! These are the engine's two observation points. They must not be presented
//! as independent measurements of intermediate Gain, Pan, or other nodes.

use std::time::{Duration, Instant};

use moiren_engine::meter::signal::{AudioWindow, ChannelLevel};
use moiren_signal::{PublisherStatus, Topic, TopicLifecycle};

use crate::service::{
    AppHandle, AppSnapshot, InputSelection, LevelTopic, SessionGeneration, SessionPhase,
};

use super::signals::{PresentationDemand, VisibleSignal};

/// A complete window must keep arriving; a retained value is not proof that
/// the audio thread or publisher is still advancing.
const STALE_AFTER: Duration = Duration::from_millis(500);

// Display ballistics. Raw windows arrive at the analysis cadence (~100 Hz
// with the default 480-frame window); these time constants interpolate the
// readings onto the display frame rate so the meter motion is continuous
// between publications instead of stepping at the window rate.
/// RMS rise time constant toward louder readings.
const RMS_ATTACK: Duration = Duration::from_millis(15);
/// RMS fall time constant toward quieter readings.
const RMS_RELEASE: Duration = Duration::from_millis(220);
/// How long the peak marker holds its maximum before starting to fall.
const PEAK_HOLD: Duration = Duration::from_millis(900);
/// Peak marker fall speed once the hold expires, in dB per second.
const PEAK_FALL_DB_PER_S: f32 = 30.;
/// Display floor for the falling peak marker (−120 dBFS ≈ digital zero).
const DISPLAY_FLOOR_DB: f32 = -120.;

#[derive(Debug, Clone)]
struct ChannelBallistics {
    rms: f32,
    peak: f32,
    hold_until: Option<Instant>,
    ticked_at: Option<Instant>,
}

impl ChannelBallistics {
    fn new() -> Self {
        Self {
            rms: 0.,
            peak: 0.,
            hold_until: None,
            ticked_at: None,
        }
    }

    /// Advance toward `raw`. The first observation snaps to it; afterwards
    /// RMS follows asymmetric attack/release time constants while the peak
    /// rises instantly, holds, then falls at a fixed dB rate.
    fn observe(&mut self, raw: ChannelLevel, now: Instant) {
        let Some(ticked_at) = self.ticked_at else {
            self.rms = raw.rms;
            self.peak = raw.peak;
            self.hold_until = Some(now + PEAK_HOLD);
            self.ticked_at = Some(now);
            return;
        };
        let dt = now.saturating_duration_since(ticked_at);
        let tau = if raw.rms >= self.rms {
            RMS_ATTACK
        } else {
            RMS_RELEASE
        };
        let alpha = 1. - (-dt.as_secs_f32() / tau.as_secs_f32()).exp();
        self.rms += (raw.rms - self.rms) * alpha;
        match self.hold_until {
            None => {
                self.peak = raw.peak;
                self.hold_until = Some(now + PEAK_HOLD);
            }
            Some(_) if raw.peak >= self.peak => {
                self.peak = raw.peak;
                self.hold_until = Some(now + PEAK_HOLD);
            }
            // The anchor is the hold deadline until it expires, then the
            // instant the fall began, so the fall rate is continuous across
            // ticks and starts exactly when the hold ends.
            Some(anchor) if now < anchor => {}
            Some(anchor) => {
                self.peak = fall_db(self.peak, now.saturating_duration_since(anchor));
                self.hold_until = Some(now);
            }
        }
        self.ticked_at = Some(now);
    }

    fn level(&self) -> ChannelLevel {
        ChannelLevel {
            peak: self.peak,
            rms: self.rms,
        }
    }
}

fn fall_db(value: f32, dt: Duration) -> f32 {
    let db = if value <= 0. {
        DISPLAY_FLOOR_DB
    } else {
        (20. * value.log10()).max(DISPLAY_FLOOR_DB)
    };
    10f32.powf((db - PEAK_FALL_DB_PER_S * dt.as_secs_f32()).max(DISPLAY_FLOOR_DB) / 20.)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeterStatus {
    Hidden,
    Stopped,
    NotAvailable,
    Waiting,
    Live,
    Stale,
    NoPublisher,
    Error(String),
}

impl MeterStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Hidden => "Hidden",
            Self::Stopped => "Stopped",
            Self::NotAvailable => "Unavailable",
            Self::Waiting => "Waiting for a complete window",
            Self::Live => "Live",
            Self::Stale => "No recent audio window",
            Self::NoPublisher => "No publisher",
            Self::Error(error) => error,
        }
    }
}

/// UI-owned subscriptions follow the actual visible monitoring demand. None
/// means unknown or unavailable; a real silent window remains Some with zeroes.
pub struct MeterSubscriptions {
    pub input: Option<[ChannelLevel; 2]>,
    pub output: Option<[ChannelLevel; 2]>,
    pub input_status: MeterStatus,
    pub output_status: MeterStatus,
    handle: AppHandle,
    session: Option<(SessionGeneration, InputSelection)>,
    input_subscription: Option<MeterSubscription>,
    output_subscription: Option<MeterSubscription>,
}

impl MeterSubscriptions {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            input: None,
            output: None,
            input_status: MeterStatus::Stopped,
            output_status: MeterStatus::Stopped,
            handle,
            session: None,
            input_subscription: None,
            output_subscription: None,
        }
    }

    /// Poll off the audio thread. The caller combines active page, window,
    /// minimization, and component visibility into `visible`.
    pub fn update(&mut self, snapshot: &AppSnapshot, visible: bool) -> bool {
        self.update_points(snapshot, visible, visible)
    }

    /// Source and output demand are independent: scrolling one observation
    /// point completely out of view releases only its own subscription.
    pub fn update_points(
        &mut self,
        snapshot: &AppSnapshot,
        input_visible: bool,
        output_visible: bool,
    ) -> bool {
        self.update_points_at(snapshot, input_visible, output_visible, Instant::now())
    }

    /// Ballistics-smoothed source reading for presentation; `None` exactly
    /// when the raw observation ([`Self::input`]) is unavailable.
    pub fn display_input(&self) -> Option<[ChannelLevel; 2]> {
        self.input_subscription
            .as_ref()
            .and_then(|subscription| stereo_levels(subscription.display.as_deref()))
    }

    /// Ballistics-smoothed output reading for presentation; `None` exactly
    /// when the raw observation ([`Self::output`]) is unavailable.
    pub fn display_output(&self) -> Option<[ChannelLevel; 2]> {
        self.output_subscription
            .as_ref()
            .and_then(|subscription| stereo_levels(subscription.display.as_deref()))
    }

    #[cfg(test)]
    fn update_at(&mut self, snapshot: &AppSnapshot, visible: bool, now: Instant) -> bool {
        self.update_points_at(snapshot, visible, visible, now)
    }

    fn update_points_at(
        &mut self,
        snapshot: &AppSnapshot,
        input_visible: bool,
        output_visible: bool,
        now: Instant,
    ) -> bool {
        let before = (
            self.input,
            self.output,
            self.input_status.clone(),
            self.output_status.clone(),
        );
        if (!input_visible && !output_visible) || snapshot.phase != SessionPhase::Running {
            self.clear();
            self.input_status = if input_visible {
                MeterStatus::Stopped
            } else {
                MeterStatus::Hidden
            };
            self.output_status = if output_visible {
                MeterStatus::Stopped
            } else {
                MeterStatus::Hidden
            };
        } else if let Some(spec) = snapshot.desired.as_ref() {
            if self.session.as_ref().is_none_or(|(generation, source)| {
                *generation != snapshot.generation || *source != spec.source
            }) {
                // Retained analysis demand can preserve a cached Latest block
                // across UI hide/show or session changes. A new subscription
                // therefore also fences every publication already attempted.
                self.clear();
                self.session = Some((snapshot.generation, spec.source.clone()));
            }
            let (input_topic, output_topic) = match spec.source {
                InputSelection::Tone { .. } => (None, &snapshot.signal_topics.tone_output),
                _ => (
                    Some(&snapshot.signal_topics.monitor_input),
                    &snapshot.signal_topics.monitor_output,
                ),
            };
            if !input_visible {
                self.input_subscription = None;
                self.input = None;
                self.input_status = MeterStatus::Hidden;
            } else if let Some(topic) = input_topic {
                ensure_subscription(&mut self.input_subscription, &self.handle, topic);
            } else {
                self.input_subscription = None;
                self.input = None;
                self.input_status = MeterStatus::NotAvailable;
            }
            if output_visible {
                ensure_subscription(&mut self.output_subscription, &self.handle, output_topic);
            } else {
                self.output_subscription = None;
                self.output = None;
                self.output_status = MeterStatus::Hidden;
            }
            let mut publication = false;
            if let Some(subscription) = self.input_subscription.as_mut() {
                publication |= subscription.poll(now);
                self.input = stereo_levels(subscription.levels.as_deref());
                self.input_status = subscription.status.clone();
            }
            if let Some(subscription) = self.output_subscription.as_mut() {
                publication |= subscription.poll(now);
                self.output = stereo_levels(subscription.levels.as_deref());
                self.output_status = subscription.status.clone();
            }
            if publication {
                return true;
            }
        } else {
            self.clear();
            self.input_status = if input_visible {
                MeterStatus::NotAvailable
            } else {
                MeterStatus::Hidden
            };
            self.output_status = if output_visible {
                MeterStatus::NotAvailable
            } else {
                MeterStatus::Hidden
            };
        }
        before
            != (
                self.input,
                self.output,
                self.input_status.clone(),
                self.output_status.clone(),
            )
    }

    fn clear(&mut self) {
        self.input_subscription = None;
        self.output_subscription = None;
        self.session = None;
        self.input = None;
        self.output = None;
    }
}

fn ensure_subscription(
    subscription: &mut Option<MeterSubscription>,
    handle: &AppHandle,
    topic: &Topic<LevelTopic>,
) {
    if subscription
        .as_ref()
        .is_none_or(|current| current.topic != *topic)
    {
        *subscription = Some(MeterSubscription::new(handle.clone(), topic.clone(), 2));
    }
}

fn stereo_levels(levels: Option<&[ChannelLevel]>) -> Option<[ChannelLevel; 2]> {
    let levels = levels?;
    let [left, right] = levels else {
        return None;
    };
    Some([*left, *right])
}

struct MeterSubscription {
    topic: Topic<LevelTopic>,
    visible: VisibleSignal<LevelTopic>,
    sequence_fence: u64,
    last_delivery: Option<Instant>,
    last_window: Option<(u64, u64)>,
    levels: Option<Box<[ChannelLevel]>>,
    channels: usize,
    status: MeterStatus,
    ballistics: Vec<ChannelBallistics>,
    display: Option<Box<[ChannelLevel]>>,
}

pub struct NodeMeters {
    handle: AppHandle,
    generation: Option<SessionGeneration>,
    subscriptions: std::collections::BTreeMap<moiren_core::graph::NodeId, MeterSubscription>,
    clips: std::collections::BTreeMap<moiren_core::graph::NodeId, ClipHold>,
}

#[derive(Default)]
struct ClipHold([bool; 2]);

impl ClipHold {
    fn observe(&mut self, levels: &[ChannelLevel]) {
        for (clip, level) in self.0.iter_mut().zip(levels) {
            *clip |= level.peak >= 1.;
        }
    }
}

impl NodeMeters {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            generation: None,
            subscriptions: Default::default(),
            clips: Default::default(),
        }
    }

    pub fn update(
        &mut self,
        snapshot: &AppSnapshot,
        visible: &std::collections::BTreeSet<moiren_core::graph::NodeId>,
    ) -> bool {
        if self.generation != Some(snapshot.generation) {
            self.subscriptions.clear();
            self.clips.clear();
            self.generation = Some(snapshot.generation);
        }
        let active = snapshot.phase == SessionPhase::Running
            && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision);
        let before = self.subscriptions.len();
        self.subscriptions.retain(|id, _| {
            active && visible.contains(id) && snapshot.signal_topics.node_levels.contains_key(id)
        });
        self.clips
            .retain(|id, _| snapshot.graph.node(*id).is_some());
        let mut changed = before != self.subscriptions.len();
        if !active {
            return changed;
        }
        let now = Instant::now();
        for id in visible {
            let Some(topic) = snapshot.signal_topics.node_levels.get(id) else {
                continue;
            };
            let Some(channels) = snapshot
                .graph
                .graph
                .get_node(*id)
                .ok()
                .map(|node| node.channels())
            else {
                continue;
            };
            let subscription = self.subscriptions.entry(*id).or_insert_with(|| {
                MeterSubscription::new(self.handle.clone(), topic.clone(), channels)
            });
            if subscription.topic != *topic || subscription.channels != channels {
                *subscription =
                    MeterSubscription::new(self.handle.clone(), topic.clone(), channels);
                self.clips.remove(id);
            }
            let status = subscription.status.clone();
            let publication = subscription.poll(now);
            changed |= publication || subscription.status != status;
            if publication && let Some(levels) = subscription.levels.as_deref() {
                self.clips.entry(*id).or_default().observe(levels);
            }
        }
        changed
    }

    /// Ballistics-smoothed stereo projection for presentation.
    pub fn levels(&self, id: moiren_core::graph::NodeId) -> Option<[ChannelLevel; 2]> {
        let levels: &[ChannelLevel; 2] = self.channel_levels(id)?.try_into().ok()?;
        Some(*levels)
    }

    /// Ballistics-smoothed per-channel readings for presentation.
    pub fn channel_levels(&self, id: moiren_core::graph::NodeId) -> Option<&[ChannelLevel]> {
        self.subscriptions
            .get(&id)
            .and_then(|subscription| subscription.display.as_deref())
    }

    pub fn status(&self, id: moiren_core::graph::NodeId) -> MeterStatus {
        self.subscriptions
            .get(&id)
            .map(|subscription| subscription.status.clone())
            .unwrap_or(MeterStatus::Waiting)
    }

    pub fn clips(&self, id: moiren_core::graph::NodeId) -> [bool; 2] {
        self.clips.get(&id).map(|hold| hold.0).unwrap_or_default()
    }

    pub fn reset_clip(&mut self, id: moiren_core::graph::NodeId) {
        self.clips.remove(&id);
    }
}

impl MeterSubscription {
    fn new(handle: AppHandle, topic: Topic<LevelTopic>, channels: usize) -> Self {
        Self {
            sequence_fence: topic.stats().attempts,
            visible: VisibleSignal::new(handle, topic.clone(), Default::default()),
            topic,
            last_delivery: None,
            last_window: None,
            levels: None,
            channels,
            status: MeterStatus::Waiting,
            ballistics: Vec::new(),
            display: None,
        }
    }

    fn poll(&mut self, now: Instant) -> bool {
        let published = self.poll_windows(now);
        self.advance_display(now);
        published
    }

    /// Reads the latest published window; returns whether a new publication
    /// was accepted.
    fn poll_windows(&mut self, now: Instant) -> bool {
        let demand = PresentationDemand {
            window_visible: true,
            minimized: false,
            page_active: true,
            component_visible: true,
            intersects_viewport: true,
        };
        if let Err(error) = self.visible.update_demand(demand) {
            self.levels = None;
            self.status = MeterStatus::Error(format!("Meter subscription: {error:?}"));
            return false;
        }
        let status = self.topic.status();
        if status.lifecycle == TopicLifecycle::Closed
            || status.publisher == PublisherStatus::NoPublisher
        {
            self.levels = None;
            self.last_delivery = None;
            self.sequence_fence = self.topic.stats().attempts;
            self.status = MeterStatus::NoPublisher;
            return false;
        }
        let subscriber = match self.visible.subscriber() {
            Ok(Some(subscriber)) => subscriber,
            Ok(None) => {
                self.status = MeterStatus::Waiting;
                return false;
            }
            Err(error) => {
                self.levels = None;
                self.status = MeterStatus::Error(format!("Meter subscription: {error}"));
                return false;
            }
        };
        if let Some(window) = subscriber.read_latest() {
            let header = window.header();
            let meta = window.meta();
            let valid = valid_level_window(
                self.channels,
                meta.sequence,
                self.sequence_fence,
                *header,
                window.elements(),
                self.last_window,
            );
            if valid {
                self.levels = Some(window.elements().to_vec().into_boxed_slice());
                self.sequence_fence = meta.sequence;
                self.last_window = Some((header.timeline_epoch, header.frame_end));
                self.last_delivery = Some(now);
                self.status = MeterStatus::Live;
                return true;
            }
        }
        if self
            .last_delivery
            .is_some_and(|last| now.saturating_duration_since(last) >= STALE_AFTER)
        {
            self.levels = None;
            self.status = MeterStatus::Stale;
        } else if self.last_delivery.is_none() {
            self.status = MeterStatus::Waiting;
        }
        false
    }

    /// Step the display ballistics. Runs on every pump (display frame or
    /// control-side timer), so decay and peak release continue smoothly
    /// between publications; the display is `None` exactly when the raw
    /// observation is.
    fn advance_display(&mut self, now: Instant) {
        let Some(levels) = self.levels.as_deref() else {
            self.display = None;
            return;
        };
        if self.ballistics.len() != levels.len()
            || self
                .display
                .as_deref()
                .is_none_or(|display| display.len() != levels.len())
        {
            self.ballistics = levels.iter().map(|_| ChannelBallistics::new()).collect();
            self.display = Some(vec![ChannelLevel { peak: 0., rms: 0. }; levels.len()].into());
        }
        let display = self.display.as_deref_mut().expect("sized with ballistics");
        for ((channel, level), shown) in self
            .ballistics
            .iter_mut()
            .zip(levels)
            .zip(display.iter_mut())
        {
            channel.observe(*level, now);
            *shown = channel.level();
        }
    }
}

fn valid_level_window(
    channels: usize,
    sequence: u64,
    sequence_fence: u64,
    header: AudioWindow,
    levels: &[ChannelLevel],
    last_window: Option<(u64, u64)>,
) -> bool {
    sequence > sequence_fence
        && channels > 0
        && header.timeline_epoch != 0
        && header.frame_end > header.frame_start
        && header.sample_rate.is_finite()
        && header.sample_rate > 0.0
        && levels.len() == channels
        && levels.iter().all(|level| {
            level.peak.is_finite() && level.peak >= 0.0 && level.rms.is_finite() && level.rms >= 0.0
        })
        && last_window
            .is_none_or(|(epoch, end)| epoch != header.timeline_epoch || header.frame_start >= end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_hold_retains_each_channel_until_explicit_reset() {
        let mut hold = ClipHold::default();
        hold.observe(&[
            ChannelLevel { peak: 1., rms: 0.5 },
            ChannelLevel {
                peak: 0.9,
                rms: 0.5,
            },
        ]);
        hold.observe(&[
            ChannelLevel { peak: 0., rms: 0. },
            ChannelLevel { peak: 0., rms: 0. },
        ]);
        assert_eq!(hold.0, [true, false]);
        hold.observe(&[
            ChannelLevel { peak: 0., rms: 0. },
            ChannelLevel {
                peak: 1.1,
                rms: 0.8,
            },
        ]);
        assert_eq!(hold.0, [true, true]);
        hold = ClipHold::default();
        assert_eq!(hold.0, [false, false]);
    }

    #[test]
    fn node_level_windows_accept_the_graph_channel_width() {
        let header = AudioWindow {
            timeline_epoch: 7,
            frame_start: 128,
            frame_end: 256,
            sample_rate: 48_000.0,
        };
        let mono = [ChannelLevel {
            peak: 0.5,
            rms: 0.25,
        }];
        let surround = [ChannelLevel {
            peak: 0.5,
            rms: 0.25,
        }; 6];

        assert!(valid_level_window(1, 2, 1, header, &mono, None));
        assert!(valid_level_window(6, 2, 1, header, &surround, None));
        assert!(!valid_level_window(2, 2, 1, header, &mono, None));
        assert!(!valid_level_window(6, 2, 1, header, &surround[..2], None));
    }

    #[test]
    fn node_meter_stereo_projection_requires_exactly_two_channels() {
        let levels = [ChannelLevel {
            peak: 0.5,
            rms: 0.25,
        }; 2];
        let projected: &[ChannelLevel; 2] = levels.as_slice().try_into().unwrap();
        assert_eq!(*projected, levels);
        assert!(<[ChannelLevel; 2]>::try_from(&levels[..1]).is_err());
        assert!(<[ChannelLevel; 2]>::try_from(&levels[..]).is_ok());
    }

    #[test]
    fn ballistics_snap_to_the_first_window_then_smooth_rms_asymmetrically() {
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        channel.observe(
            ChannelLevel {
                peak: 0.5,
                rms: 0.3,
            },
            start,
        );
        assert_eq!(
            channel.level(),
            ChannelLevel {
                peak: 0.5,
                rms: 0.3
            },
            "the first observation snaps to the raw window"
        );

        // Rising RMS covers 1 - 1/e of the gap after one attack constant.
        channel.observe(
            ChannelLevel {
                peak: 0.5,
                rms: 0.4,
            },
            start + RMS_ATTACK,
        );
        let expected = 0.3 + (0.4 - 0.3) * (1. - std::f32::consts::E.recip());
        assert!((channel.level().rms - expected).abs() < 0.000001);

        // Falling RMS is far slower: after one release constant the reading
        // still holds 1/e of its previous value.
        let falling_from = channel.level().rms;
        channel.observe(
            ChannelLevel { peak: 0., rms: 0. },
            start + RMS_ATTACK + RMS_RELEASE,
        );
        let expected = falling_from * std::f32::consts::E.recip();
        assert!((channel.level().rms - expected).abs() < 0.000001);
    }

    #[test]
    fn ballistics_peak_holds_then_falls_at_a_fixed_db_rate() {
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        channel.observe(ChannelLevel { peak: 0.5, rms: 0. }, start);
        // Quieter windows never lower the marker while the hold lasts.
        channel.observe(
            ChannelLevel { peak: 0.1, rms: 0. },
            start + Duration::from_millis(100),
        );
        channel.observe(ChannelLevel { peak: 0.1, rms: 0. }, start + PEAK_HOLD);
        assert!((channel.level().peak - 0.5).abs() < 0.000001);

        // 100 ms into the fall: −6.02 dB fell by 3 dB (30 dB/s).
        channel.observe(
            ChannelLevel { peak: 0., rms: 0. },
            start + PEAK_HOLD + Duration::from_millis(100),
        );
        let db = 20. * channel.level().peak.max(f32::MIN_POSITIVE).log10();
        assert!((db - (-6.0206 - 3.)).abs() < 0.01);

        // A later, louder window raises the marker instantly and re-holds.
        channel.observe(
            ChannelLevel { peak: 0.8, rms: 0. },
            start + PEAK_HOLD + Duration::from_millis(200),
        );
        assert!((channel.level().peak - 0.8).abs() < 0.000001);
    }

    use crate::service::{RunLimit, ServiceContext, ServiceMailbox, SessionSpec};
    use crate::tone::{ToneConfig, prepare_tone_with_signals};
    use moiren_engine::{boundary::AudioReader, runtime::Engine};

    fn fixture(source: InputSelection) -> (ServiceContext, AppHandle, ServiceMailbox, AppSnapshot) {
        let mut context = ServiceContext::default();
        let generation = context
            .core
            .start(SessionSpec {
                source,
                output_endpoint_id: "test-output".into(),
                limit: RunLimit::UntilStopped,
                gain: 0.05,
                pan: 0.0,
                max_block_frames: 256,
            })
            .unwrap();
        assert!(context.core.activated(generation));
        assert!(context.core.owner_started(generation));
        let snapshot = crate::service::snapshot(&context);
        let (handle, mailbox) =
            ServiceMailbox::with_snapshot(std::thread::current(), snapshot.clone());
        (context, handle, mailbox, snapshot)
    }

    fn drain(mailbox: &ServiceMailbox, context: &mut ServiceContext) {
        while let Some(request) = mailbox.try_dispatch(&mut context.signals) {
            assert!(request.is_none());
        }
    }

    #[test]
    fn node_meter_demand_is_per_node_visible_and_confirmed_revision_scoped() {
        let (mut context, handle, mailbox, mut snapshot) =
            fixture(InputSelection::Tone { frequency_hz: 440. });
        let id = snapshot.graph.nodes[1].id;
        let other = snapshot.graph.nodes[2].id;
        snapshot
            .signal_topics
            .node_levels
            .insert(id, snapshot.signal_topics.tone_output.clone());
        snapshot
            .signal_topics
            .node_levels
            .insert(other, snapshot.signal_topics.monitor_output.clone());
        std::sync::Arc::make_mut(&mut snapshot.graph).active_revision =
            Some(snapshot.graph.candidate_revision);
        let mut meters = NodeMeters::new(handle);
        let visible = [id].into_iter().collect();
        meters.update(&snapshot, &visible);
        drain(&mailbox, &mut context);
        assert_eq!(
            snapshot.signal_topics.node_levels[&id].business_subscribers(),
            1
        );
        assert_eq!(
            snapshot.signal_topics.node_levels[&other].business_subscribers(),
            0
        );
        std::sync::Arc::make_mut(&mut snapshot.graph).candidate_revision += 1;
        meters.update(&snapshot, &visible);
        assert_eq!(
            snapshot.signal_topics.node_levels[&id].business_subscribers(),
            0
        );
        std::sync::Arc::make_mut(&mut snapshot.graph).active_revision =
            Some(snapshot.graph.candidate_revision);
        meters.update(&snapshot, &visible);
        drain(&mailbox, &mut context);
        assert_eq!(
            snapshot.signal_topics.node_levels[&id].business_subscribers(),
            1
        );
        meters.update(&snapshot, &Default::default());
        assert_eq!(
            snapshot.signal_topics.node_levels[&id].business_subscribers(),
            0
        );
        assert!(meters.levels(id).is_none());
    }

    fn render(engine: &mut Engine<f32>, output: &mut AudioReader<f32>, blocks: usize) {
        for _ in 0..blocks {
            engine.render(256).unwrap();
            assert_eq!(
                output
                    .read_interleaved(&mut [0.0; 512])
                    .unwrap()
                    .transferred_frames,
                256
            );
        }
    }

    #[test]
    fn non_running_and_hidden_observers_create_no_demand() {
        let (mut context, handle, mailbox, mut snapshot) = fixture(InputSelection::Tone {
            frequency_hz: 440.0,
        });
        let topic = snapshot.signal_topics.tone_output.clone();
        let mut meters = MeterSubscriptions::new(handle);
        for phase in [
            SessionPhase::Idle,
            SessionPhase::Starting,
            SessionPhase::Stopping,
            SessionPhase::Failed,
        ] {
            snapshot.phase = phase;
            meters.update(&snapshot, true);
            drain(&mailbox, &mut context);
            assert_eq!(topic.business_subscribers(), 0);
            assert_eq!(meters.output, None);
            assert_eq!(meters.output_status, MeterStatus::Stopped);
        }
        snapshot.phase = SessionPhase::Running;
        meters.update(&snapshot, false);
        drain(&mailbox, &mut context);
        assert_eq!(topic.business_subscribers(), 0);
        assert_eq!(meters.output_status, MeterStatus::Hidden);
        meters.update(&snapshot, true);
        drain(&mailbox, &mut context);
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(meters.output_status, MeterStatus::NoPublisher);
        assert_eq!(meters.input_status, MeterStatus::NotAvailable);
        meters.update(&snapshot, false);
        assert_eq!(topic.business_subscribers(), 0);
        assert_eq!(meters.output, None);
    }

    #[test]
    fn complete_real_windows_become_live_then_stale_and_resume_without_fake_zeroes() {
        let (mut context, handle, mailbox, snapshot) = fixture(InputSelection::Tone {
            frequency_hz: 440.0,
        });
        let mut preparation = context
            .signals
            .prepare_session(&snapshot.desired.as_ref().unwrap().source)
            .unwrap();
        let mut tone = prepare_tone_with_signals(
            ToneConfig {
                gain: 0.01,
                pan: 1.0,
                ..Default::default()
            },
            &mut preparation,
        )
        .unwrap();
        let mut meters = MeterSubscriptions::new(handle);
        let now = Instant::now();
        meters.update_at(&snapshot, true, now);
        drain(&mailbox, &mut context);
        render(&mut tone.compiled.engine, &mut tone.output, 1);
        context.signals.pump_if_due(now).unwrap();
        meters.update_at(&snapshot, true, now);
        assert_eq!(meters.output, None, "partial windows are unknown");
        assert_eq!(meters.output_status, MeterStatus::Waiting);
        render(&mut tone.compiled.engine, &mut tone.output, 1);
        let received = now + Duration::from_millis(5);
        context.signals.pump_if_due(received).unwrap();
        assert!(meters.update_at(&snapshot, true, received));
        let [left, right] = meters.output.unwrap();
        assert_eq!(
            left,
            ChannelLevel {
                peak: 0.0,
                rms: 0.0
            },
            "real silence remains a measured zero"
        );
        assert!((right.peak - 0.01).abs() < 0.00001);
        assert!((right.rms - 0.01 / 2.0_f32.sqrt()).abs() < 0.0001);
        assert_eq!(meters.output_status, MeterStatus::Live);
        assert_eq!(meters.input, None);
        assert_eq!(
            meters.display_output(),
            meters.output,
            "the first live window snaps the display to the raw reading"
        );
        assert!(!meters.update_at(&snapshot, true, received));
        assert!(meters.update_at(&snapshot, true, received + STALE_AFTER));
        assert_eq!(meters.output_status, MeterStatus::Stale);
        assert_eq!(meters.output, None);
        assert_eq!(meters.display_output(), None, "stale clears the display");
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        context.signals.pump_if_due(received + STALE_AFTER).unwrap();
        assert!(meters.update_at(&snapshot, true, received + STALE_AFTER));
        assert_eq!(meters.output_status, MeterStatus::Live);
        assert!(meters.output.is_some());
        drop(tone);
        drop(preparation);
        assert!(meters.update_at(&snapshot, true, received + STALE_AFTER));
        assert_eq!(meters.output, None);
        assert_eq!(meters.output_status, MeterStatus::NoPublisher);
    }

    #[test]
    fn retained_analysis_cache_is_fenced_after_hide_and_generation_changes() {
        let (mut context, handle, mailbox, mut snapshot) = fixture(InputSelection::Tone {
            frequency_hz: 440.0,
        });
        let topic = snapshot.signal_topics.tone_output.clone();
        let mut ticket = handle
            .subscribe_topic(topic.clone(), Default::default())
            .unwrap();
        drain(&mailbox, &mut context);
        let _analysis = ticket.try_take().unwrap().unwrap();
        let demand_generation = topic.demand_snapshot().generation;
        let mut preparation = context
            .signals
            .prepare_session(&snapshot.desired.as_ref().unwrap().source)
            .unwrap();
        let mut tone = prepare_tone_with_signals(ToneConfig::default(), &mut preparation).unwrap();
        let now = Instant::now();
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        context.signals.pump_if_due(now).unwrap();
        let mut meters = MeterSubscriptions::new(handle);
        meters.update_at(&snapshot, true, now);
        drain(&mailbox, &mut context);
        meters.update_at(&snapshot, true, now);
        assert_eq!(meters.output, None, "cache from before UI demand is fenced");
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        context
            .signals
            .pump_if_due(now + Duration::from_millis(5))
            .unwrap();
        meters.update_at(&snapshot, true, now + Duration::from_millis(5));
        assert!(meters.output.is_some());
        meters.update_at(&snapshot, false, now + Duration::from_millis(5));
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(topic.demand_snapshot().generation, demand_generation);
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        context
            .signals
            .pump_if_due(now + Duration::from_millis(10))
            .unwrap();
        meters.update_at(&snapshot, true, now + Duration::from_millis(10));
        drain(&mailbox, &mut context);
        meters.update_at(&snapshot, true, now + Duration::from_millis(10));
        assert_eq!(
            meters.output, None,
            "hidden-period cache is fenced even with persistent analysis demand"
        );
        drop(tone);
        drop(preparation);
        snapshot.generation = SessionGeneration(snapshot.generation.0 + 1);
        let mut preparation = context
            .signals
            .prepare_session(&snapshot.desired.as_ref().unwrap().source)
            .unwrap();
        let mut tone = prepare_tone_with_signals(
            ToneConfig {
                gain: 0.2,
                ..Default::default()
            },
            &mut preparation,
        )
        .unwrap();
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        context
            .signals
            .pump_if_due(now + Duration::from_millis(15))
            .unwrap();
        meters.update_at(&snapshot, true, now + Duration::from_millis(15));
        drain(&mailbox, &mut context);
        meters.update_at(&snapshot, true, now + Duration::from_millis(15));
        assert_eq!(
            meters.output, None,
            "new session begins unknown until a post-reset publication"
        );
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        context
            .signals
            .pump_if_due(now + Duration::from_millis(20))
            .unwrap();
        meters.update_at(&snapshot, true, now + Duration::from_millis(20));
        assert!((meters.output.unwrap()[0].peak - 0.2).abs() < 0.00001);
        assert_eq!(meters.output_status, MeterStatus::Live);
        assert_eq!(topic.demand_snapshot().generation, demand_generation);
        drop(meters);
        assert_eq!(topic.business_subscribers(), 1);
    }

    #[test]
    fn monitor_observations_are_independent_input_and_output_measurements() {
        use crate::monitor::{MonitorConfig, prepare_monitor_with_signals};
        use moiren_engine::boundary::ConstantSource;

        let (mut context, handle, mailbox, snapshot) = fixture(InputSelection::Physical {
            endpoint_id: "test-input".into(),
        });
        let mut preparation = context
            .signals
            .prepare_session(&snapshot.desired.as_ref().unwrap().source)
            .unwrap();
        let mut monitor = prepare_monitor_with_signals(
            ConstantSource {
                channels: 2,
                value: 0.4,
            },
            MonitorConfig {
                gain: 0.25,
                ..Default::default()
            },
            &mut preparation,
        )
        .unwrap();
        let mut meters = MeterSubscriptions::new(handle);
        let now = Instant::now();
        meters.update_at(&snapshot, true, now);
        drain(&mailbox, &mut context);
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        context.signals.pump_if_due(now).unwrap();
        meters.update_at(&snapshot, true, now);
        assert_eq!(meters.input_status, MeterStatus::Live);
        assert_eq!(meters.output_status, MeterStatus::Live);
        for level in meters.input.unwrap() {
            assert!((level.peak - 0.4).abs() < 0.00001);
            assert!((level.rms - 0.4).abs() < 0.00001);
        }
        for level in meters.output.unwrap() {
            assert!((level.peak - 0.1).abs() < 0.00001);
            assert!((level.rms - 0.1).abs() < 0.00001);
        }
        meters.update_at(&snapshot, false, now);
        assert_eq!(
            snapshot.signal_topics.monitor_input.business_subscribers(),
            0
        );
        assert_eq!(
            snapshot.signal_topics.monitor_output.business_subscribers(),
            0
        );
    }

    #[test]
    fn scrolling_one_monitor_point_out_of_view_preserves_the_other_live_subscription() {
        use crate::monitor::{MonitorConfig, prepare_monitor_with_signals};
        use moiren_engine::boundary::ConstantSource;

        let (mut context, handle, mailbox, snapshot) = fixture(InputSelection::Physical {
            endpoint_id: "test-input".into(),
        });
        let input_topic = snapshot.signal_topics.monitor_input.clone();
        let output_topic = snapshot.signal_topics.monitor_output.clone();
        let mut preparation = context
            .signals
            .prepare_session(&snapshot.desired.as_ref().unwrap().source)
            .unwrap();
        let mut monitor = prepare_monitor_with_signals(
            ConstantSource {
                channels: 2,
                value: 0.4,
            },
            MonitorConfig {
                gain: 0.25,
                ..Default::default()
            },
            &mut preparation,
        )
        .unwrap();
        let mut meters = MeterSubscriptions::new(handle);
        let now = Instant::now();
        meters.update_points_at(&snapshot, true, true, now);
        drain(&mailbox, &mut context);
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        context.signals.pump_if_due(now).unwrap();
        meters.update_points_at(&snapshot, true, true, now);
        assert_eq!(meters.input_status, MeterStatus::Live);
        assert_eq!(meters.output_status, MeterStatus::Live);
        let output_generation = output_topic.demand_snapshot().generation;
        let input_attempts = input_topic.stats().attempts;

        assert!(meters.update_points_at(&snapshot, false, true, now));
        assert_eq!(input_topic.business_subscribers(), 0);
        assert_eq!(meters.input, None);
        assert_eq!(meters.input_status, MeterStatus::Hidden);
        assert_eq!(output_topic.business_subscribers(), 1);
        assert_eq!(meters.output_status, MeterStatus::Live);
        assert!(meters.output.is_some());
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        context
            .signals
            .pump_if_due(now + Duration::from_millis(5))
            .unwrap();
        assert!(meters.update_points_at(&snapshot, false, true, now + Duration::from_millis(5)));
        assert_eq!(input_topic.stats().attempts, input_attempts);
        assert_eq!(output_topic.demand_snapshot().generation, output_generation);
        assert!((meters.output.unwrap()[0].peak - 0.1).abs() < 0.00001);

        meters.update_points_at(&snapshot, true, true, now + Duration::from_millis(5));
        drain(&mailbox, &mut context);
        meters.update_points_at(&snapshot, true, true, now + Duration::from_millis(5));
        assert_eq!(
            meters.input, None,
            "reappearing point waits for its own fresh window"
        );
        assert_eq!(output_topic.demand_snapshot().generation, output_generation);
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        context
            .signals
            .pump_if_due(now + Duration::from_millis(10))
            .unwrap();
        meters.update_points_at(&snapshot, true, true, now + Duration::from_millis(10));
        assert_eq!(meters.input_status, MeterStatus::Live);
        assert!((meters.input.unwrap()[0].peak - 0.4).abs() < 0.00001);
        let input_generation = input_topic.demand_snapshot().generation;

        assert!(meters.update_points_at(&snapshot, true, false, now + Duration::from_millis(10)));
        assert_eq!(output_topic.business_subscribers(), 0);
        assert_eq!(meters.output, None);
        assert_eq!(meters.output_status, MeterStatus::Hidden);
        assert_eq!(input_topic.business_subscribers(), 1);
        assert_eq!(meters.input_status, MeterStatus::Live);
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        context
            .signals
            .pump_if_due(now + Duration::from_millis(15))
            .unwrap();
        assert!(meters.update_points_at(&snapshot, true, false, now + Duration::from_millis(15)));
        assert_eq!(input_topic.demand_snapshot().generation, input_generation);
        assert_eq!(meters.input_status, MeterStatus::Live);
    }
}
