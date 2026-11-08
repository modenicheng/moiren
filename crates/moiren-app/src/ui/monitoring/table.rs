//! The shared per-node demand-driven subscription table (issue #124).
//!
//! [`NodeMeters`](super::NodeMeters) and
//! [`NodeSpectra`](super::spectrum::NodeSpectra) keep the same lifecycle
//! around their payload subscriptions: the session generation fences a
//! wholesale reset, visibility is remembered with a grace window so
//! per-frame flapping does not thrash subscriptions, only the running
//! session may hold them, and a demand change (a new topic, a new channel
//! width) rebuilds the subscription in place. [`SubscriptionTable`] is that
//! lifecycle, written once; the two tables specialize it with their payload
//! type, their per-tick context (display preferences, projection
//! parameters), and their payload-specific wake rule (folded into
//! [`TopicSubscription::poll`]).
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use moiren_core::graph::NodeId;

use crate::service::{AppHandle, AppSnapshot, SessionGeneration, SessionPhase};

/// How long a subscription outlives its node leaving the visible set,
/// instead of flapping with per-frame visibility: the graph canvas pans and
/// zooms, subscriptions must not follow.
pub(crate) const SUBSCRIPTION_GRACE: Duration = Duration::from_millis(750);

/// A payload subscription the table manages. The table owns the lifecycle;
/// the payload owns everything else — its readings, its status, and its
/// wake rule, folded into [`TopicSubscription::poll`].
pub(crate) trait TopicSubscription {
    /// The per-node demand key: what the subscription is built from, and
    /// rebuilt in place for when it changes (a level topic and its channel
    /// width; a spectrum topic).
    type Topic;

    /// Table-wide per-tick context the payload needs: display preferences
    /// for the meters, projection parameters for the spectra.
    type Context;

    /// Build a fresh subscription for a demand key.
    fn build(handle: AppHandle, topic: &Self::Topic, context: &Self::Context) -> Self;

    /// Whether the subscription currently reads this demand key.
    fn reads(&self, topic: &Self::Topic) -> bool;

    /// Apply the per-tick context before polling.
    fn prepare(&mut self, context: &Self::Context);

    /// Poll the payload and report its full wake rule: whether the
    /// interface must repaint (a reading change, a status transition —
    /// whatever the payload defines; a publication alone never counts, a
    /// running stream keeps publishing windows whose readings may not
    /// move).
    fn poll(&mut self, now: Instant) -> bool;
}

/// The lifecycle shared by the per-node meter and spectrum tables.
pub(crate) struct SubscriptionTable<S> {
    handle: AppHandle,
    generation: Option<SessionGeneration>,
    subscriptions: BTreeMap<NodeId, S>,
    /// Nodes seen visible recently; subscriptions outlive the visible set
    /// by [`SUBSCRIPTION_GRACE`] instead of flapping with per-frame
    /// visibility.
    seen: BTreeMap<NodeId, Instant>,
    /// Nodes whose subscription the last [`SubscriptionTable::update_at`]
    /// rebuilt in place. Payload latches that must reset with a rebuild —
    /// the meters' clip hold — drain this after the pass. Rebuilds are
    /// rare, so the scratch stays empty and draining is free.
    rebuilt: Vec<NodeId>,
}

impl<S> SubscriptionTable<S> {
    pub(crate) fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            generation: None,
            subscriptions: BTreeMap::new(),
            seen: BTreeMap::new(),
            rebuilt: Vec::new(),
        }
    }
}

impl<S: TopicSubscription> SubscriptionTable<S> {
    /// Reconcile demand with the session: reset on a new generation,
    /// register visibility, drop stale/grace-expired/non-running demand,
    /// and poll every live subscription. `resolve` maps a node to its
    /// current demand key — a node without one holds no subscription.
    /// Returns whether anything the interface must wake for changed:
    /// subscription-set changes always count, and each subscription's
    /// [`TopicSubscription::poll`] decides its payload's wake rule.
    pub(crate) fn update_at(
        &mut self,
        snapshot: &AppSnapshot,
        visible: impl IntoIterator<Item = NodeId>,
        context: &S::Context,
        resolve: impl Fn(NodeId) -> Option<S::Topic>,
        now: Instant,
    ) -> bool {
        if self.generation != Some(snapshot.generation) {
            self.subscriptions.clear();
            self.seen.clear();
            self.generation = Some(snapshot.generation);
        }
        // Subscriptions belong to the running session: candidate edits and
        // the brief swap window of an automatic rebuild must never blank
        // them.
        let active = snapshot.phase == SessionPhase::Running;
        for id in visible {
            self.seen.insert(id, now);
        }
        self.seen.retain(|id, seen| {
            snapshot.graph.node(*id).is_some()
                && now.saturating_duration_since(*seen) < SUBSCRIPTION_GRACE
        });
        let before = self.subscriptions.len();
        self.subscriptions
            .retain(|id, _| active && self.seen.contains_key(id) && resolve(*id).is_some());
        let mut changed = before != self.subscriptions.len();
        if !active {
            return changed;
        }
        self.rebuilt.clear();
        // Borrow the seen table for the whole pass: the body only touches
        // the sibling maps, and cloning the table here would allocate on
        // every pump tick for every visible node (HC-14).
        for id in self.seen.keys() {
            let Some(topic) = resolve(*id) else {
                continue;
            };
            let subscription = self
                .subscriptions
                .entry(*id)
                .or_insert_with(|| S::build(self.handle.clone(), &topic, context));
            if !subscription.reads(&topic) {
                *subscription = S::build(self.handle.clone(), &topic, context);
                self.rebuilt.push(*id);
            }
            subscription.prepare(context);
            changed |= subscription.poll(now);
        }
        changed
    }

    /// The live subscriptions with their nodes.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&NodeId, &S)> {
        self.subscriptions.iter()
    }

    /// One node's live subscription, if any.
    pub(crate) fn get(&self, id: NodeId) -> Option<&S> {
        self.subscriptions.get(&id)
    }

    /// Every live subscription.
    pub(crate) fn values(&self) -> impl Iterator<Item = &S> {
        self.subscriptions.values()
    }

    /// The nodes whose subscriptions the last pass rebuilt in place.
    pub(crate) fn take_rebuilt(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.rebuilt)
    }
}
