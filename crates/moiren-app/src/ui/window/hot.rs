//! Tier H of the two-tier component model (design 17): hot surfaces.
//!
//! A hot surface is the one sanctioned way to put per-frame pixels on
//! screen: a hitbox-free canvas mounted as a sibling after its cached panel.
//! Its prepaint pumps fresh observations and — while the frame-demand
//! arbiter holds — requests the next animation frame; its paint reads the
//! geometry the sibling panel published and the live observations at paint
//! time. A cached panel replays last frame's paint between rebuilds, so any
//! dynamic content inside one freezes; content that must track the display
//! frame belongs here instead.
use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::ui::shared_frame::SharedFrame;

/// One per-window answer to "does this window need display-rate frames".
/// Sources register while they are active — meter motion, and later active
/// drags, spring animations, or pending scroll — and the control-side 33 ms
/// poll is the complement: it pumps only while nothing registers, keeping
/// observations current and re-arming the loop when fresh data moves a layer
/// (#37/#40 semantics).
#[derive(Default)]
pub(in crate::ui::window) struct FrameDemand {
    sources: std::collections::BTreeSet<&'static str>,
}

/// The meter-motion demand source registered after every pump.
pub(in crate::ui::window) const METER_MOTION: &str = "meters";

impl FrameDemand {
    pub(in crate::ui::window) fn set(&mut self, source: &'static str, active: bool) {
        if active {
            self.sources.insert(source);
        } else {
            self.sources.remove(source);
        }
    }

    pub(in crate::ui::window) fn any(&self) -> bool {
        !self.sources.is_empty()
    }
}

/// A hot surface: the declaration of what a display-rate layer pumps and
/// paints. The mechanics — the no-hitbox canvas, the pump/gate/continue
/// skeleton, the sibling placement — live once in [`HotLayerView`].
pub(in crate::ui::window) trait HotSurface: 'static + Sized {
    /// Geometry the sibling cached panel published for this frame.
    type Frame;

    /// Drain fresh observations once per painted frame, inside the shell
    /// update. Returns whether anything display-bearing changed — raw
    /// readings, status, or a freshly latched clip; ballistics motion alone
    /// does not, because animation frames (not notify) carry it.
    fn pump(&mut self, shell: &mut StudioShell, cx: &mut Context<StudioShell>) -> bool;

    /// Whether this surface's content is still moving and wants another
    /// display-rate frame. Registers with the frame-demand arbiter via the
    /// shell; window-level eligibility is the shell's, not the surface's.
    fn wants_frames(&self, shell: &StudioShell, cx: &App) -> bool;

    /// The geometry the sibling cached panel last published.
    fn frame(&self) -> &SharedFrame<Self::Frame>;

    /// Paint the live layer above the cached panel. Reads observations at
    /// paint time; aside from test traces, must not mutate shell state.
    fn paint(
        &self,
        frame: &Self::Frame,
        shell: &mut StudioShell,
        window: &mut Window,
        cx: &mut Context<StudioShell>,
    );
}

/// The host view around a [`HotSurface`]: a painting-only, hitbox-free
/// canvas that owns the animation loop for its surface. Controls under the
/// layer keep their pointer routing untouched.
pub(in crate::ui::window) struct HotLayerView<S: HotSurface> {
    shell: WeakEntity<StudioShell>,
    surface: Rc<RefCell<S>>,
}

impl<S: HotSurface> HotLayerView<S> {
    pub(in crate::ui::window) fn new(
        shell: WeakEntity<StudioShell>,
        surface: Rc<RefCell<S>>,
    ) -> Self {
        Self { shell, surface }
    }
}

impl<S: HotSurface> Render for HotLayerView<S> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pump_shell = self.shell.clone();
        let paint_shell = self.shell.clone();
        let pump_surface = self.surface.clone();
        let paint_surface = self.surface.clone();
        canvas(
            move |_, window, cx| {
                let _ = pump_shell.update(cx, |shell, cx| {
                    pump_surface.borrow_mut().pump(shell, cx);
                    // Motion, not visibility, sustains the loop: at rest the
                    // control-side poll keeps observations current and
                    // re-arms the frames once fresh data moves a layer.
                    if shell.frames_wanted(cx) {
                        window.request_animation_frame();
                    }
                });
            },
            move |_, _, window, cx| {
                let Some(shell) = paint_shell.upgrade() else {
                    return;
                };
                let surface = paint_surface.borrow();
                // A cached view replays last frame's paint operations instead
                // of re-running its paint closures, so anything dynamic the
                // cached panel painted stands still between rebuilds. Paint
                // it here, above the panel, at the frame the panel shared.
                let Some(frame) = surface.frame().read() else {
                    return;
                };
                shell.update(cx, |shell, cx| {
                    surface.paint(&frame, shell, window, cx);
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: the module glob would shadow the built-in #[test]
    // with the gpui test macro re-exported through the window module chain.
    use super::{FrameDemand, METER_MOTION};

    #[test]
    fn frame_demand_tracks_registered_sources() {
        let mut demand = FrameDemand::default();
        assert!(!demand.any(), "an idle window registers nothing");
        demand.set(METER_MOTION, true);
        assert!(demand.any());
        demand.set(METER_MOTION, true);
        assert!(demand.any(), "re-registration stays a single source");
        demand.set(METER_MOTION, false);
        assert!(!demand.any());
    }
}
