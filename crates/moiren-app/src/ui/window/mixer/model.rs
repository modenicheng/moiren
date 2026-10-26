use super::*;
use crate::service::GraphSnapshot;
use crate::ui::widgets::fader::{Commit, FaderScale, SliderBehavior};
use rust_i18n::t;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(in crate::ui::window) struct MixerPresentation {
    pub order: Vec<NodeId>,
    pub faders: BTreeMap<NodeId, SliderSlot>,
    pub sends: BTreeMap<EdgeId, SliderSlot>,
    pub send_inputs: BTreeMap<EdgeId, Entity<component::input::InputState>>,
    pub editing_send: Option<EdgeId>,
    pub inputs_collapsed: bool,
    pub sends_collapsed: bool,
    synced_revision: Option<u64>,
    #[cfg(feature = "gpui-test")]
    pub trace: std::cell::RefCell<super::performance::MeterTrace>,
}

/// Per-slider bookkeeping kept next to its entity: the dragging flag lets the
/// snapshot write-back in [`sync_slider_range`] stand down while a drag owns
/// the thumb, and the behavior carries the shared wheel/double-click actions.
pub(in crate::ui::window) struct SliderSlot {
    pub state: Entity<base::slider::SliderState>,
    pub dragging: Rc<Cell<bool>>,
    pub behavior: SliderBehavior,
}

/// Drag Changes reach the service at most once per interval; the release
/// passes its final value unless a wheel/double-click commit already
/// superseded it. Pure and clock-injected so tests drive it without gpui,
/// and so no timer is needed: a stashed pending value flushes on the next
/// Change past the interval, or at latest on release.
const FADER_DISPATCH_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Default)]
struct DispatchGate {
    last_sent: Option<Instant>,
    pending: Option<f32>,
}

impl DispatchGate {
    /// One drag Change: send it now when the interval elapsed since the last
    /// dispatch (or no dispatch happened yet), otherwise supersede whatever
    /// pending value is stashed.
    fn change(&mut self, value: f32, now: Instant) -> Option<f32> {
        match self.last_sent {
            Some(sent) if now.duration_since(sent) < FADER_DISPATCH_INTERVAL => {
                self.pending = Some(value);
                None
            }
            _ => {
                self.last_sent = Some(now);
                self.pending = None;
                Some(value)
            }
        }
    }

    /// Drag finished: the event's final value is dispatched unconditionally
    /// by the caller; forget the timer and any stashed intermediate so the
    /// next drag's first Change sends immediately.
    fn release(&mut self) {
        self.last_sent = None;
        self.pending = None;
    }
}

impl MixerPresentation {
    /// Fixed strip and dock widths; strips are not user-resizable.
    pub(in crate::ui::window) const STRIP_WIDTH: f32 = 160.;
    pub(in crate::ui::window) const DOCK_WIDTH: f32 = 360.;
}

/// dB readout for send gains: −∞ collapses to −60 dB at the slider's quiet
/// end, +24 dB caps the boost end.
fn gain_db(gain: f64) -> f32 {
    if gain <= 0. {
        -60.
    } else {
        (20. * gain.log10()).clamp(-60., 24.) as f32
    }
}

fn linear_gain(value: f32) -> f64 {
    if value <= -60. {
        0.
    } else {
        10f64.powf(f64::from(value) / 20.)
    }
}

/// Builds the real-time edit for a slider's new value from the current
/// snapshot; None when the target vanished since the slider was created.
pub(in crate::ui::window) type SliderEdit = Box<dyn Fn(f32, &GraphSnapshot) -> Option<AppRequest>>;

/// Loudest gain a [`MixerState`] accepts. The fader scale tops out at +12 dB,
/// well below this, so clamping only guards edit validity.
const FADER_MAX_GAIN: f64 = 16.;

