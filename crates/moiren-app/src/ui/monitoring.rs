//! Visible, session-scoped source/output Peak and RMS observations.
//!
//! Legacy source/output topics and managed per-node topics share lifecycle
//! validation and display envelopes.

use std::time::{Duration, Instant};

use moiren_engine::meter::signal::{AudioWindow, ChannelLevel};
use moiren_signal::{PublisherStatus, Subscriber, SubscriptionOptions, Topic, TopicLifecycle};
use rust_i18n::t;

use crate::service::{
    AppHandle, AppSnapshot, InputSelection, LevelTopic, SessionGeneration, SessionPhase,
};

/// A complete window must keep arriving; a retained value is not proof that
/// the audio thread or publisher is still advancing.
const STALE_AFTER: Duration = Duration::from_millis(500);

/// Keep brief visibility changes from recreating subscriptions and fencing
/// fresh publications while the graph canvas is panned or zoomed.
const SUBSCRIPTION_GRACE: Duration = Duration::from_millis(750);

/// Queued depth for level subscriptions: sixteen windows hold ~67 ms of the
/// 240 Hz publication stream, covering both display-rate readers and the
/// slower control-side pump without dropping intermediate peaks.
const LEVEL_WINDOW_DEPTH: usize = 16;

/// The last tick of the shared meter scale, kept in lockstep with the final
/// `FaderScale::ANCHORS` entry (a test pins the two together). Below the tick
/// the painted track tail is amplitude-linear, so readings this far down have
/// effectively reached the bottom edge.
const SCALE_TICK_DB: f32 = -72.;

/// How far under the last tick a reading must fall before no layer can move
/// a visible pixel: the tail spans 3% of the track, and −24 dB under the tick
/// leaves every layer under a third of that tail's smallest gradation.
const REST_BELOW_TICK_DB: f32 = 24.;

/// A display parked at or under this reading is visually at the scale bottom;
/// the animation-frame loop can stand down until fresh data lifts a layer.
pub(super) const REST_DB: f32 = SCALE_TICK_DB - REST_BELOW_TICK_DB;

mod ballistics;
use crate::project::MeterDisplayPreferences;
use ballistics::ChannelBallistics;
#[cfg(test)]
use ballistics::{PEAK_HOLD, RMS_INTEGRATION};

