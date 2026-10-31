//! Tier C of the two-tier component model (design 17): cached panels.
//!
//! A panel is a subtree whose contents change only when its semantic key
//! changes. Between key changes gpui-fast replays last frame's
//! layout/prepaint/paint without re-running the panel's render closure, so
//! the root element rebuild that every drawn frame performs splices the
//! cached subtree instead of re-rendering it. Dynamic content — anything
//! that must track the display frame — never belongs in a panel; it belongs
//! in a hot surface.
//!
//! Panels are siblings, never nested: a cached parent forces its nested
//! caches fresh on every rebuild.
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;

/// Arc equality with a pointer fast path. Snapshots are usually identical
/// between notifications; derived `PartialEq` on `Arc<T>` walks the whole
/// value, which for a graph snapshot is the entire topology.
#[derive(Debug)]
pub(in crate::ui::window) struct ArcSnapshot<T>(Arc<T>);

impl<T> ArcSnapshot<T> {
    pub(in crate::ui::window) fn new(arc: Arc<T>) -> Self {
        Self(arc)
    }
}

impl<T> Clone for ArcSnapshot<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: PartialEq> PartialEq for ArcSnapshot<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

/// Publish identity for snapshots without value equality
/// ([`crate::service::AppSnapshot`]). The service dedups publishes by
/// content, so a fresh Arc always carries a real change; pointer identity is
/// both sound and cheaper than any compare.
#[derive(Debug)]
pub(in crate::ui::window) struct PublishRef<T>(Arc<T>);

impl<T> PublishRef<T> {
    pub(in crate::ui::window) fn new(arc: Arc<T>) -> Self {
        Self(arc)
    }
}

impl<T> Clone for PublishRef<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> PartialEq for PublishRef<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// How far apart nested-model rebuilds may land. Matches the fader dispatch
/// interval (`mixer::model::FADER_DISPATCH_INTERVAL`): during a drag the
/// service confirmations that carry the dB readouts arrive at the same pace,
/// so the panel never rebuilds faster than its inputs actually change.
pub(in crate::ui::window) const NESTED_REBUILD_INTERVAL: Duration = Duration::from_millis(33);

/// What [`NestedGate::notify`] asks the host to do with one nested-model
/// notification (a slider drag tick, an input edit).
#[derive(Debug, PartialEq, Eq)]
pub(in crate::ui::window) enum NestedAction {
    /// Rebuild the panel now: the first notification of a burst must reach
    /// the screen without waiting for the interval.
    Rebuild,
    /// Suppress now and arm the single trailing rebuild.
    Arm,
    /// Suppress; a trailing rebuild is already armed.
    Ignore,
}

/// Leading-edge throttle for nested-model notifications. A drag emits a
/// Change per pointer move — far above the display rate — and every one of
/// them costs a full panel rebuild, so sustained drags starve the frame
/// budget and stutter. The first notification of a burst rebuilds at once;
/// later ones coalesce into one trailing rebuild, so a paused drag still
/// paints its last suppressed value before release.
#[derive(Default)]
pub(in crate::ui::window) struct NestedGate {
    last_rebuild: Option<Instant>,
    armed_at: Option<Instant>,
}

impl NestedGate {
    fn notify(&mut self, now: Instant) -> NestedAction {
        if self
            .last_rebuild
            .is_none_or(|last| now.duration_since(last) >= NESTED_REBUILD_INTERVAL)
        {
            self.last_rebuild = Some(now);
            return NestedAction::Rebuild;
        }
        if self.armed_at.is_none() {
            self.armed_at = Some(now);
            return NestedAction::Arm;
        }
        NestedAction::Ignore
    }

    /// The armed trailing timer fired INTERVAL after arming. Rebuilds unless
    /// a nested rebuild already resumed after arming (its burst carries the
    /// fresh values); consumes the armed state either way.
    fn fire(&mut self, now: Instant) -> bool {
        let Some(armed_at) = self.armed_at.take() else {
            return false;
        };
        if self.last_rebuild.is_some_and(|last| last > armed_at) {
            return false;
        }
        self.last_rebuild = Some(now);
        true
    }
}