/// One wheel notch on a node fader: a multiplicative ±1 dB step in the
/// perceptual value space (`1 - FaderScale::position(gain)`). Stepping below
/// the −72 dB floor snaps to exact digital zero, and stepping up from exact
/// silence un-silences one step above the floor so the wheel can always
/// leave −∞ (0 gain cannot be multiplied into something audible).
fn fader_step(value: f32, dir: i32) -> f32 {
    if dir == 0 {
        return value;
    }
    let floor = 10f64.powf(-72. / 20.);
    let gain = FaderScale::gain(1. - value);
    let stepped = if gain <= 0. && dir > 0 {
        floor * 10f64.powf(f64::from(dir) / 20.)
    } else {
        gain * 10f64.powf(f64::from(dir) / 20.)
    };
    // f32 round-trips through FaderScale carry a few 1e-5 dB of slack, so
    // only fall a meaningful 0.001 dB below the floor: the floor itself must
    // stay finite across a wheel round trip.
    let gain = if stepped * 10f64.powf(0.001 / 20.) < floor {
        0.
    } else {
        stepped.min(FADER_MAX_GAIN)
    };
    1. - FaderScale::position(gain)
}

/// Wheel + double-click commit shared by every mixer slider: write the value
/// into the slider state (`set_value` emits no `Change`, so this never loops
/// through the drag path) and dispatch the edit immediately, bypassing the
/// drag coalescer.
pub(in crate::ui::window) fn commit_closure(
    ui: &Entity<UiState>,
    edit: &Rc<SliderEdit>,
    slider: &Entity<base::slider::SliderState>,
) -> Commit {
    let ui = ui.clone();
    let edit = edit.clone();
    let slider = slider.clone();
    Rc::new(move |value, window, cx| {
        slider.update(cx, |state, cx| state.set_value(value, window, cx));
        dispatch_edit(&ui, &edit, value, cx);
    })
}

/// Node fader behavior (main mixer, inspector channel strip, tray flyout):
/// ±1 dB per notch, double click restores unity. Slots built by
/// [`sync_faders`] carry it, so every surface renders the same gestures.
fn fader_behavior(
    ui: Entity<UiState>,
    edit: Rc<SliderEdit>,
    slider: Entity<base::slider::SliderState>,
) -> SliderBehavior {
    SliderBehavior {
        step: Rc::new(fader_step),
        reset: 1. - FaderScale::position(1.),
        commit: commit_closure(&ui, &edit, &slider),
    }
}

/// Send faders live in dB space: one notch is 1 dB, double click restores
/// unity (0 dB, the value `0.` in this space).
fn send_behavior(
    ui: Entity<UiState>,
    edit: Rc<SliderEdit>,
    slider: Entity<base::slider::SliderState>,
) -> SliderBehavior {
    SliderBehavior {
        step: Rc::new(|value, dir| (value + dir as f32).clamp(-60., 24.)),
        reset: 0.,
        commit: commit_closure(&ui, &edit, &slider),
    }
}

/// Applies one slider edit against the live snapshot; a target that vanished
/// since the slider was created simply skips.
fn dispatch_edit(ui: &Entity<UiState>, edit: &Rc<SliderEdit>, value: f32, cx: &mut App) {
    ui.update(cx, |state, _| {
        if let Some(request) = edit(value, &state.snapshot.graph) {
            state.dispatch(request);
        }
    });
}

/// Fader rows for every node, shared verbatim by the main mixer and the tray
/// flyout so both edit the same perceptual value space `[0..1]`: value
/// `1 - FaderScale::position(gain)` (1 = track top), and an edit maps back
/// through `FaderScale::gain`. The slot's subscription coalesces drag
/// Changes and dispatches the release's final value.
pub(in crate::ui::window) fn fader_rows(
    snapshot: &GraphSnapshot,
) -> Vec<(NodeId, f32, SliderEdit)> {
    snapshot
        .nodes
        .iter()
        .map(|node| {
            let id = node.id;
            (
                id,
                1. - FaderScale::position(node.mixer.gain),
                Box::new(move |value: f32, graph: &GraphSnapshot| {
                    graph.node(id).map(|node| {
                        AppRequest::EditGraph(GraphEdit::SetMixer {
                            node: id,
                            state: MixerState {
                                gain: FaderScale::gain(1. - value),
                                ..node.mixer
                            },
                        })
                    })
                }) as SliderEdit,
            )
        })
        .collect()
}