/// Ballistics-processed presentation reading for one channel: the moving
/// peak-programme bar fill, the held recent-maximum marker, and the smoothed
/// RMS body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayLevel {
    pub bar: f32,
    pub marker: f32,
    pub rms: f32,
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
    settings: MeterDisplayPreferences,
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
            settings: MeterDisplayPreferences::default(),
            input_subscription: None,
            output_subscription: None,
        }
    }

    pub fn set_preferences(&mut self, settings: MeterDisplayPreferences) {
        self.settings = settings;
    }

    /// Test visibility: whether the legacy capture-tap subscription is live.
    /// A managed strip renders from NodeMeters alone, so the tap must stay
    /// unsubscribed there even while its probe is on screen.
    #[cfg(all(test, feature = "gpui-test"))]
    pub(crate) fn input_subscribed(&self) -> bool {
        self.input_subscription.is_some()
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
    pub fn display_input(&self) -> Option<[DisplayLevel; 2]> {
        self.input_subscription
            .as_ref()
            .and_then(|subscription| stereo_display(subscription.display.as_deref()))
    }

    /// Whether every observed point is parked at the scale bottom, so the
    /// display stays current without animation frames. Unobserved points
    /// hold no subscription and cannot move anything.
    pub fn at_rest(&self) -> bool {
        [&self.input_subscription, &self.output_subscription]
            .into_iter()
            .all(|subscription| subscription.as_ref().is_none_or(MeterSubscription::at_rest))
    }

    /// Ballistics-smoothed output reading for presentation; `None` exactly
    /// when the raw observation ([`Self::output`]) is unavailable.
    pub fn display_output(&self) -> Option<[DisplayLevel; 2]> {
        self.output_subscription
            .as_ref()
            .and_then(|subscription| stereo_display(subscription.display.as_deref()))
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
            // Which topics the running session feeds is a published fact
            // resolved by the service's meter bridge from the active host
            // graph — a tone session has no observable input at all.
            let (input_topic, output_topic) = if snapshot.tone_session {
                (None, &snapshot.signal_topics.tone_output)
            } else {
                (
                    Some(&snapshot.signal_topics.monitor_input),
                    &snapshot.signal_topics.monitor_output,
                )
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
            // The legacy path shares the NodeMeters wake policy (see
            // `NodeMeters::update_at`): a publication alone must not wake
            // the interface, because a running stream keeps publishing
            // windows whose readings may not move — notifying on each would
            // repaint at the control-poll rate forever. The projection
            // comparison below returns true exactly when a raw reading or a
            // status did, which still wakes every lifecycle transition
            // (fresh subscription, stall, hide, stop).
            if let Some(subscription) = self.input_subscription.as_mut() {
                subscription.settings = self.settings;
                subscription.poll(now);
                self.input = stereo_levels(subscription.levels.as_deref());
                self.input_status = subscription.status.clone();
            }
            if let Some(subscription) = self.output_subscription.as_mut() {
                subscription.settings = self.settings;
                subscription.poll(now);
                self.output = stereo_levels(subscription.levels.as_deref());
                self.output_status = subscription.status.clone();
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

fn stereo_display(display: Option<&[DisplayLevel]>) -> Option<[DisplayLevel; 2]> {
    let levels = display?;
    let [left, right] = levels else {
        return None;
    };
    Some([*left, *right])
}

struct MeterSubscription {
    topic: Topic<LevelTopic>,
    /// The level topic's subscription, latched once at construction: the
    /// meter path has no demand gating left — visibility is the caller's
    /// visible set, and the demand wrapper it used was hardcoded to "always
    /// wants" (design 17 deletions).
    ticket: Option<crate::service::SubscriptionTicket<LevelTopic>>,
    subscriber: Option<Subscriber<LevelTopic>>,
    sequence_fence: u64,
    last_delivery: Option<Instant>,
    last_window: Option<(u64, u64)>,
    levels: Option<Box<[ChannelLevel]>>,
    channels: usize,
    status: MeterStatus,
    settings: MeterDisplayPreferences,
    ballistics: Vec<ChannelBallistics>,
    display: Option<Box<[DisplayLevel]>>,
}

pub struct NodeMeters {
    handle: AppHandle,
    generation: Option<SessionGeneration>,
    settings: MeterDisplayPreferences,
    subscriptions: std::collections::BTreeMap<moiren_core::graph::NodeId, MeterSubscription>,
    clips: std::collections::BTreeMap<moiren_core::graph::NodeId, ClipHold>,
    /// Nodes seen visible recently; subscriptions outlive the visible set by
    /// [`SUBSCRIPTION_GRACE`] instead of flapping with per-frame visibility.
    seen: std::collections::BTreeMap<moiren_core::graph::NodeId, Instant>,
}

#[derive(Default)]
struct ClipHold(Vec<bool>);

impl ClipHold {
    /// Latches overload per channel and reports whether the latch state
    /// changed, so the pump can wake on a freshly lit clip without cloning
    /// the latch vector to compare (HC-14). A widening (new channels)
    /// counts as a change, exactly as the previous before/after compare did.
    fn observe(&mut self, levels: &[ChannelLevel]) -> bool {
        let mut changed = self.0.len() != levels.len();
        self.0.resize(levels.len(), false);
        for (clip, level) in self.0.iter_mut().zip(levels) {
            if !*clip && level.peak >= 1. {
                *clip = true;
                changed = true;
            }
        }
        changed
    }
}

impl NodeMeters {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            generation: None,
            settings: MeterDisplayPreferences::default(),
            subscriptions: Default::default(),
            clips: Default::default(),
            seen: Default::default(),
        }
    }

    pub fn set_preferences(&mut self, settings: MeterDisplayPreferences) {
        self.settings = settings;
    }

    pub fn update(
        &mut self,
        snapshot: &AppSnapshot,
        visible: impl IntoIterator<Item = moiren_core::graph::NodeId>,
    ) -> bool {
        self.update_at(snapshot, visible, Instant::now())
    }

    pub fn update_at(
        &mut self,
        snapshot: &AppSnapshot,
        visible: impl IntoIterator<Item = moiren_core::graph::NodeId>,
        now: Instant,
    ) -> bool {
        if self.generation != Some(snapshot.generation) {
            self.subscriptions.clear();
            self.clips.clear();
            self.seen.clear();
            self.generation = Some(snapshot.generation);
        }
        // Meters belong to the running session: candidate edits and the
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
                && snapshot.signal_topics.node_levels.contains_key(id)
        });
        self.clips
            .retain(|id, _| snapshot.graph.node(*id).is_some());
        let mut changed = before != self.subscriptions.len();
        if !active {
            return changed;
        }
        // Borrow the seen table for the whole pass: the body only touches
        // the sibling maps, and cloning the table here would allocate on
        // every pump tick for every visible node (HC-14).
        for id in self.seen.keys() {
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
            subscription.settings = self.settings;
            let poll = subscription.poll(now);
            // A publication alone must not wake the interface: a running
            // stream publishes silence windows too, and notifying on each
            // would repaint at the control-poll rate forever. Only a
            // display-bearing change — raw readings, status, or a freshly
            // latched clip — justifies a repaint.
            changed |= subscription.status != status || poll.raw_changed;
            if poll.published
                && let Some(levels) = subscription.levels.as_deref()
            {
                let clips = self.clips.entry(*id).or_default();
                changed |= clips.observe(levels);
            }
        }
        changed
    }

    /// Whether every observed node is parked at the scale bottom, so the
    /// display stays current without animation frames.
    pub fn at_rest(&self) -> bool {
        self.subscriptions.values().all(MeterSubscription::at_rest)
    }

    /// Ballistics-smoothed stereo projection for presentation.
    pub fn levels(&self, id: moiren_core::graph::NodeId) -> Option<[DisplayLevel; 2]> {
        let levels: &[DisplayLevel; 2] = self.channel_levels(id)?.try_into().ok()?;
        Some(*levels)
    }

    /// Ballistics-smoothed per-channel readings for presentation.
    pub fn channel_levels(&self, id: moiren_core::graph::NodeId) -> Option<&[DisplayLevel]> {
        self.subscriptions
            .get(&id)
            .and_then(|subscription| subscription.display.as_deref())
    }

    pub fn raw_levels(&self, id: moiren_core::graph::NodeId) -> Option<&[ChannelLevel]> {
        self.subscriptions
            .get(&id)
            .and_then(|subscription| subscription.levels.as_deref())
    }

    pub fn status(&self, id: moiren_core::graph::NodeId) -> MeterStatus {
        self.subscriptions
            .get(&id)
            .map(|subscription| subscription.status.clone())
            .unwrap_or(MeterStatus::Waiting)
    }

    pub fn clips(&self, id: moiren_core::graph::NodeId) -> [bool; 2] {
        let clips = self.channel_clips(id);
        [
            clips.first().copied().unwrap_or(false),
            clips.get(1).copied().unwrap_or(false),
        ]
    }

    pub fn channel_clips(&self, id: moiren_core::graph::NodeId) -> &[bool] {
        self.clips
            .get(&id)
            .map(|hold| hold.0.as_slice())
            .unwrap_or_default()
    }

    pub fn reset_clip(&mut self, id: moiren_core::graph::NodeId) {
        self.clips.remove(&id);
    }
}

