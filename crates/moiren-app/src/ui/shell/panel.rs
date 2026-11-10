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

use super::*;

/// Arc equality with a pointer fast path. Snapshots are usually identical
/// between notifications; derived `PartialEq` on `Arc<T>` walks the whole
/// value, which for a graph snapshot is the entire topology.
#[derive(Debug)]
pub(crate) struct ArcSnapshot<T>(Arc<T>);

impl<T> ArcSnapshot<T> {
    pub(crate) fn new(arc: Arc<T>) -> Self {
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
pub(crate) struct PublishRef<T>(Arc<T>);

impl<T> PublishRef<T> {
    pub(crate) fn new(arc: Arc<T>) -> Self {
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

/// A cached panel: the declaration of what a cached subtree consumes and how
/// it is built. The mechanics — key diffing, invalidation, the accessibility
/// fallback, nested-model subscriptions — live once in [`PanelHost`].
///
/// The panel value itself is per-instance state: one `StripPanel { id }` per
/// mixer strip, one unit struct per whole-page panel. Cache granularity is a
/// per-panel decision — split a page into per-item panels when one item's
/// inputs change far more often than the rest (a fader drag republishes the
/// graph at the dispatch rate; only the dragged strip's content changes).
pub(crate) trait CachedPanel: 'static + Sized {
    /// The state entity this panel projects. Most panels project the main
    /// window's [`StudioShell`]; the tray flyout projects its own root view.
    type Owner: 'static;

    /// Inputs the panel's tree consumes. Elapsed time and audio observations
    /// must never appear here: a key carrying them turns every poll into a
    /// full rebuild, and an input missing from the key freezes on screen.
    type Key: PartialEq + 'static;

    /// Capture the key from the owner. Runs on every owner notification and
    /// on every rebuild.
    fn key(&self, owner: &Self::Owner, cx: &App) -> Self::Key;

    /// Build the tree. A pure function of [`Self::Key`] and the locale:
    /// between key changes the cached subtree replays last frame's paint.
    fn render(
        &self,
        owner: &mut Self::Owner,
        window: &mut Window,
        cx: &mut Context<Self::Owner>,
    ) -> AnyElement;

    /// Per-rebuild side state: re-register meter demand, reset geometry
    /// scratch. Runs immediately before [`Self::render`]. Per-item panels
    /// must clean only their own registrations — the whole-table resets of a
    /// page panel would wipe siblings that are not rebuilding.
    fn on_rebuild(&self, owner: &mut Self::Owner) {
        let _ = (self, owner);
    }

    /// Subscribe to nested model entities (slider states, search inputs)
    /// whose notifications cannot invalidate this cached ancestor by
    /// themselves. Runs whenever the captured key or locale changed, which is
    /// exactly when the host replaces its subscription pool with the result;
    /// the pool's entities are created by the owner's update path before it
    /// notifies, so capture time always sees them. Nested notifications reach
    /// the screen through [`PanelHost::notify_nested`] instead.
    fn observe_nested(
        &self,
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
///
/// Callers splice the host entity directly (`entity.cached(refinement)`).
/// Accessibility needs no live escape hatch: gpui-fast's splice machinery
/// gates on a11y staleness itself — on activation it rebuilds until the a11y
/// nodes are recorded, then replays them from the cache's records — so
/// forcing live rebuilds at the splice only melted the whole window whenever
/// a platform client probed accessibility (every alt-tab through certain
/// setups does).
pub(crate) struct PanelHost<P: CachedPanel> {
    panel: P,
    owner: WeakEntity<P::Owner>,
    key: Option<P::Key>,
    locale: crate::i18n::UiLanguage,
    nested: Vec<Subscription>,
    _subscription: Subscription,
    /// Rebuild counter for the cache tests (design 17 hazard 6): assertions
    /// pair it with a paint-output check, never with the root's per-frame
    /// render count. Test-only so the release build carries no branch.
    #[cfg(all(test, feature = "gpui-test"))]
    rebuilds: usize,
}

impl<P: CachedPanel> PanelHost<P> {
    /// How many times this panel actually rendered (a rebuild), as opposed to
    /// replaying its cached paint.
    #[cfg(all(test, feature = "gpui-test"))]
    pub(crate) fn rebuild_count(&self) -> usize {
        self.rebuilds
    }

    pub(crate) fn new(panel: P, owner: Entity<P::Owner>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&owner, |host, owner, cx| {
            // The key capture and the nested subscriptions run here, in the
            // notification effect — never during this view's render. Reading
            // the owner during render would register it as a read dependency
            // of the cached view, and the owner is notified on every publish:
            // the framework then invalidates the cached view before the key
            // ever gets compared, and every publish rebuilds every panel.
            let key = host.panel.key(owner.read(cx), cx);
            let locale = crate::i18n::current();
            // `key` starts None, so the first notification always captures.
            // No empty-pool retry on top of that: a panel whose
            // `observe_nested` legitimately yields nothing would take this
            // branch on every notification and rebuild per notify.
            if host.key.as_ref() != Some(&key) || host.locale != locale {
                host.key = Some(key);
                host.locale = locale;
                host.nested = host.panel.observe_nested(&owner, cx);
                cx.notify();
            }
        });
        Self {
            panel,
            owner: owner.downgrade(),
            key: None,
            locale: crate::i18n::current(),
            nested: Vec::new(),
            _subscription: subscription,
            #[cfg(all(test, feature = "gpui-test"))]
            rebuilds: 0,
        }
    }

    /// A nested model entity moved (slider value, input edit): rebuild the
    /// panel so the change reaches the screen. gpui folds every notification
    /// of a frame into one render, so a sustained gesture rebuilds the panel
    /// at most once per displayed frame — the moved thumb and its readout
    /// keep pace with the display (design 22 D0). A 33 ms throttle used to
    /// sit here, from when the whole Mixer page was one panel; the per-strip
    /// split keeps a gesture's rebuild to the strip that moved, and the
    /// throttle only held touchpad and wheel gestures to ~30 fps.
    pub(crate) fn notify_nested(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
}

impl<P: CachedPanel> Render for PanelHost<P> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        // No reads of the owner here: the render's only entity traffic is
        // the writes it performs and the state the built tree reads (which
        // is written, not notified, between rebuilds). Key capture lives in
        // the owner observer above.
        #[cfg(all(test, feature = "gpui-test"))]
        {
            self.rebuilds += 1;
        }
        let panel = &self.panel;
        owner.update(cx, |owner, cx| {
            panel.on_rebuild(owner);
            panel.render(owner, window, cx)
        })
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: the module glob would shadow the built-in #[test]
    // with the gpui test macro re-exported through the window module chain.
    use std::sync::Arc;

    use super::{ArcSnapshot, PublishRef};

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
}
