//! Visibility-owned spectrum observations; the service owns every publisher.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::service::{
    AppHandle, AppSnapshot, SessionGeneration, SessionPhase, SpectrumBinding, SpectrumSettings,
    SpectrumTopic, SpectrumWindow,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_signal::{PublisherStatus, TopicLifecycle};

use super::signals::{PresentationDemand, VisibleSignal};

const STALE_AFTER: Duration = Duration::from_millis(500);
const SUBSCRIPTION_GRACE: Duration = Duration::from_millis(750);

fn freshness_timeout(settings: SpectrumSettings, sample_rate: f64) -> Option<Duration> {
    if !settings.valid() || !sample_rate.is_finite() || sample_rate <= 0. {
        return None;
    }
    // Two active hops plus scheduling slack also covers slow, large FFTs.
    // Checked conversion rejects unusably small rates without panicking.
    Duration::try_from_secs_f64(2. * settings.hop_frames as f64 / sample_rate + 0.1)
        .ok()
        .map(|timeout| timeout.max(STALE_AFTER))
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumObservation {
    pub window: SpectrumWindow,
    pub magnitudes: Arc<[f32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpectrumStatus {
    Hidden,
    Stopped,
    Waiting,
    Live,
    Stale,
    Unavailable,
    Error(String),
}

impl SpectrumStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Hidden => "Hidden",
            Self::Stopped => "Stopped",
            Self::Waiting => "Waiting for a complete spectrum window",
            Self::Live => "Live",
            Self::Stale => "No recent spectrum window",
            Self::Unavailable => "Spectrum unavailable",
            Self::Error(error) => error,
        }
    }
}

pub struct NodeSpectra {
    handle: AppHandle,
    generation: Option<SessionGeneration>,
    running: bool,
    visible: BTreeSet<NodeId>,
    subscriptions: BTreeMap<NodeId, SpectrumSubscription>,
}

impl NodeSpectra {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            generation: None,
            running: false,
            visible: BTreeSet::new(),
            subscriptions: BTreeMap::new(),
        }
    }

    pub fn update(&mut self, snapshot: &AppSnapshot, visible_node_ids: &BTreeSet<NodeId>) -> bool {
        self.update_at(snapshot, visible_node_ids, Instant::now())
    }

    pub fn observation(&self, id: NodeId) -> Option<&SpectrumObservation> {
        let subscription = self.subscriptions.get(&id)?;
        (self.visible.contains(&id)
            && subscription.status == SpectrumStatus::Live
            && !subscription.stale_at(Instant::now()))
        .then_some(subscription.observation.as_ref())
        .flatten()
    }

    pub fn status(&self, id: NodeId, snapshot: &AppSnapshot) -> SpectrumStatus {
        if !self.visible.contains(&id) {
            return SpectrumStatus::Hidden;
        }
        if snapshot.phase != SessionPhase::Running {
            return SpectrumStatus::Stopped;
        }
        let Some(binding) = active_binding(snapshot, id) else {
            return SpectrumStatus::Unavailable;
        };
        let topic_status = binding.topic.status();
        if topic_status.lifecycle == TopicLifecycle::Closed
            || topic_status.publisher == PublisherStatus::NoPublisher
        {
            return SpectrumStatus::Unavailable;
        }
        let Some(subscription) = self.subscriptions.get(&id) else {
            return SpectrumStatus::Waiting;
        };
        // A renderer may see a newer service snapshot before the next poll.
        if self.generation != Some(snapshot.generation) || !subscription.matches(binding) {
            return SpectrumStatus::Waiting;
        }
        if subscription.status == SpectrumStatus::Live && subscription.stale_at(Instant::now()) {
            SpectrumStatus::Stale
        } else {
            subscription.status.clone()
        }
    }

    fn update_at(
        &mut self,
        snapshot: &AppSnapshot,
        visible: &BTreeSet<NodeId>,
        now: Instant,
    ) -> bool {
        let running = snapshot.phase == SessionPhase::Running;
        let mut changed = self.visible != *visible || self.running != running;
        self.visible.clone_from(visible);
        self.running = running;
        if self.generation != Some(snapshot.generation) {
            self.subscriptions.clear();
            self.generation = Some(snapshot.generation);
            changed = true;
        }
        if !running {
            changed |= !self.subscriptions.is_empty();
            self.subscriptions.clear();
            return changed;
        }

        let count = self.subscriptions.len();
        self.subscriptions.retain(|id, subscription| {
            if !active_binding(snapshot, *id).is_some_and(|binding| subscription.matches(binding)) {
                return false;
            }
            if visible.contains(id) {
                return true;
            }
            changed |= subscription.hide(now);
            subscription
                .hidden_since
                .is_some_and(|hidden| now.saturating_duration_since(hidden) < SUBSCRIPTION_GRACE)
        });
        changed |= count != self.subscriptions.len();
        for id in visible {
            let Some(binding) = active_binding(snapshot, *id) else {
                continue;
            };
            let subscription = self.subscriptions.entry(*id).or_insert_with(|| {
                changed = true;
                SpectrumSubscription::new(self.handle.clone(), binding.clone())
            });
            changed |= subscription.show();
            changed |= subscription.poll(now);
        }
        changed
    }
}

fn active_binding(snapshot: &AppSnapshot, id: NodeId) -> Option<&SpectrumBinding> {
    let node = snapshot.graph.graph.get_node(id).ok()?;
    if node.kind() != NodeKind::SpectrumAnalyzer {
        return None;
    }
    let binding = snapshot.signal_topics.node_spectra.get(&id)?;
    // Binding settings are confirmed engine state, not the pending candidate.
    (binding.settings.valid() && (1..=256).contains(&binding.channels)).then_some(binding)
}