/// What one pump of a subscription observed. `published` says windows were
/// accepted; `raw_changed` says the stored reading moved. Callers may wake
/// the interface only on the latter (plus status and clip changes): a
/// running stream publishes unchanged windows at its own cadence, so a
/// publication alone must not notify, and reporting the change from the
/// drain itself spares the caller a pre-poll clone of every reading.
struct MeterPoll {
    published: bool,
    raw_changed: bool,
}

impl MeterSubscription {
    /// Whether every layer of every channel is parked at the scale bottom:
    /// further frames would not move anything a viewer can see.
    fn at_rest(&self) -> bool {
        self.ballistics.iter().all(|channel| channel.at_rest())
    }

    fn new(handle: AppHandle, topic: Topic<LevelTopic>, channels: usize) -> Self {
        let mut this = Self {
            sequence_fence: topic.stats().attempts,
            ticket: None,
            subscriber: None,
            topic,
            last_delivery: None,
            last_window: None,
            levels: None,
            channels,
            status: MeterStatus::Waiting,
            settings: MeterDisplayPreferences::default(),
            ballistics: Vec::new(),
            display: None,
        };
        match handle.subscribe_topic(
            this.topic.clone(),
            SubscriptionOptions {
                stream_depth: Some(LEVEL_WINDOW_DEPTH),
            },
        ) {
            Ok(ticket) => this.ticket = Some(ticket),
            Err(error) => {
                this.status = MeterStatus::Error(
                    t!("Meter subscription: %{error}", error = format!("{error:?}")).to_string(),
                );
            }
        }
        this
    }