/// A cached panel: the declaration of what a cached subtree consumes and how
/// it is built. The mechanics — key diffing, invalidation, the accessibility
/// fallback, nested-model subscriptions — live once in [`PanelHost`].
pub(in crate::ui::window) trait CachedPanel: 'static + Sized {
    /// The state entity this panel projects. Most panels project the main
    /// window's [`StudioShell`]; the tray flyout projects its own root view.
    type Owner: 'static;

    /// Inputs the panel's tree consumes. Elapsed time and audio observations
    /// must never appear here: a key carrying them turns every poll into a
    /// full rebuild, and an input missing from the key freezes on screen.
    type Key: PartialEq + 'static;

    /// Capture the key from the owner. Runs on every owner notification and
    /// on every rebuild.
    fn key(owner: &Self::Owner, cx: &App) -> Self::Key;

    /// Build the tree. A pure function of [`Self::Key`] and the locale:
    /// between key changes the cached subtree replays last frame's paint.
    fn render(
        owner: &mut Self::Owner,
        window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement;

    /// Per-rebuild side state: re-register meter demand, reset geometry
    /// scratch. Runs immediately before [`Self::render`].
    fn on_rebuild(owner: &mut Self::Owner) {
        let _ = owner;
    }

    /// Subscribe to nested model entities (slider states, search inputs)
    /// whose notifications cannot invalidate this cached ancestor by
    /// themselves. Runs on every rebuild; the host replaces its subscription
    /// pool with the result.
    fn observe_nested(
        _owner: &Entity<Self::Owner>,
        _cx: &mut Context<PanelHost<Self>>,
    ) -> Vec<Subscription> {
        Vec::new()
    }
}

/// The host entity around a [`CachedPanel`]: it observes the owner, rebuilds
/// the panel only when the panel's key (or the active locale) changed, and
/// owns the mechanics every panel shares.
///
/// The locale rides outside the key on purpose: every panel renders `t!`
/// strings, so a language switch must rebuild all of them regardless of what
/// each key happens to capture.
pub(in crate::ui::window) struct PanelHost<P: CachedPanel> {
    owner: WeakEntity<P::Owner>,
    key: Option<P::Key>,
    locale: crate::i18n::UiLanguage,
    nested: Vec<Subscription>,
    nested_gate: NestedGate,
    _subscription: Subscription,
}