/// Creates and value-syncs fader sliders over [`fader_rows`] entries in the
/// shared perceptual range `[0..1]`. Callers pass the rows so a view can
/// filter them first: the tray flyout only shows audio-bearing strips.
pub(in crate::ui::window) fn sync_faders<T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    state: &Entity<UiState>,
    faders: &mut BTreeMap<NodeId, SliderSlot>,
    rows: Vec<(NodeId, f32, SliderEdit)>,
) {
    sync_slider_range(
        window,
        cx,
        state,
        faders,
        rows,
        [0., 1., 0.0001],
        |_, ui, edit, slider| fader_behavior(ui, edit, slider),
    );
}

/// Slider construction and value sync for the send (edge) column in dB space;
/// faders use [`sync_faders`]'s perceptual space instead. Drag Changes are
/// coalesced and the release's final value dispatches unconditionally.
fn sync_sliders<K: Ord + Clone, T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    state: &Entity<UiState>,
    sliders: &mut BTreeMap<K, SliderSlot>,
    rows: Vec<(K, f32, SliderEdit)>,
) {
    sync_slider_range(
        window,
        cx,
        state,
        sliders,
        rows,
        [-60., 24., 0.1],
        |_, ui, edit, slider| send_behavior(ui, edit, slider),
    );
}