    fn poll(&mut self, now: Instant) -> MeterPoll {
        let poll = self.poll_windows(now);
        self.advance_display(now);
        poll
    }

    /// Drains every queued window; reports whether a new publication was
    /// accepted and whether the raw reading moved. Every drained window
    /// contributes its peak and its RMS share, so intermediate windows
    /// between display reads are never lost.
    fn poll_windows(&mut self, now: Instant) -> MeterPoll {
        let mut raw_changed = false;
        let status = self.topic.status();
        if status.lifecycle == TopicLifecycle::Closed
            || status.publisher == PublisherStatus::NoPublisher
        {
            raw_changed |= self.levels.is_some();
            self.levels = None;
            self.last_delivery = None;
            self.sequence_fence = self.topic.stats().attempts;
            self.status = MeterStatus::NoPublisher;
            return MeterPoll {
                published: false,
                raw_changed,
            };
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
                    raw_changed |= self.levels.is_some();
                    self.levels = None;
                    self.status = MeterStatus::Error(
                        t!("Meter subscription: %{error}", error = error.to_string()).to_string(),
                    );
                    return MeterPoll {
                        published: false,
                        raw_changed,
                    };
                }
            }
        }
        let Some(subscriber) = self.subscriber.as_mut() else {
            self.status = MeterStatus::Waiting;
            return MeterPoll {
                published: false,
                raw_changed,
            };
        };
        let mut published = false;
        while let Some(window) = subscriber.next() {
            let meta = window.meta();
            let header = *window.header();
            let elements = window.elements();
            if !valid_level_window(
                self.channels,
                meta.sequence,
                self.sequence_fence,
                header,
                elements,
                self.last_window,
            ) {
                continue;
            }
            self.sequence_fence = meta.sequence;
            self.last_window = Some((header.timeline_epoch, header.frame_end));
            self.last_delivery = Some(now);
            self.status = MeterStatus::Live;
            published = true;
            match self.levels.as_deref_mut() {
                Some(levels) => {
                    for (level, element) in levels.iter_mut().zip(elements) {
                        raw_changed |= element.peak > level.peak || element.rms != level.rms;
                        level.peak = level.peak.max(element.peak);
                        level.rms = element.rms;
                    }
                }
                None => {
                    self.levels = Some(elements.to_vec().into_boxed_slice());
                    raw_changed = true;
                }
            }
            let duration = window_secs(header);
            sync_ballistics(&mut self.ballistics, &mut self.display, self.channels);
            for (channel, element) in self.ballistics.iter_mut().zip(elements) {
                channel.ingest(*element, duration, now);
            }
        }
        if published {
            return MeterPoll {
                published: true,
                raw_changed,
            };
        }
        if self
            .last_delivery
            .is_some_and(|last| now.saturating_duration_since(last) >= STALE_AFTER)
        {
            raw_changed |= self.levels.is_some();
            self.levels = None;
            self.status = MeterStatus::Stale;
        } else if self.last_delivery.is_none() {
            self.status = MeterStatus::Waiting;
        }
        MeterPoll {
            published: false,
            raw_changed,
        }
    }

    /// Step the display ballistics. Runs on every pump (display frame or
    /// control-side timer), so decay and peak release continue smoothly
    /// between publications; the display is `None` exactly when the raw
    /// observation is.
    fn advance_display(&mut self, now: Instant) {
        let Some(levels) = self.levels.as_deref() else {
            self.display = None;
            self.ballistics.clear();
            return;
        };
        sync_ballistics(&mut self.ballistics, &mut self.display, levels.len());
        let display = self.display.as_deref_mut().expect("sized with ballistics");
        for (channel, shown) in self.ballistics.iter_mut().zip(display.iter_mut()) {
            channel.settings = self.settings;
            channel.advance(now);
            *shown = channel.level();
        }
    }
}

