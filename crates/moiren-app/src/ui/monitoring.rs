//! Visible, session-scoped source/output Peak and RMS observations.
//!
//! Legacy source/output topics and managed per-node topics share lifecycle
//! validation and display envelopes.

use std::time::{Duration, Instant};

use moiren_engine::meter::signal::{AudioWindow, ChannelLevel};
use moiren_signal::{PublisherStatus, SubscriptionOptions, Topic, TopicLifecycle};
use rust_i18n::t;

use crate::service::{
    AppHandle, AppSnapshot, InputSelection, LevelTopic, SessionGeneration, SessionPhase,
};

use super::signals::{PresentationDemand, VisibleSignal};

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

    /// Ballistics-smoothed output reading for presentation; `None` exactly
    /// when the raw observation ([`Self::output`]) is unavailable.
    pub fn display_output(&self) -> Option<[DisplayLevel; 2]> {
        self.output_subscription
            .as_ref()
            .and_then(|subscription| stereo_display(subscription.display.as_deref()))
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
                subscription.settings = self.settings;
                publication |= subscription.poll(now);
                self.input = stereo_levels(subscription.levels.as_deref());
                self.input_status = subscription.status.clone();
            }
            if let Some(subscription) = self.output_subscription.as_mut() {
                subscription.settings = self.settings;
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

fn stereo_display(display: Option<&[DisplayLevel]>) -> Option<[DisplayLevel; 2]> {
    let levels = display?;
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
    fn observe(&mut self, levels: &[ChannelLevel]) {
        self.0.resize(levels.len(), false);
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
        visible: &std::collections::BTreeSet<moiren_core::graph::NodeId>,
    ) -> bool {
        self.update_at(snapshot, visible, Instant::now())
    }

    pub fn update_at(
        &mut self,
        snapshot: &AppSnapshot,
        visible: &std::collections::BTreeSet<moiren_core::graph::NodeId>,
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
            self.seen.insert(*id, now);
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
        for (id, _) in self.seen.clone() {
            let Some(topic) = snapshot.signal_topics.node_levels.get(&id) else {
                continue;
            };
            let Some(channels) = snapshot
                .graph
                .graph
                .get_node(id)
                .ok()
                .map(|node| node.channels())
            else {
                continue;
            };
            let subscription = self.subscriptions.entry(id).or_insert_with(|| {
                MeterSubscription::new(self.handle.clone(), topic.clone(), channels)
            });
            if subscription.topic != *topic || subscription.channels != channels {
                *subscription =
                    MeterSubscription::new(self.handle.clone(), topic.clone(), channels);
                self.clips.remove(&id);
            }
            let status = subscription.status.clone();
            subscription.settings = self.settings;
            let publication = subscription.poll(now);
            changed |= publication || subscription.status != status;
            if publication && let Some(levels) = subscription.levels.as_deref() {
                self.clips.entry(id).or_default().observe(levels);
            }
        }
        changed
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

impl MeterSubscription {
    fn new(handle: AppHandle, topic: Topic<LevelTopic>, channels: usize) -> Self {
        Self {
            sequence_fence: topic.stats().attempts,
            visible: VisibleSignal::new(
                handle,
                topic.clone(),
                SubscriptionOptions {
                    stream_depth: Some(LEVEL_WINDOW_DEPTH),
                },
            ),
            topic,
            last_delivery: None,
            last_window: None,
            levels: None,
            channels,
            status: MeterStatus::Waiting,
            settings: MeterDisplayPreferences::default(),
            ballistics: Vec::new(),
            display: None,
        }
    }

    fn poll(&mut self, now: Instant) -> bool {
        let published = self.poll_windows(now);
        self.advance_display(now);
        published
    }

    /// Drains every queued window; returns whether a new publication was
    /// accepted. Every drained window contributes its peak and its RMS share,
    /// so intermediate windows between display reads are never lost.
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
            self.status = MeterStatus::Error(
                t!("Meter subscription: %{error}", error = format!("{error:?}")).to_string(),
            );
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
                self.status = MeterStatus::Error(
                    t!("Meter subscription: %{error}", error = error.to_string()).to_string(),
                );
                return false;
            }
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
                        level.peak = level.peak.max(element.peak);
                        level.rms = element.rms;
                    }
                }
                None => self.levels = Some(elements.to_vec().into_boxed_slice()),
            }
            let duration = window_secs(header);
            sync_ballistics(&mut self.ballistics, &mut self.display, self.channels);
            for (channel, element) in self.ballistics.iter_mut().zip(elements) {
                channel.ingest(*element, duration, now);
            }
        }
        if published {
            return true;
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

    /// One pump advances each stream topic only a few dispatch steps, so a
    /// queue of level windows takes several 1 ms polls to drain. Mirror the
    /// service-thread cadence until the bus reports nothing pending.
    fn pump_settled(context: &mut ServiceContext, now: &mut Instant) {
        for _ in 0..64 {
            *now += Duration::from_millis(1);
            match context.signals.pump_if_due(*now) {
                Some(report) if report.has_pending => {}
                _ => return,
            }
        }
    }

    #[test]
    fn node_meter_demand_is_per_node_visible_and_survives_candidate_edits() {
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
        let mut meters = NodeMeters::new(handle);
        let visible = [id].into_iter().collect();
        let now = Instant::now();
        meters.update_at(&snapshot, &visible, now);
        drain(&mailbox, &mut context);
        assert_eq!(
            snapshot.signal_topics.node_levels[&id].business_subscribers(),
            1
        );
        assert_eq!(
            snapshot.signal_topics.node_levels[&other].business_subscribers(),
            0
        );
        // A candidate edit (revision drift while a rebuild is pending) must
        // not interrupt meter demand or drop the current levels.
        std::sync::Arc::make_mut(&mut snapshot.graph).candidate_revision += 1;
        meters.update_at(&snapshot, &visible, now);
        assert_eq!(
            snapshot.signal_topics.node_levels[&id].business_subscribers(),
            1
        );
        // Leaving visibility keeps the subscription for the grace period so a
        // one-frame viewport flap cannot reset the RT window, then releases it.
        meters.update_at(&snapshot, &Default::default(), now);
        assert_eq!(
            snapshot.signal_topics.node_levels[&id].business_subscribers(),
            1,
            "demand survives a short invisibility gap"
        );
        meters.update_at(&snapshot, &Default::default(), now + SUBSCRIPTION_GRACE);
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
        tone.compiled.engine.render(199).unwrap();
        tone.output.read_interleaved(&mut [0.; 398]).unwrap();
        context.signals.pump_if_due(now).unwrap();
        meters.update_at(&snapshot, true, now);
        assert_eq!(meters.output, None, "partial windows are unknown");
        assert_eq!(meters.output_status, MeterStatus::Waiting);
        tone.compiled.engine.render(1).unwrap();
        tone.output.read_interleaved(&mut [0.; 2]).unwrap();
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
        let expected_rms = ((0..200)
            .map(|frame| {
                (std::f64::consts::TAU * 440. * f64::from(frame) / 48_000.)
                    .sin()
                    .powi(2)
            })
            .sum::<f64>()
            / 200.)
            .sqrt()
            * 0.01;
        assert!((f64::from(right.rms) - expected_rms).abs() < 0.000001);
        assert_eq!(meters.output_status, MeterStatus::Live);
        assert_eq!(meters.input, None);
        let [raw_left, raw_right] = meters.output.unwrap();
        let display = meters.display_output().unwrap();
        // The first live window snaps the peak layers to the raw reading;
        // the RMS body integrates from silence, so it starts below the raw
        // window RMS and climbs as further windows arrive.
        assert!((display[0].bar - raw_left.peak).abs() < 0.000001);
        assert!((display[1].bar - raw_right.peak).abs() < 0.000001);
        assert!((display[0].marker - raw_left.peak).abs() < 0.000001);
        assert!((display[1].marker - raw_right.peak).abs() < 0.000001);
        assert!(display[1].rms > 0. && display[1].rms < raw_right.rms);
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
        let mut now = Instant::now();
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        pump_settled(&mut context, &mut now);
        let mut meters = MeterSubscriptions::new(handle);
        meters.update_at(&snapshot, true, now);
        drain(&mailbox, &mut context);
        meters.update_at(&snapshot, true, now);
        assert_eq!(meters.output, None, "cache from before UI demand is fenced");
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        pump_settled(&mut context, &mut now);
        meters.update_at(&snapshot, true, now);
        assert!(meters.output.is_some());
        meters.update_at(&snapshot, false, now);
        assert_eq!(topic.business_subscribers(), 1);
        assert_eq!(topic.demand_snapshot().generation, demand_generation);
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        pump_settled(&mut context, &mut now);
        meters.update_at(&snapshot, true, now);
        drain(&mailbox, &mut context);
        meters.update_at(&snapshot, true, now);
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
        pump_settled(&mut context, &mut now);
        meters.update_at(&snapshot, true, now);
        drain(&mailbox, &mut context);
        meters.update_at(&snapshot, true, now);
        assert_eq!(
            meters.output, None,
            "new session begins unknown until a post-reset publication"
        );
        render(&mut tone.compiled.engine, &mut tone.output, 6);
        pump_settled(&mut context, &mut now);
        meters.update_at(&snapshot, true, now);
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
        let mut now = Instant::now();
        meters.update_points_at(&snapshot, true, true, now);
        drain(&mailbox, &mut context);
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        pump_settled(&mut context, &mut now);
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
        pump_settled(&mut context, &mut now);
        assert!(meters.update_points_at(&snapshot, false, true, now));
        assert_eq!(input_topic.stats().attempts, input_attempts);
        assert_eq!(output_topic.demand_snapshot().generation, output_generation);
        assert!((meters.output.unwrap()[0].peak - 0.1).abs() < 0.00001);

        meters.update_points_at(&snapshot, true, true, now);
        drain(&mailbox, &mut context);
        meters.update_points_at(&snapshot, true, true, now);
        assert_eq!(
            meters.input, None,
            "reappearing point waits for its own fresh window"
        );
        assert_eq!(output_topic.demand_snapshot().generation, output_generation);
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        pump_settled(&mut context, &mut now);
        meters.update_points_at(&snapshot, true, true, now);
        assert_eq!(meters.input_status, MeterStatus::Live);
        assert!((meters.input.unwrap()[0].peak - 0.4).abs() < 0.00001);
        let input_generation = input_topic.demand_snapshot().generation;

        assert!(meters.update_points_at(&snapshot, true, false, now));
        assert_eq!(output_topic.business_subscribers(), 0);
        assert_eq!(meters.output, None);
        assert_eq!(meters.output_status, MeterStatus::Hidden);
        assert_eq!(input_topic.business_subscribers(), 1);
        assert_eq!(meters.input_status, MeterStatus::Live);
        render(&mut monitor.compiled.engine, &mut monitor.output, 6);
        pump_settled(&mut context, &mut now);
        assert!(meters.update_points_at(&snapshot, true, false, now));
        assert_eq!(input_topic.demand_snapshot().generation, input_generation);
        assert_eq!(meters.input_status, MeterStatus::Live);
    }
}