impl<P: CachedPanel> PanelHost<P> {
    pub(in crate::ui::window) fn new(owner: Entity<P::Owner>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&owner, |host, owner, cx| {
            let key = P::key(owner.read(cx), cx);
            let locale = crate::i18n::current();
            if host.key.as_ref() != Some(&key) || host.locale != locale {
                host.key = Some(key);
                host.locale = locale;
                cx.notify();
            }
        });
        Self {
            owner: owner.downgrade(),
            key: None,
            locale: crate::i18n::current(),
            nested: Vec::new(),
            nested_gate: NestedGate::default(),
            _subscription: subscription,
        }
    }

    /// A nested model entity moved (slider drag tick, input edit): schedule
    /// the panel rebuild that carries it to the screen. The throttle keeps a
    /// sustained drag from paying a full panel rebuild per pointer move; the
    /// first notification of a burst still rebuilds immediately, and release
    /// paths repaint through their own publishes regardless.
    pub(in crate::ui::window) fn notify_nested(&mut self, cx: &mut Context<Self>) {
        match self.nested_gate.notify(Instant::now()) {
            NestedAction::Rebuild => cx.notify(),
            NestedAction::Arm => {
                cx.spawn(async move |host, cx| {
                    cx.background_executor()
                        .timer(NESTED_REBUILD_INTERVAL)
                        .await;
                    host.update(cx, |host, cx| {
                        if host.nested_gate.fire(Instant::now()) {
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            NestedAction::Ignore => {}
        }
    }
}

impl<P: CachedPanel> Render for PanelHost<P> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        // Self-healing: a rebuild always records the inputs it rendered with,
        // so a missed notification cannot pin a stale key.
        self.key = Some(P::key(owner.read(cx), cx));
        self.locale = crate::i18n::current();
        self.nested = P::observe_nested(&owner, cx);
        owner.update(cx, |owner, cx| {
            P::on_rebuild(owner);
            P::render(owner, window, cx)
        })
    }
}

/// Splice a cached panel entity, or render it live while accessibility is
/// active. GPUI's cache replays paint/layout but not the per-frame
/// accessibility tree, so an active accessibility client needs the entity's
/// subtree rebuilt into each frame's tree.
pub(in crate::ui::window) fn cached_or_live<E: 'static>(
    entity: Entity<E>,
    window: &Window,
    cached: impl FnOnce(Entity<E>) -> AnyElement,
    live: impl FnOnce(Entity<E>) -> AnyElement,
) -> AnyElement {
    if window.is_a11y_active() {
        live(entity)
    } else {
        cached(entity)
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: the module glob would shadow the built-in #[test]
    // with the gpui test macro re-exported through the window module chain.
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::{ArcSnapshot, NestedAction, NestedGate, PublishRef};

    #[test]
    fn arc_snapshot_prefers_pointer_identity_before_value_equality() {
        let arc: Arc<(u32, String)> = Arc::new((1, "one".into()));
        let a = ArcSnapshot::new(Arc::clone(&arc));
        let b = ArcSnapshot::new(Arc::clone(&arc));
        assert_eq!(a, b, "the same Arc is equal without walking the value");

        let twin = Arc::new((1, "one".into()));
        assert_eq!(a, ArcSnapshot::new(twin), "equal contents stay equal");

        let other = Arc::new((2, "two".into()));
        assert_ne!(a, ArcSnapshot::new(other), "different contents differ");
    }

    #[test]
    fn publish_ref_compares_publishes_by_identity_only() {
        let arc: Arc<u64> = Arc::new(7);
        let a = PublishRef::new(Arc::clone(&arc));
        assert_eq!(a, PublishRef::new(Arc::clone(&arc)));
        // The service dedups publishes by content, so equal contents in two
        // different publishes should not occur; identity is the whole truth.
        assert_ne!(a, PublishRef::new(Arc::new(7)));
    }

    #[test]
    fn nested_gate_rebuilds_the_first_notification_of_a_burst() {
        let mut gate = NestedGate::default();
        let base = Instant::now();
        assert_eq!(gate.notify(base), NestedAction::Rebuild);
        assert_eq!(
            gate.notify(base + Duration::from_millis(10)),
            NestedAction::Arm
        );
        assert_eq!(
            gate.notify(base + Duration::from_millis(20)),
            NestedAction::Ignore,
            "a trailing rebuild is already armed"
        );
        // The interval elapsed since the last rebuild: the burst's next
        // notification leads again.
        assert_eq!(
            gate.notify(base + Duration::from_millis(33)),
            NestedAction::Rebuild
        );
    }

    #[test]
    fn nested_gate_trailing_fire_rebuilds_once_and_yields_to_living_bursts() {
        let mut gate = NestedGate::default();
        let base = Instant::now();
        gate.notify(base);
        gate.notify(base + Duration::from_millis(5));
        // The timer fires a full interval after arming; the last rebuild is
        // then older than the interval, so the suppressed tail must paint.
        assert!(gate.fire(base + Duration::from_millis(38)));
        assert!(
            !gate.fire(base + Duration::from_millis(39)),
            "the trailing edge is single-shot"
        );

        // A burst that resumes on its own between arming and the fire makes
        // the trailing rebuild redundant: its leading rebuilds carry the
        // values, and the stale timer must not spend another rebuild.
        gate.notify(base + Duration::from_millis(100));
        gate.notify(base + Duration::from_millis(105));
        gate.notify(base + Duration::from_millis(140));
        assert!(
            !gate.fire(base + Duration::from_millis(145)),
            "resumed rebuilds supersede the armed trailing edge"
        );
    }
}