fn sync_ballistics(
    ballistics: &mut Vec<ChannelBallistics>,
    display: &mut Option<Box<[DisplayLevel]>>,
    channels: usize,
) {
    if ballistics.len() != channels
        || display
            .as_deref()
            .is_none_or(|display| display.len() != channels)
    {
        *ballistics = (0..channels).map(|_| ChannelBallistics::new()).collect();
        *display = Some(
            vec![
                DisplayLevel {
                    bar: 0.,
                    marker: 0.,
                    rms: 0.
                };
                channels
            ]
            .into(),
        );
    }
}

/// True window duration; validation guarantees positive frames and rate.
fn window_secs(header: AudioWindow) -> Duration {
    Duration::from_secs_f64((header.frame_end - header.frame_start) as f64 / header.sample_rate)
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

    const WINDOW: Duration = Duration::from_millis(4);

    #[test]
    fn ballistics_reach_rest_only_after_every_layer_decays_under_the_scale_bottom() {
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        assert!(channel.at_rest(), "fresh silence starts at rest");
        channel.ingest(ChannelLevel { peak: 1., rms: 0.5 }, WINDOW, start);
        channel.advance(start);
        assert!(!channel.at_rest(), "a loud window breaks rest");
        // Digital silence at the real publication cadence: windows keep
        // arriving every 4 ms, the bar falls at the default Fast 40 dB/s,
        // the marker holds 0 dBFS for the default half second, and the RMS
        // body glides down with its integrator.
        for tick in 1..=800u32 {
            let now = start + WINDOW * tick;
            channel.ingest(ChannelLevel { peak: 0., rms: 0. }, WINDOW, now);
            channel.advance(now);
            if matches!(tick * 4, 100 | 200 | 400 | 800) {
                assert!(!channel.at_rest(), "still decaying at {} ms", tick * 4);
            }
        }
        assert!(
            channel.at_rest(),
            "every layer parks at the scale bottom eventually"
        );
    }

    #[test]
    fn sub_visible_readings_keep_the_display_at_rest() {
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        channel.ingest(
            ChannelLevel {
                peak: 10f32.powf(REST_DB / 20.),
                rms: 0.,
            },
            WINDOW,
            start,
        );
        channel.advance(start);
        assert!(
            channel.at_rest(),
            "a reading already under the visible bottom cannot move a pixel"
        );
    }

    #[test]
    fn rest_threshold_tracks_the_shared_fader_scale_floor() {
        use crate::ui::widgets::fader::FaderScale;
        let &(tick, _) = FaderScale::ANCHORS.last().expect("non-empty");
        assert_eq!(
            tick as f32, SCALE_TICK_DB,
            "the rest threshold must follow the painted scale's last tick"
        );
        const {
            assert!(REST_DB < SCALE_TICK_DB);
        }
    }

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
        assert!(hold.0.is_empty());
    }

    #[test]
    fn clip_hold_retains_overloads_beyond_the_stereo_pair() {
        let mut hold = ClipHold::default();
        let mut levels = [ChannelLevel { peak: 0., rms: 0. }; 6];
        levels[5].peak = 1.2;
        hold.observe(&levels);
        levels[5].peak = 0.;
        hold.observe(&levels);
        assert_eq!(
            hold.0.as_slice(),
            &[false, false, false, false, false, true]
        );
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
    fn falling_peak_never_undershoots_the_current_received_target() {
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        channel.ingest(ChannelLevel { peak: 1., rms: 0. }, WINDOW, start);
        channel.advance(start);
        channel.ingest(
            ChannelLevel { peak: 0.1, rms: 0. },
            WINDOW,
            start + Duration::from_secs(10),
        );
        channel.advance(start + Duration::from_secs(10));
        assert!(channel.level().bar >= 0.1);
        assert!(channel.level().marker >= channel.level().bar);
    }

    #[test]
    fn ballistics_snap_the_peak_to_the_first_window_and_integrate_rms() {
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        channel.ingest(
            ChannelLevel {
                peak: 0.5,
                rms: 0.3,
            },
            WINDOW,
            start,
        );
        channel.advance(start);
        let level = channel.level();
        assert!((level.bar - 0.5).abs() < 0.000001, "the peak snaps");
        assert!((level.marker - 0.5).abs() < 0.000001, "the marker snaps");

        // The body integrates from silence, so one integration constant of a
        // constant 0.4 RMS reading moves it toward the target without
        // overshooting it.
        channel.ingest(
            ChannelLevel {
                peak: 0.5,
                rms: 0.4,
            },
            RMS_INTEGRATION,
            start + WINDOW,
        );
        channel.advance(start + WINDOW + RMS_INTEGRATION);
        let risen = channel.level().rms;
        assert!(
            risen > level.rms && risen < 0.4,
            "body {risen} rises toward the raw window without reaching it"
        );
    }

    #[test]
    fn ballistics_marker_holds_then_falls_at_the_preference_rate() {
        let start = Instant::now();
        let rate = crate::project::MeterFalloff::Fast.db_per_second();
        let mut channel = ChannelBallistics::new();
        channel.ingest(ChannelLevel { peak: 0.5, rms: 0. }, WINDOW, start);
        channel.advance(start);
        // Quieter windows never lower the marker while the hold lasts.
        channel.ingest(
            ChannelLevel { peak: 0.1, rms: 0. },
            WINDOW,
            start + Duration::from_millis(100),
        );
        channel.advance(start + Duration::from_millis(100));
        channel.ingest(
            ChannelLevel { peak: 0.1, rms: 0. },
            WINDOW,
            start + PEAK_HOLD,
        );
        channel.advance(start + PEAK_HOLD);
        assert!((channel.level().marker - 0.5).abs() < 0.000001);

        // 100 ms into the fall: −6.02 dB less the fall at the preference rate.
        channel.ingest(
            ChannelLevel { peak: 0., rms: 0. },
            WINDOW,
            start + PEAK_HOLD + Duration::from_millis(100),
        );
        channel.advance(start + PEAK_HOLD + Duration::from_millis(100));
        let db = 20. * channel.level().marker.max(f32::MIN_POSITIVE).log10();
        assert!((db - (-6.0206 - rate * 0.1)).abs() < 0.01);

        // A later, louder window raises the marker instantly and re-holds.
        channel.ingest(
            ChannelLevel { peak: 0.8, rms: 0. },
            WINDOW,
            start + PEAK_HOLD + Duration::from_millis(200),
        );
        channel.advance(start + PEAK_HOLD + Duration::from_millis(200));
        assert!((channel.level().marker - 0.8).abs() < 0.000001);
    }

    #[test]
    fn ballistics_peak_bar_moves_immediately_without_hold() {
        let start = Instant::now();
        let rate = crate::project::MeterFalloff::Fast.db_per_second();
        let mut channel = ChannelBallistics::new();
        channel.ingest(ChannelLevel { peak: 0.5, rms: 0. }, WINDOW, start);
        channel.advance(start);
        // A quieter window starts the fallback at once — the bar never pins
        // at the recent maximum the way the marker does.
        channel.ingest(
            ChannelLevel { peak: 0.1, rms: 0. },
            WINDOW,
            start + Duration::from_millis(100),
        );
        channel.advance(start + Duration::from_millis(100));
        let level = channel.level();
        assert!((level.marker - 0.5).abs() < 0.000001, "marker still holds");
        // 100 ms of fall below −6.02 dB, floored by the quieter target.
        let db = 20. * level.bar.max(f32::MIN_POSITIVE).log10();
        let expected = (-6.0206 - rate * 0.1).max(20. * 0.1f32.log10());
        assert!((db - expected).abs() < 0.01, "bar fell to {db} dB");

        // A louder window raises the bar instantly.
        channel.ingest(
            ChannelLevel { peak: 0.8, rms: 0. },
            WINDOW,
            start + Duration::from_millis(200),
        );
        channel.advance(start + Duration::from_millis(200));
        assert!((channel.level().bar - 0.8).abs() < 0.000001);
    }

    #[test]
    fn rms_body_glides_through_sustained_silence_instead_of_dropping_to_zero() {
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        for _ in 0..1_500 {
            channel.ingest(ChannelLevel { peak: 0., rms: 0.5 }, WINDOW, start);
        }
        channel.advance(start);
        let settled_db = 20. * channel.level().rms.max(f32::MIN_POSITIVE).log10();
        // One real second of digital silence keeps publishing windows; the
        // integrator decays exponentially (~29 dB per second at 150 ms) and
        // the display follows it downward instead of stepping to zero.
        for step in 0..250 {
            channel.ingest(
                ChannelLevel { peak: 0., rms: 0. },
                WINDOW,
                start + Duration::from_millis(step * 4),
            );
        }
        channel.advance(start + Duration::from_secs(1));
        let display_db = 20. * channel.level().rms.max(f32::MIN_POSITIVE).log10();
        assert!(
            display_db < settled_db - 20.,
            "body {display_db} dB did not glide down from {settled_db} dB"
        );
        assert!(
            display_db > settled_db - 60.,
            "body {display_db} dB snapped to silence from {settled_db} dB"
        );
    }

    #[test]
    fn rms_integration_flattens_window_envelope_ripple() {
        // A short RMS window swings with the bass envelope; feed that swing
        // (±6 dB at 5 Hz) and require the integrated body to keep the
        // displayed swing to a fraction of the input swing.
        let start = Instant::now();
        let mut channel = ChannelBallistics::new();
        let mut high: f32 = 0.;
        let mut low: f32 = 1.;
        for step in 0..40 {
            let rms = if step % 2 == 0 { 1. } else { 0.25 };
            channel.ingest(
                ChannelLevel { peak: 0., rms },
                Duration::from_millis(100),
                start + Duration::from_millis(100 * (step + 1)),
            );
            channel.advance(start + Duration::from_millis(100 * (step + 1)));
            // Skip the settling transient.
            if step >= 20 {
                high = high.max(channel.level().rms);
                low = low.min(channel.level().rms);
            }
        }
        let swing_db = 20. * (high / low).log10();
        assert!(
            swing_db < 3.,
            "displayed swing {swing_db} dB is not smoothed against the 12 dB input swing"
        );
    }

    /// The legacy source/output probe must follow the NodeMeters wake policy:
    /// a running stream publishes windows at its own cadence whether or not
    /// the readings move, so only a reading or status change may wake the
    /// interface (SD-8). Driven by the fake engine's constant source, whose
    /// windows stand in for silence: arriving, valid, and bit-identical.
    #[test]
    fn legacy_meter_wakes_on_changes_only_not_on_publications() {
        use std::sync::atomic::Ordering;

        use crate::service::{
            AppRequest, GraphEdit, InputSelection, ManagedDriver, RunLimit, ServiceRuntime,
            SessionPhase, SessionSpec, test_backend::FakeBackend,
        };
        use crate::ui::{finish_service, wait_for, wait_until};

        let backend = FakeBackend::default();
        backend.0.timed_meter_frames.store(96, Ordering::Relaxed);
        let backend_state = backend.0.clone();
        let runtime = ServiceRuntime::spawn_with_driver(ManagedDriver::new(move |_| {
            Ok(Box::new(backend.clone()))
        }))
        .unwrap();
        let handle = runtime.handle();
        handle
            .try_request(AppRequest::Start(SessionSpec {
                source: InputSelection::Physical {
                    endpoint_id: "input".into(),
                },
                output_endpoint_id: "output".into(),
                limit: RunLimit::UntilStopped,
                gain: 1.,
                pan: 0.,
                max_block_frames: 256,
            }))
            .unwrap();
        let snapshot = wait_for(Duration::from_secs(3), || {
            handle
                .try_snapshot()
                .filter(|snapshot| snapshot.phase == SessionPhase::Running)
        });

        let mut meters = MeterSubscriptions::new(handle.clone());
        // Fresh demand must wake once the first windows land (Waiting → Live,
        // no reading → first window): the new-subscription path stays loud.
        wait_until(Duration::from_secs(3), || {
            meters.update_points(&snapshot, true, true)
        });
        // Let the relayed readings converge on the constant source, then
        // require the stream to fall asleep: windows keep arriving, the
        // readings never move, so no poll may return true again.
        std::thread::sleep(Duration::from_secs(1));
        wait_until(Duration::from_secs(3), || {
            !meters.update_points(&snapshot, true, true)
                && !meters.update_points(&snapshot, true, true)
        });
        assert!(
            !meters.update_points(&snapshot, true, true),
            "a publication whose readings did not change must not wake"
        );

        // A raw reading change must wake even though the stream keeps
        // publishing: silencing the sink's feed drops its tap from full
        // scale to zero.
        let graph = &snapshot.graph.graph;
        let sink = graph
            .nodes()
            .iter()
            .find(|node| node.kind() == moiren_core::graph::NodeKind::Sink)
            .expect("the template graph carries a sink");
        let feed = graph
            .edges()
            .iter()
            .find(|edge| {
                sink.inputs()
                    .iter()
                    .any(|port| port.id() == edge.dst_port())
            })
            .expect("the template graph wires the sink");
        handle
            .try_request(AppRequest::EditGraph(GraphEdit::Disconnect(feed.id())))
            .unwrap();
        wait_until(Duration::from_secs(3), || {
            handle.try_snapshot().is_some_and(|snapshot| {
                !snapshot
                    .graph
                    .graph
                    .edges()
                    .iter()
                    .any(|edge| edge.id() == feed.id())
            })
        });
        wait_until(Duration::from_secs(3), || {
            meters.update_points(&snapshot, true, true)
        });

        // A delivery stall is a status change: it must wake exactly once,
        // then stay quiet while the stall persists.
        backend_state.pause_audio.store(true, Ordering::Relaxed);
        std::thread::sleep(STALE_AFTER + Duration::from_millis(200));
        assert!(
            meters.update_points(&snapshot, true, true),
            "the transition to Stale must wake"
        );
        assert!(
            !meters.update_points(&snapshot, true, true),
            "a persisted stall without new publications must not keep waking"
        );

        // Recovery is a status change too: Stale → Live with fresh readings.
        backend_state.pause_audio.store(false, Ordering::Relaxed);
        wait_until(Duration::from_secs(3), || {
            meters.update_points(&snapshot, true, true)
        });

        finish_service(runtime);
    }
}