/// Slider construction and value sync shared by every mixer-style slider
/// (channel faders, sends, compressor parameters): per-slider slot with the
/// dragging flag and wheel/double-click behavior, drag Changes coalesced
/// through [`DispatchGate`], the release's final value dispatched, and the
/// snapshot write-back suppressed while a drag owns the thumb. `behavior`
/// receives the row key so one call can serve keyed parameter families.
pub(in crate::ui::window) fn sync_slider_range<K: Ord + Clone, T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    state: &Entity<UiState>,
    sliders: &mut BTreeMap<K, SliderSlot>,
    rows: Vec<(K, f32, SliderEdit)>,
    range: [f32; 3],
    behavior: impl Fn(
        &K,
        Entity<UiState>,
        Rc<SliderEdit>,
        Entity<base::slider::SliderState>,
    ) -> SliderBehavior,
) {
    for (id, value, edit) in rows {
        let slot = sliders.entry(id.clone()).or_insert_with(|| {
            let edit = Rc::new(edit);
            let dragging = Rc::new(Cell::new(false));
            let slider = cx.new(|_| {
                base::slider::SliderState::new()
                    .min(range[0])
                    .max(range[1])
                    .step(range[2])
                    .default_value(value)
            });
            let ui = state.clone();
            let drag_flag = dragging.clone();
            let drag_edit = edit.clone();
            let mut gate = DispatchGate::default();
            cx.subscribe(
                &slider,
                move |_, slider, event: &base::slider::SliderEvent, cx| {
                    match event {
                        base::slider::SliderEvent::Change(value) => {
                            drag_flag.set(true);
                            if let Some(send) = gate.change(value.end(), Instant::now()) {
                                dispatch_edit(&ui, &drag_edit, send, cx);
                            }
                        }
                        base::slider::SliderEvent::Release(value) => {
                            drag_flag.set(false);
                            gate.release();
                            let value = value.end();
                            // The release always lands — unless a wheel or
                            // double-click commit already moved the slider
                            // past it: those handlers paint before the
                            // slider's release listener, so on the same
                            // mouse-up a stale track value would otherwise
                            // overwrite the committed one.
                            if slider.read(cx).value().end() == value {
                                dispatch_edit(&ui, &drag_edit, value, cx);
                            }
                        }
                    }
                    cx.notify();
                },
            )
            .detach();
            SliderSlot {
                state: slider.clone(),
                dragging,
                behavior: behavior(&id, state.clone(), edit, slider),
            }
        });
        // A live drag owns the thumb: snapshot write-backs (about one
        // dispatch interval stale) must not snap it away from the pointer.
        // Normal revision sync resumes on release and any one stale
        // write-back self-corrects on the next revision.
        if !slot.dragging.get() && slot.state.read(cx).value().end() != value {
            slot.state
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }
}

impl StudioShell {
    pub(in crate::ui::window) fn sync_mixer_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = self.state.read(cx).snapshot.graph.candidate_revision;
        if self.mixer.synced_revision == Some(revision) {
            return;
        }
        self.mixer.synced_revision = Some(revision);
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.mixer.order.retain(|id| graph.node(*id).is_some());
        self.mixer.faders.retain(|id, _| graph.node(*id).is_some());
        self.mixer
            .sends
            .retain(|id, _| graph.graph.get_edge(*id).is_ok());
        self.mixer
            .send_inputs
            .retain(|id, _| graph.graph.get_edge(*id).is_ok());
        if self
            .mixer
            .editing_send
            .is_some_and(|id| graph.graph.get_edge(id).is_err())
        {
            self.mixer.editing_send = None;
        }
        for node in &graph.nodes {
            if !self.mixer.order.contains(&node.id) {
                self.mixer.order.push(node.id);
            }
        }
        sync_faders(
            window,
            cx,
            &self.state,
            &mut self.mixer.faders,
            fader_rows(&graph),
        );
        for edge in graph.graph.edges() {
            let id = edge.id();
            let input = self.mixer.send_inputs.entry(id).or_insert_with(|| {
                let input = cx.new(|cx| component::input::InputState::new(window, cx));
                cx.subscribe(
                    &input,
                    move |shell, input, event: &component::input::InputEvent, cx| {
                        if !matches!(
                            event,
                            component::input::InputEvent::PressEnter { .. }
                                | component::input::InputEvent::Blur
                        ) || shell.mixer.editing_send != Some(id)
                        {
                            return;
                        }
                        shell.mixer.editing_send = None;
                        let text = input.read(cx).value();
                        let gain = if matches!(text.trim(), "-inf" | "−∞" | "-∞") {
                            Some(0.)
                        } else {
                            text.parse::<f64>()
                                .ok()
                                .filter(|db| db.is_finite() && (-120. ..=24.).contains(db))
                                .map(|db| 10f64.powf(db / 20.))
                        };
                        if let Some(gain) = gain {
                            shell.state.update(cx, |state, _| {
                                if let Ok(edge) = state.snapshot.graph.graph.get_edge(id) {
                                    state.dispatch(AppRequest::EditGraph(GraphEdit::SetSend {
                                        edge: id,
                                        params: SendParams {
                                            gain,
                                            ..*edge.params()
                                        },
                                    }));
                                }
                            });
                        }
                        cx.notify();
                    },
                )
                .detach();
                input
            });
            if self.mixer.editing_send != Some(id) {
                let text = if edge.params().gain <= 0. {
                    "-inf".into()
                } else {
                    format!("{:.1}", 20. * edge.params().gain.log10())
                };
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
        let sends = graph
            .graph
            .edges()
            .iter()
            .map(|edge| {
                let id = edge.id();
                (
                    id,
                    gain_db(edge.params().gain),
                    Box::new(move |value: f32, graph: &GraphSnapshot| {
                        graph.graph.get_edge(id).ok().map(|edge| {
                            AppRequest::EditGraph(GraphEdit::SetSend {
                                edge: id,
                                params: SendParams {
                                    gain: linear_gain(value),
                                    ..*edge.params()
                                },
                            })
                        })
                    }) as SliderEdit,
                )
            })
            .collect();
        sync_sliders(window, cx, &self.state, &mut self.mixer.sends, sends);
    }
}

pub(super) fn reorder(order: &mut Vec<NodeId>, dragged: NodeId, target: NodeId) {
    if dragged == target || !order.contains(&dragged) || !order.contains(&target) {
        return;
    }
    order.retain(|id| *id != dragged);
    let index = order
        .iter()
        .position(|id| *id == target)
        .expect("existing reorder target");
    order.insert(index, dragged);
}

pub(super) struct StripDrag;
impl Render for StripDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(12.))
            .bg(theme::SURFACE_3)
            .text_color(theme::TEXT)
            .child(t!("Move channel"))
    }
}