struct SpectrumSubscription {
    binding: SpectrumBinding,
    signal: VisibleSignal<SpectrumTopic>,
    sequence_fence: u64,
    last_window: Option<SpectrumWindow>,
    last_delivery: Option<Instant>,
    observation: Option<SpectrumObservation>,
    status: SpectrumStatus,
    hidden_since: Option<Instant>,
}

impl SpectrumSubscription {
    fn new(handle: AppHandle, binding: SpectrumBinding) -> Self {
        Self {
            sequence_fence: binding.topic.stats().attempts,
            signal: VisibleSignal::new(handle, binding.topic.clone(), Default::default()),
            binding,
            last_window: None,
            last_delivery: None,
            observation: None,
            status: SpectrumStatus::Waiting,
            hidden_since: None,
        }
    }

    fn matches(&self, binding: &SpectrumBinding) -> bool {
        self.binding.topic == binding.topic
            && self.binding.owner_generation == binding.owner_generation
            && self.binding.settings == binding.settings
            && self.binding.channels == binding.channels
    }

    fn hide(&mut self, now: Instant) -> bool {
        if self.hidden_since.is_some() {
            return false;
        }
        self.hidden_since = Some(now);
        self.observation = None;
        self.last_delivery = None;
        self.status = SpectrumStatus::Hidden;
        true
    }

    fn show(&mut self) -> bool {
        if self.hidden_since.take().is_none() {
            return false;
        }
        // Keep short-pan demand alive, but never display the retained Latest
        // as current: only a publication attempted after reappearance counts.
        self.sequence_fence = self.binding.topic.stats().attempts;
        self.status = SpectrumStatus::Waiting;
        true
    }

    fn stale_at(&self, now: Instant) -> bool {
        self.last_delivery
            .zip(self.last_window)
            .is_some_and(|(delivered, window)| {
                freshness_timeout(self.binding.settings, window.sample_rate)
                    .is_none_or(|timeout| now.saturating_duration_since(delivered) >= timeout)
            })
    }

    fn poll(&mut self, now: Instant) -> bool {
        let before = self.status.clone();
        let published = self.poll_window(now);
        published || self.status != before
    }

    fn poll_window(&mut self, now: Instant) -> bool {
        let status = self.binding.topic.status();
        if status.lifecycle == TopicLifecycle::Closed
            || status.publisher == PublisherStatus::NoPublisher
        {
            self.signal.clear();
            self.sequence_fence = self.binding.topic.stats().attempts;
            self.observation = None;
            self.last_delivery = None;
            self.status = SpectrumStatus::Unavailable;
            return false;
        }
        if let Err(error) = self.signal.update_demand(PresentationDemand {
            window_visible: true,
            minimized: false,
            page_active: true,
            component_visible: true,
            intersects_viewport: true,
        }) {
            self.observation = None;
            self.last_delivery = None;
            self.status = SpectrumStatus::Error(format!("Spectrum subscription: {error:?}"));
            return false;
        }
        let subscriber = match self.signal.subscriber() {
            Ok(Some(subscriber)) => subscriber,
            Ok(None) => {
                self.status = SpectrumStatus::Waiting;
                return false;
            }
            Err(error) => {
                self.observation = None;
                self.last_delivery = None;
                self.status = SpectrumStatus::Error(format!("Spectrum subscription: {error}"));
                return false;
            }
        };
        let incoming = subscriber.read_latest().and_then(|block| {
            let sequence = block.meta().sequence;
            if sequence <= self.sequence_fence {
                return None;
            }
            self.sequence_fence = sequence;
            let window = *block.header();
            Some(
                if valid_window(
                    self.binding.settings,
                    self.binding.channels,
                    window,
                    block.elements(),
                    self.last_window,
                ) {
                    Ok(SpectrumObservation {
                        window,
                        magnitudes: Arc::from(block.elements()),
                    })
                } else {
                    Err(())
                },
            )
        });
        match incoming {
            Some(Ok(observation)) => {
                self.last_window = Some(observation.window);
                self.last_delivery = Some(now);
                self.observation = Some(observation);
                self.status = SpectrumStatus::Live;
                true
            }
            Some(Err(())) => {
                self.observation = None;
                self.last_delivery = None;
                self.status = SpectrumStatus::Error("Invalid spectrum window".into());
                false
            }
            None => {
                if self.stale_at(now) {
                    self.observation = None;
                    self.status = SpectrumStatus::Stale;
                }
                false
            }
        }
    }
}

fn valid_window(
    settings: SpectrumSettings,
    channels: usize,
    window: SpectrumWindow,
    magnitudes: &[f32],
    previous: Option<SpectrumWindow>,
) -> bool {
    settings.valid()
        && (1..=256).contains(&channels)
        && settings.bins_per_channel().checked_mul(channels) == Some(magnitudes.len())
        && window.fft_size == settings.fft_size
        && window.channels == channels
        && window.timeline_epoch != 0
        && window.frame_end.checked_sub(window.frame_start) == Some(settings.fft_size as u64)
        && freshness_timeout(settings, window.sample_rate).is_some()
        && magnitudes
            .iter()
            .all(|value| value.is_finite() && *value >= 0.)
        && previous.is_none_or(|previous| {
            window.timeline_epoch > previous.timeline_epoch
                || (window.timeline_epoch == previous.timeline_epoch
                    && window.sample_rate == previous.sample_rate
                    // FFT windows overlap by design; progress, not disjointness,
                    // prevents retained/replayed audio from refreshing liveness.
                    && window.frame_start > previous.frame_start
                    && window.frame_end > previous.frame_end)
        })
}

#[cfg(test)]
mod tests;