#[cfg(test)]
mod tests {
    use super::{DispatchGate, Duration, FaderScale, Instant, fader_step};

    fn millis(base: Instant, offset: u64) -> Instant {
        base + Duration::from_millis(offset)
    }

    #[test]
    fn first_change_dispatches_immediately() {
        let mut gate = DispatchGate::default();
        let base = Instant::now();
        assert_eq!(gate.change(0.5, base), Some(0.5));
    }

    #[test]
    fn changes_within_the_interval_are_coalesced_and_superseded() {
        let mut gate = DispatchGate::default();
        let base = Instant::now();
        assert_eq!(gate.change(0.5, base), Some(0.5));
        assert_eq!(gate.change(0.6, millis(base, 10)), None);
        // A later Change within the interval replaces the stashed value.
        assert_eq!(gate.change(0.7, millis(base, 20)), None);
        assert_eq!(gate.change(0.8, millis(base, 32)), None);
    }

    #[test]
    fn change_after_the_interval_dispatches_the_latest_value() {
        let mut gate = DispatchGate::default();
        let base = Instant::now();
        assert_eq!(gate.change(0.5, base), Some(0.5));
        assert_eq!(gate.change(0.6, millis(base, 10)), None);
        // The stashed intermediate is superseded by the newer value.
        assert_eq!(gate.change(0.7, millis(base, 33)), Some(0.7));
        assert_eq!(gate.change(0.8, millis(base, 40)), None);
    }

    #[test]
    fn release_clears_the_gate_so_the_next_drag_resumes_immediately() {
        let mut gate = DispatchGate::default();
        let base = Instant::now();
        assert_eq!(gate.change(0.5, base), Some(0.5));
        assert_eq!(gate.change(0.6, millis(base, 5)), None);
        gate.release();
        assert_eq!(gate.pending, None);
        // Even an instant next drag sends its first Change.
        assert_eq!(gate.change(0.1, millis(base, 6)), Some(0.1));
    }

    #[test]
    fn fader_step_moves_one_db_and_round_trips_symmetrically() {
        let at_db = |db: f64| 1. - FaderScale::position(10f64.powf(db / 20.));
        let db_of = |value: f32| 20. * FaderScale::gain(1. - value).log10();
        // Up then down from −6 dB returns to the same slider value.
        let start = at_db(-6.);
        let up = fader_step(start, 1);
        assert!((db_of(up) + 5.).abs() < 0.001, "got {} dB", db_of(up));
        assert!((fader_step(up, -1) - start).abs() < 0.00001);
        let down = fader_step(start, -1);
        assert!((db_of(down) + 7.).abs() < 0.001, "got {} dB", db_of(down));
    }

    #[test]
    fn fader_step_unsilences_into_the_quiet_tail_and_snaps_back_to_silence() {
        // Silence cannot be multiplied into something audible: the first
        // up-step lands one dB above the −72 dB floor, inside the final
        // finite segment between the −72 and −60 dB ticks.
        let unsilenced = fader_step(0., 1);
        let db = 20. * FaderScale::gain(1. - unsilenced).log10();
        assert!(db > -72. && db <= -60., "got {db} dB");
        // Stepping down again returns to the floor (still finite); one more
        // step falls below it and snaps to exact digital zero.
        let floor = fader_step(unsilenced, -1);
        assert!(
            (FaderScale::gain(1. - floor) - 10f64.powf(-72. / 20.)).abs() < 1e-9,
            "expected the −72 dB floor"
        );
        assert_eq!(FaderScale::gain(1. - fader_step(floor, -1)), 0.);
        // Silence is sticky downwards.
        assert_eq!(fader_step(0., -1), 0.);
    }

    #[test]
    fn fader_step_clamps_at_the_loud_ceiling() {
        // Stepping above the +12 dB scale top clamps back to the top value
        // through FaderScale::position, and the stepped gain stays inside
        // the amplitude-16 validity bound of MixerState.
        let top = 1. - FaderScale::position(10f64.powf(12. / 20.));
        assert_eq!(fader_step(top, 1), top);
    }
}
