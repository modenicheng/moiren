//! The cross-surface slider and click-to-type editing kernel, shared by
//! every surface that edits a mixer-side value: the Mixer page's strips,
//! the graph page's inspector (channel strip, compressor parameters,
//! Gain/Pan fields) and the tray flyout. Extracted verbatim from
//! `pages/mixer/model.rs` (audit 2026-11-09 AU-1 / 2026-11-11 B-4), whose
//! graph and tray consumers reached the kernel through the shell's page
//! re-exports; mixer, graph and tray now import this module directly as
//! equal consumers.
//!
//! Two standing rules the kernel encodes (kept untouched by the move):
//! - A gpui `SliderState` serves exactly one view: the kit stores the
//!   pointer-mapping bounds of whichever view registered last, so every
//!   surface keeps its own slot set (strip faders, inspector fader rows,
//!   compressor sliders, the tray flyout's own map) over the same snapshot.
//! - Every edit a slot dispatches fences the snapshot write-back
//!   ([`SliderSlot::leads`], design 22): a refused or unconfirmed edit
//!   leaves the thumb at its own value until the snapshot catches up.

use gpui_kit::prelude::*;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::service::{AppRequest, GraphEdit, GraphSnapshot, MixerState, RequestId};
use crate::ui::shell::StudioShell;
use crate::ui::state::UiState;
use crate::ui::widgets::fader::{Commit, FaderScale, SliderBehavior, send_gain, send_value};
use moiren_core::graph::{EdgeId, NodeId, SendParams};

/// Per-slider bookkeeping kept next to its entity: the dragging flag and the
/// edit path's revision fence decide when the snapshot write-back in
/// [`sync_slider_range`] may move the thumb, and the behavior carries the
/// shared wheel/double-click actions.
pub(crate) struct SliderSlot {
    pub state: Entity<base::slider::SliderState>,
    pub dragging: Rc<Cell<bool>>,
    cancelled: Rc<Cell<bool>>,
    gate: Rc<RefCell<DispatchGate>>,
    dispatch: Rc<SliderDispatch>,
    pub behavior: SliderBehavior,
}

impl SliderSlot {
    /// Cancels an in-flight pointer drag: the write-back stand-down lifts
    /// immediately and the gesture's tail — the coalesced pending value and
    /// the release's stale pointer value — is dropped instead of landing as
    /// fresh edits. Used when a journal step (undo) lands under a live drag:
    /// without this, the suppressed write-back hides the undone value and
    /// the release then overwrites it (audit SD-7). The revision fence stays:
    /// the journal step is a newer request than any edit the drag sent, so
    /// its publish clears the fence while older echoes of the drag cannot.
    pub(crate) fn cancel_drag(&self) {
        self.dragging.set(false);
        self.cancelled.set(true);
        self.gate.borrow_mut().discard();
    }

    /// Whether the slider is ahead of the snapshot that consumed request
    /// `consumed`: a drag owns the thumb, or the slot's newest edit is not
    /// in that snapshot yet. Either way the snapshot's value for this slider
    /// is older than the one the slider shows.
    pub(crate) fn leads(&self, consumed: RequestId) -> bool {
        self.dragging.get() || self.dispatch.unconfirmed(consumed)
    }

    /// The slider's own value while it leads the published snapshot (see
    /// [`Self::leads`]); `None` once the snapshot caught up.
    pub(crate) fn leading_value(&self, cx: &App) -> Option<f32> {
        let consumed = self.dispatch.ui.read(cx).snapshot.consumed_request_id;
        self.leads(consumed)
            .then(|| self.state.read(cx).value().end())
    }
}

/// The gain a fader readout shows (design 22 D1): the slot's own value while
/// it leads the snapshot — a drag, or an edit the service has not confirmed
/// yet — and the published gain otherwise, so the number moves with the
/// thumb instead of trailing it by a service round trip. The edit dispatches
/// exactly this gain, so the switch back to the published one is seamless.
pub(crate) fn fader_readout_gain(slot: Option<&SliderSlot>, published: f64, cx: &App) -> f64 {
    slot.and_then(|slot| slot.leading_value(cx))
        .map_or(published, |value| FaderScale::gain(1. - value))
}

/// [`fader_readout_gain`] for a send slider, mapped through the send bar's
/// own gain pair ([`send_gain`]).
pub(crate) fn send_readout_gain(slot: Option<&SliderSlot>, published: f64, cx: &App) -> f64 {
    slot.and_then(|slot| slot.leading_value(cx))
        .map_or(published, send_gain)
}

/// One slider's edit path: builds the request against the live snapshot,
/// dispatches it, and fences the slot's snapshot write-back on it (design 22
/// D2). The drag subscription and the wheel/double-click commit share one,
/// so every edit a slot sends fences the echoes that predate it.
pub(crate) struct SliderDispatch {
    ui: Entity<UiState>,
    edit: SliderEdit,
    /// The newest request this slot dispatched. A snapshot that consumed an
    /// older request was built before the service applied it, so its value
    /// for this slider is stale: writing it back is the release echo race
    /// that pulled a just-released thumb back by one dispatch tick of
    /// travel, until the next publish snapped it forward again.
    sent: Cell<Option<RequestId>>,
}

impl SliderDispatch {
    fn new(ui: Entity<UiState>, edit: SliderEdit) -> Self {
        Self {
            ui,
            edit,
            sent: Cell::new(None),
        }
    }

    /// Applies one slider value; a target that vanished since the slider was
    /// created simply skips. A refused dispatch leaves the fence on the last
    /// request that did go out.
    fn send(&self, value: f32, cx: &mut App) {
        let sent = self.ui.update(cx, |state, _| {
            let request = (self.edit)(value, &state.snapshot.graph)?;
            state.try_dispatch(request).ok()
        });
        if sent.is_some() {
            self.sent.set(sent);
        }
    }

    /// Whether a snapshot that consumed request `consumed` predates this
    /// slot's newest edit.
    fn unconfirmed(&self, consumed: RequestId) -> bool {
        self.sent.get().is_some_and(|sent| consumed < sent)
    }
}

/// Drag Changes reach the service at most once per displayed frame (design
/// 22 D4). The first Change dispatches at once and arms a frame boundary;
/// later Changes before it supersede one pending value, which the boundary
/// dispatches — re-arming while the pointer keeps producing Changes and
/// reopening after a frame without one. A drag that pauses therefore lands
/// its last value one frame later, where the fixed 33 ms interval this
/// replaced only flushed on the next move or at release. Pure so the tests
/// drive it without gpui; the subscription schedules the boundary through
/// `Window::on_next_frame` ([`flush_at_next_frame`]).
#[derive(Default)]
struct DispatchGate {
    /// A frame boundary is scheduled: Changes until it only stash.
    armed: bool,
    pending: Option<f32>,
}

impl DispatchGate {
    /// One drag Change: `Some` to dispatch now — the caller then schedules
    /// the frame boundary — or `None` when it only supersedes the pending
    /// value.
    fn change(&mut self, value: f32) -> Option<f32> {
        if self.armed {
            self.pending = Some(value);
            None
        } else {
            self.armed = true;
            Some(value)
        }
    }

    /// The frame boundary: the pending value to dispatch now, which keeps
    /// the gate armed for one more frame (the caller schedules it); with
    /// nothing pending the gate reopens.
    fn frame(&mut self) -> Option<f32> {
        let pending = self.pending.take();
        self.armed = pending.is_some();
        pending
    }

    /// Drops the pending value: the release dispatches its own final value,
    /// and a cancelled gesture's tail must not land at all. A boundary that
    /// is already scheduled still fires, finds nothing and reopens the gate.
    fn discard(&mut self) {
        self.pending = None;
    }
}

/// Schedules a drag gate's frame boundary: the pending value, if any,
/// dispatches when the next frame starts, and the boundary re-arms for as
/// long as the pointer keeps the value moving.
fn flush_at_next_frame(
    window: &Window,
    gate: &Rc<RefCell<DispatchGate>>,
    dispatch: &Rc<SliderDispatch>,
) {
    let (gate, dispatch) = (gate.clone(), dispatch.clone());
    window.on_next_frame(move |window, cx| {
        let pending = gate.borrow_mut().frame();
        if let Some(value) = pending {
            dispatch.send(value, cx);
            flush_at_next_frame(window, &gate, &dispatch);
        }
    });
}

/// Loudness domain of the click-to-type dB entries. +24 dB keeps the parsed
/// amplitude inside [`MixerState::valid`]'s 0..=16 bound (10^(24/20) ≈ 15.8);
/// the −120 dB floor matches the send readouts' editing domain.
const ENTRY_MIN_DB: f64 = -120.;
const ENTRY_MAX_DB: f64 = 24.;

/// Parses a fader/gain dB entry into an amplitude. Digital silence accepts
/// the `-inf` spellings (any case, ASCII or U+2212 minus, `∞`); otherwise
/// the text must be a finite number inside the entry domain. Out-of-range or
/// garbage input is `None` — never clamped — so an invalid entry changes
/// nothing.
fn parse_gain_db(text: &str) -> Option<f64> {
    match text.trim().to_lowercase().as_str() {
        "-inf" | "−inf" | "-infinity" | "−infinity" | "-∞" | "−∞" => return Some(0.),
        _ => {}
    }
    let db: f64 = text.trim().parse().ok()?;
    (db.is_finite() && (ENTRY_MIN_DB..=ENTRY_MAX_DB).contains(&db)).then(|| 10f64.powf(db / 20.))
}

/// Parses a balance label entry (`C`, `L 50`, `R25`, `-25`, `+100`) into the
/// −1..=1 mixer balance. Numbers outside 0..=100 after the optional
/// left/right sign are rejected, never clamped (e.g. "150" is `None`).
fn parse_balance(text: &str) -> Option<f64> {
    parse_centered(text, true)
}

/// Parses a pan-node position entry: a bare finite number in −1..=1, or the
/// same `C`/`L`/`R` percent grammar as [`parse_balance`].
fn parse_pan(text: &str) -> Option<f64> {
    // The bare-number path also accepts the Unicode minus, so a typed raw
    // position like "−0.5" reads as a position and never as the percent
    // grammar's left-0.5%.
    let text = text.trim().replace('\u{2212}', "-");
    if let Ok(value) = text.parse::<f64>()
        && value.is_finite()
        && (-1. ..=1.).contains(&value)
    {
        return Some(value);
    }
    parse_centered(&text, false)
}

/// The `C` / `L`·`R`·`−`·`+` percent grammar shared by balance and pan:
/// center tokens, then an optional direction prefix before a 0..=100 number
/// mapped onto −1..=1. `bare` lets an un-signed number be a plain −100..=100
/// reading (balance); otherwise the direction prefix is required so a bare
/// out-of-range position is rejected instead of read as a percent.
fn parse_centered(text: &str, bare: bool) -> Option<f64> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("C") || text == "0" {
        return Some(0.);
    }
    let mut chars = text.char_indices();
    let (direction, skip) = match chars.next()?.1 {
        'l' | 'L' | '-' | '\u{2212}' => (-1., chars.next().map_or(text.len(), |(i, _)| i)),
        'r' | 'R' | '+' => (1., chars.next().map_or(text.len(), |(i, _)| i)),
        _ if bare => (1., 0),
        _ => return None,
    };
    let percent: f64 = text[skip..].trim().parse().ok()?;
    (percent.is_finite() && (0. ..=100.).contains(&percent)).then(|| direction * percent / 100.)
}

/// Editable text for a gain: exact digital silence is `-inf`, anything else
/// is dB with one decimal. Shared by the send sync and the click-to-type
/// prefills so both round-trip through [`parse_gain_db`].
pub(crate) fn gain_db_text(gain: f64) -> String {
    if gain <= 0. {
        "-inf".into()
    } else {
        format!("{:.1}", 20. * gain.log10())
    }
}

/// Balance label text: `C` at center, otherwise `L`/`R` with the percent
/// offset. Shared by the strip label and its editor prefill so the shown
/// value always round-trips through [`parse_balance`].
pub(crate) fn balance_text(balance: f64) -> String {
    if balance.abs() < 0.005 {
        "C".into()
    } else {
        format!(
            "{} {:.0}",
            if balance < 0. { "L" } else { "R" },
            balance.abs() * 100.
        )
    }
}

/// Builds the real-time edit for a slider's new value from the current
/// snapshot; None when the target vanished since the slider was created.
pub(crate) type SliderEdit = Box<dyn Fn(f32, &GraphSnapshot) -> Option<AppRequest>>;

/// Loudest gain a [`MixerState`] accepts. The fader scale tops out at +12 dB,
/// well below this, so clamping only guards edit validity.
const FADER_MAX_GAIN: f64 = 16.;

/// Wheel travel on a node fader: a multiplicative step of `notches` dB in
/// the perceptual value space (`1 - FaderScale::position(gain)`), ±1 per
/// notch and a fraction of it from fine-grained wheels. Stepping below the
/// −72 dB floor snaps to exact digital zero. Stepping up from below the
/// floor — exact silence, or a quieter gain only a drag reaches — enters one
/// step above the floor: silence cannot be multiplied into something
/// audible, and a sub-floor gain multiplied by a small step would still sit
/// below the floor and snap to silence, cutting the signal on a wheel up.
fn fader_step(value: f32, notches: f32) -> f32 {
    if notches == 0. {
        return value;
    }
    let floor = 10f64.powf(-72. / 20.);
    let gain = FaderScale::gain(1. - value);
    let step = 10f64.powf(f64::from(notches) / 20.);
    let stepped = if gain < floor && notches > 0. {
        floor * step
    } else {
        gain * step
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
/// drag coalescer. The dispatch fences the write-back like a drag's.
pub(crate) fn commit_closure(
    dispatch: &Rc<SliderDispatch>,
    slider: &Entity<base::slider::SliderState>,
) -> Commit {
    let dispatch = dispatch.clone();
    let slider = slider.clone();
    Rc::new(move |value, window, cx| {
        slider.update(cx, |state, cx| state.set_value(value, window, cx));
        dispatch.send(value, cx);
    })
}

/// Node fader behavior (main mixer, inspector channel strip, tray flyout):
/// ±1 dB per notch, double click restores unity. Slots built by
/// [`sync_faders`] carry it, so every surface renders the same gestures.
fn fader_behavior(
    dispatch: &Rc<SliderDispatch>,
    slider: Entity<base::slider::SliderState>,
) -> SliderBehavior {
    SliderBehavior {
        step: Rc::new(fader_step),
        reset: 1. - FaderScale::position(1.),
        commit: commit_closure(dispatch, &slider),
    }
}

/// Send bars share the channel fader's perceptual value space (see
/// [`fader::send_value`]) and gestures: ±1 dB per wheel notch in gain space,
/// double click restores unity, and the fader step's floor handling lets the
/// wheel leave exact silence.
fn send_behavior(
    dispatch: &Rc<SliderDispatch>,
    slider: Entity<base::slider::SliderState>,
) -> SliderBehavior {
    SliderBehavior {
        step: Rc::new(fader_step),
        reset: send_value(1.),
        commit: commit_closure(dispatch, &slider),
    }
}

/// Which click-to-type editor family a readout belongs to. The strip fader
/// readout and the inspector Fader row share [`Self::Fader`]; the Gain and
/// Pan node fields stay separate families because a Gain/Pan node also has a
/// strip fader, and those edit a different value (`mixer.gain` vs the node's
/// own `gain`/`pan` parameter).
#[derive(Clone, Copy)]
pub(crate) enum EditableValueFamily {
    Fader,
    Balance,
    NodeGain,
    NodePan,
}

impl EditableValueFamily {
    /// Split borrow of the family's editing flag and editor map.
    fn parts(
        self,
        shell: &mut StudioShell,
    ) -> (
        &mut Option<NodeId>,
        &mut BTreeMap<NodeId, Entity<component::input::InputState>>,
    ) {
        match self {
            Self::Fader => (
                &mut shell.mixer.presentation.editing_value,
                &mut shell.mixer.presentation.value_inputs,
            ),
            Self::Balance => (
                &mut shell.mixer.presentation.editing_balance,
                &mut shell.mixer.presentation.balance_inputs,
            ),
            Self::NodeGain => (
                &mut shell.mixer.presentation.editing_node_gain,
                &mut shell.mixer.presentation.node_gain_inputs,
            ),
            Self::NodePan => (
                &mut shell.mixer.presentation.editing_node_pan,
                &mut shell.mixer.presentation.node_pan_inputs,
            ),
        }
    }

    /// The family's Enter/blur commit, so one match drives both the family's
    /// input subscription and [`commit_active_editor`].
    fn commit(self) -> fn(&mut StudioShell, &NodeId, &str, &mut App) {
        match self {
            Self::Fader => commit_mixer_gain,
            Self::Balance => commit_mixer_balance,
            Self::NodeGain => commit_node_gain,
            Self::NodePan => commit_node_pan,
        }
    }
}

/// Click handler that opens one editable numeric readout: enters the family's
/// edit mode, prefills the editor from the live snapshot and focuses it.
/// Parsing and dispatch live in the family's input subscription, created once
/// by [`sync_value_inputs`].
pub(crate) fn start_value_edit(
    this: Entity<StudioShell>,
    family: EditableValueFamily,
    id: NodeId,
    prefill: impl Fn(&GraphSnapshot) -> String + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        let focused = this.update(cx, |shell, cx| {
            let text = prefill(&shell.state.read(cx).snapshot.graph);
            let (editing, inputs) = family.parts(shell);
            *editing = Some(id);
            let focused = inputs.get(&id).cloned();
            if let Some(input) = focused.clone() {
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
            cx.notify();
            focused
        });
        if let Some(input) = focused {
            let handle = input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
    }
}

/// One strip edit (mute/solo/mode/dim) shared by the main mixer and the tray
/// flyout: the edit applies through the live parameter path, then the hosting
/// view is notified so its page repaints. `owner` is whichever view hosts the
/// strip — each one notifies itself, so no surface changes whose render
/// invalidation wakes.
pub(crate) fn mixer_edit<O: 'static>(
    state: Entity<UiState>,
    owner: Entity<O>,
    id: NodeId,
    change: impl Fn(&mut MixerState) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        state.update(cx, |state, _| state.edit_mixer(id, &change));
        owner.update(cx, |_, cx| cx.notify());
    }
}

/// Clip-latch reset on a meter-surface press, shared by every meter surface
/// (main strip, tray flyout, inspector track): one handler so the click
/// semantics cannot drift between surfaces. The hosting view — not `state` —
/// is notified, because the hot layer painting the meters belongs to it.
pub(crate) fn reset_clip_on_click<O: 'static>(
    state: Entity<UiState>,
    owner: Entity<O>,
    id: NodeId,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) {
    move |_, _, cx| {
        state.update(cx, |state, _| state.node_meters.reset_clip(id));
        owner.update(cx, |_, cx| cx.notify());
    }
}

/// Creates (once) and value-syncs the inline editors for one keyed family.
/// While a family member is being edited its text stands down, so typing is
/// never overwritten by the snapshot write-back. `commit` runs on Enter and
/// blur: it guards and clears the family's editing flag, parses the text and
/// dispatches only a valid value — invalid input just closes the editor.
pub(crate) fn sync_value_inputs<K: Ord + Clone + 'static>(
    window: &mut Window,
    cx: &mut Context<StudioShell>,
    map: &mut BTreeMap<K, Entity<component::input::InputState>>,
    editing: Option<&K>,
    rows: Vec<(K, String)>,
    commit: impl Fn(&mut StudioShell, &K, &str, &mut App) + Clone + 'static,
) {
    for (id, text) in rows {
        let commit = commit.clone();
        let captured = id.clone();
        let input = map.entry(id.clone()).or_insert_with(|| {
            let input = cx.new(|cx| component::input::InputState::new(window, cx));
            cx.subscribe(
                &input,
                move |shell, input, event: &component::input::InputEvent, cx| {
                    if matches!(
                        event,
                        component::input::InputEvent::PressEnter { .. }
                            | component::input::InputEvent::Blur
                    ) {
                        let text = input.read(cx).value().to_string();
                        commit(shell, &captured, &text, cx);
                        cx.notify();
                    }
                },
            )
            .detach();
            input
        });
        // InputState::set_value notifies unconditionally, even for
        // identical text — writing every publish would wake every panel
        // that observes its editors. Skip the write when the text is
        // already current.
        if editing != Some(&id) && input.read(cx).value() != text {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }
}

/// Enter/blur commit shared by the fader readout editors: the edit is built
/// from the live snapshot like the slider path, so a vanished node skips.
/// The input subscription that calls these notifies afterwards.
pub(crate) fn commit_mixer_gain(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    if shell.mixer.presentation.editing_value != Some(*id) {
        return;
    }
    shell.mixer.presentation.editing_value = None;
    if let Some(gain) = parse_gain_db(text) {
        shell.state.update(cx, |state, _| {
            state.edit_mixer(*id, |state| state.gain = gain)
        });
    }
}

/// Enter/blur commit for the balance label editors.
pub(crate) fn commit_mixer_balance(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    if shell.mixer.presentation.editing_balance != Some(*id) {
        return;
    }
    shell.mixer.presentation.editing_balance = None;
    if let Some(balance) = parse_balance(text) {
        shell.state.update(cx, |state, _| {
            state.edit_mixer(*id, |state| state.balance = balance);
        });
    }
}

/// Enter/blur commit for the Gain node's "Requested" dB field.
pub(crate) fn commit_node_gain(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    if shell.mixer.presentation.editing_node_gain != Some(*id) {
        return;
    }
    shell.mixer.presentation.editing_node_gain = None;
    if let Some(gain) = parse_gain_db(text) {
        shell.state.update(cx, |state, _| {
            if state.snapshot.graph.node(*id).is_some() {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetGain {
                    node: *id,
                    gain,
                }));
            }
        });
    }
}

/// Enter/blur commit for the Pan node's position field.
pub(crate) fn commit_node_pan(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    if shell.mixer.presentation.editing_node_pan != Some(*id) {
        return;
    }
    shell.mixer.presentation.editing_node_pan = None;
    if let Some(pan) = parse_pan(text) {
        shell.state.update(cx, |state, _| {
            if state.snapshot.graph.node(*id).is_some() {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetPan { node: *id, pan }));
            }
        });
    }
}

/// Enter/blur commit for the send gain editor; the input subscription and
/// [`commit_active_editor`] share it. Parsing goes through [`parse_gain_db`]
/// like every fader readout, so the send and fader entries accept the same
/// silence spellings and domain.
pub(crate) fn commit_send_gain(shell: &mut StudioShell, id: &EdgeId, text: &str, cx: &mut App) {
    if shell.mixer.presentation.editing_send != Some(*id) {
        return;
    }
    shell.mixer.presentation.editing_send = None;
    if let Some(gain) = parse_gain_db(text) {
        shell.state.update(cx, |state, _| {
            if let Ok(edge) = state.snapshot.graph.graph.get_edge(*id) {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetSend {
                    edge: *id,
                    params: SendParams {
                        gain,
                        ..*edge.params()
                    },
                }));
            }
        });
    }
}

/// Commits whichever inline editor is open, exactly as its Enter or blur
/// would — same guard, parse, and invalid-input semantics. The window root
/// listens for mouse presses and the editors stop presses inside their own
/// bounds from propagating, so a press that reaches the root landed outside
/// the active editor. Returns whether an editor was open (the caller
/// notifies only then).
pub(crate) fn commit_active_editor(shell: &mut StudioShell, cx: &mut App) -> bool {
    let mut active = false;
    for family in [
        EditableValueFamily::Fader,
        EditableValueFamily::Balance,
        EditableValueFamily::NodeGain,
        EditableValueFamily::NodePan,
    ] {
        let (editing, inputs) = family.parts(shell);
        let Some(id) = *editing else {
            continue;
        };
        active = true;
        if let Some(text) = inputs
            .get(&id)
            .map(|input| input.read(cx).value().to_string())
        {
            (family.commit())(shell, &id, &text, cx);
        } else {
            *editing = None;
        }
    }
    // The send family is EdgeId-keyed, so it stays explicit rather than
    // joining the NodeId-keyed loop above.
    if let Some(id) = shell.mixer.presentation.editing_send {
        active = true;
        if let Some(input) = shell.mixer.presentation.send_inputs.get(&id) {
            let text = input.read(cx).value().to_string();
            commit_send_gain(shell, &id, &text, cx);
        } else {
            shell.mixer.presentation.editing_send = None;
        }
    }
    active
}

/// Fader rows for every node, shared verbatim by the main mixer and the tray
/// flyout so both edit the same perceptual value space `[0..1]`: value
/// `1 - FaderScale::position(gain)` (1 = track top), and an edit maps back
/// through `FaderScale::gain`. The slot's subscription coalesces drag
/// Changes and dispatches the release's final value.
pub(crate) fn fader_rows(snapshot: &GraphSnapshot) -> Vec<(NodeId, f32, SliderEdit)> {
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
pub(crate) fn sync_faders<T: 'static>(
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
        |_, dispatch, slider| fader_behavior(dispatch, slider),
    );
}

/// Slider construction and value sync for the send (edge) column. The value
/// space is [`fader::send_value`]'s perceptual fraction — the same number the
/// bar paints as its fill — so dragging right raises the level with the fill
/// tracking the pointer one-to-one, the right rail is the +12 dB scale top
/// and the left rail is exact silence. Drag Changes are coalesced and the
/// release's final value dispatches unconditionally.
pub(crate) fn sync_sliders<K: Ord + Clone, T: 'static>(
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
        [0., 1., 0.0001],
        |_, dispatch, slider| send_behavior(dispatch, slider),
    );
}

/// Slider construction and value sync shared by every mixer-style slider
/// (channel faders, sends, compressor parameters): per-slider slot with the
/// dragging flag and wheel/double-click behavior, drag Changes coalesced
/// through [`DispatchGate`], the release's final value dispatched, and the
/// snapshot write-back suppressed while the slider leads the snapshot (see
/// [`SliderSlot::leads`]). `behavior` receives the row key so one call can
/// serve keyed parameter families.
pub(crate) fn sync_slider_range<K: Ord + Clone, T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    state: &Entity<UiState>,
    sliders: &mut BTreeMap<K, SliderSlot>,
    rows: Vec<(K, f32, SliderEdit)>,
    range: [f32; 3],
    behavior: impl Fn(&K, &Rc<SliderDispatch>, Entity<base::slider::SliderState>) -> SliderBehavior,
) {
    let consumed = state.read(cx).snapshot.consumed_request_id;
    for (id, value, edit) in rows {
        let slot = sliders.entry(id.clone()).or_insert_with(|| {
            let dispatch = Rc::new(SliderDispatch::new(state.clone(), edit));
            let dragging = Rc::new(Cell::new(false));
            let cancelled = Rc::new(Cell::new(false));
            let slider = cx.new(|_| {
                base::slider::SliderState::new()
                    .min(range[0])
                    .max(range[1])
                    .step(range[2])
                    .default_value(value)
            });
            let drag_flag = dragging.clone();
            let cancel_flag = cancelled.clone();
            let drag_dispatch = dispatch.clone();
            let gate = Rc::new(RefCell::new(DispatchGate::default()));
            let drag_gate = gate.clone();
            cx.subscribe_in(
                &slider,
                window,
                move |_, slider, event: &base::slider::SliderEvent, window, cx| {
                    match event {
                        base::slider::SliderEvent::Change(value) => {
                            drag_flag.set(true);
                            // Pointer movement after a cancel reopens the
                            // drag: these edits are the user's own again,
                            // stacked on whatever the journal step landed.
                            cancel_flag.set(false);
                            let value = value.end();
                            // A degenerate pointer mapping (zero state bounds
                            // before the first layout) yields NaN; dispatching
                            // it would produce an invalid gain and strand the
                            // thumb until the next write-back.
                            if !value.is_finite() {
                                return;
                            }
                            let now = drag_gate.borrow_mut().change(value);
                            if let Some(value) = now {
                                drag_dispatch.send(value, cx);
                                flush_at_next_frame(window, &drag_gate, &drag_dispatch);
                            }
                        }
                        base::slider::SliderEvent::Release(value) => {
                            drag_flag.set(false);
                            drag_gate.borrow_mut().discard();
                            let value = value.end();
                            // A cancelled drag's release carries only the
                            // stale pointer tail: the journal already moved
                            // on, and landing it would overwrite the undone
                            // value (audit SD-7).
                            if !value.is_finite() || cancel_flag.take() {
                                return;
                            }
                            // The release always lands — unless a wheel or
                            // double-click commit already moved the slider
                            // past it: those handlers paint before the
                            // slider's release listener, so on the same
                            // mouse-up a stale track value would otherwise
                            // overwrite the committed one.
                            if slider.read(cx).value().end() == value {
                                drag_dispatch.send(value, cx);
                            }
                        }
                    }
                    cx.notify();
                },
            )
            .detach();
            SliderSlot {
                behavior: behavior(&id, &dispatch, slider.clone()),
                state: slider,
                dragging,
                cancelled,
                gate,
                dispatch,
            }
        });
        // The snapshot reconciles into the slider only once it caught up
        // with it. A live drag owns the thumb, and a snapshot older than the
        // slot's newest edit — the release's, a wheel step's — still carries
        // a value the pointer already left behind.
        if !slot.leads(consumed) && slot.state.read(cx).value().end() != value {
            slot.state
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DispatchGate, fader_step, parse_balance, parse_gain_db, parse_pan};
    use crate::ui::widgets::fader::FaderScale;

    #[test]
    fn first_change_dispatches_immediately_and_arms_the_frame_boundary() {
        let mut gate = DispatchGate::default();
        assert_eq!(gate.change(0.5), Some(0.5));
        assert_eq!(
            gate.change(0.6),
            None,
            "until the boundary, Changes only stash"
        );
    }

    #[test]
    fn changes_within_a_frame_supersede_one_pending_value() {
        let mut gate = DispatchGate::default();
        assert_eq!(gate.change(0.5), Some(0.5));
        assert_eq!(gate.change(0.6), None);
        assert_eq!(gate.change(0.7), None);
        assert_eq!(gate.frame(), Some(0.7), "the boundary sends the newest");
    }

    #[test]
    fn the_boundary_rearms_while_changes_keep_coming_and_reopens_after_a_quiet_frame() {
        let mut gate = DispatchGate::default();
        assert_eq!(gate.change(0.5), Some(0.5));
        assert_eq!(gate.change(0.6), None);
        assert_eq!(gate.frame(), Some(0.6));
        // The flush keeps the gate armed for the frame it started.
        assert_eq!(gate.change(0.65), None);
        assert_eq!(gate.frame(), Some(0.65));
        // A frame without Changes reopens it: the next one goes out at once.
        assert_eq!(gate.frame(), None);
        assert_eq!(gate.change(0.7), Some(0.7));
    }

    #[test]
    fn a_paused_drag_lands_its_last_value_at_the_next_frame() {
        // The fixed interval this gate replaced flushed only on the next
        // Change past the interval or at release: a drag that stopped
        // moving left the service on an older value while the button stayed
        // down. Frames keep coming without Changes.
        let mut gate = DispatchGate::default();
        assert_eq!(gate.change(0.5), Some(0.5));
        assert_eq!(gate.change(0.42), None);
        assert_eq!(gate.frame(), Some(0.42));
        assert_eq!(gate.frame(), None);
    }

    #[test]
    fn discard_drops_the_pending_value_and_the_next_drag_resumes_immediately() {
        let mut gate = DispatchGate::default();
        assert_eq!(gate.change(0.5), Some(0.5));
        assert_eq!(gate.change(0.6), None);
        gate.discard();
        assert_eq!(
            gate.frame(),
            None,
            "a released or cancelled gesture's tail never lands"
        );
        assert_eq!(gate.change(0.1), Some(0.1));
    }

    #[test]
    fn fader_step_moves_one_db_and_round_trips_symmetrically() {
        let at_db = |db: f64| 1. - FaderScale::position(10f64.powf(db / 20.));
        let db_of = |value: f32| 20. * FaderScale::gain(1. - value).log10();
        // Up then down from −6 dB returns to the same slider value.
        let start = at_db(-6.);
        let up = fader_step(start, 1.);
        assert!((db_of(up) + 5.).abs() < 0.001, "got {} dB", db_of(up));
        assert!((fader_step(up, -1.) - start).abs() < 0.00001);
        let down = fader_step(start, -1.);
        assert!((db_of(down) + 7.).abs() < 0.001, "got {} dB", db_of(down));
    }

    #[test]
    fn fader_step_glides_by_fractional_notches() {
        let at_db = |db: f64| 1. - FaderScale::position(10f64.powf(db / 20.));
        let db_of = |value: f32| 20. * FaderScale::gain(1. - value).log10();
        let start = at_db(-6.);
        let quarter = fader_step(start, 0.25);
        assert!(
            (db_of(quarter) + 5.75).abs() < 0.001,
            "got {} dB",
            db_of(quarter)
        );
        // Four quarter notches travel exactly one notch.
        let value = (0..4).fold(start, |value, _| fader_step(value, 0.25));
        assert!((db_of(value) + 5.).abs() < 0.001, "got {} dB", db_of(value));
    }

    #[test]
    fn fader_step_unsilences_into_the_quiet_tail_and_snaps_back_to_silence() {
        // Silence cannot be multiplied into something audible: the first
        // up-step lands one dB above the −72 dB floor, inside the final
        // finite segment between the −72 and −60 dB ticks.
        let unsilenced = fader_step(0., 1.);
        let db = 20. * FaderScale::gain(1. - unsilenced).log10();
        assert!(db > -72. && db <= -60., "got {db} dB");
        // Stepping down again returns to the floor (still finite); one more
        // step falls below it and snaps to exact digital zero.
        let floor = fader_step(unsilenced, -1.);
        assert!(
            (FaderScale::gain(1. - floor) - 10f64.powf(-72. / 20.)).abs() < 1e-9,
            "expected the −72 dB floor"
        );
        assert_eq!(FaderScale::gain(1. - fader_step(floor, -1.)), 0.);
        // Silence is sticky downwards.
        assert_eq!(fader_step(0., -1.), 0.);
    }

    #[test]
    fn fader_step_up_from_below_the_floor_never_cuts_to_silence() {
        // A drag can park the fader below the −72 dB floor. A wheel up there
        // used to multiply the gain by its step, stay under the floor and
        // snap to digital silence — muting the strip on a wheel up, and on
        // every fractional event of a fine-grained wheel.
        let deep = 1. - FaderScale::position(10f64.powf(-90. / 20.));
        for notches in [1., 0.25] {
            let db = 20. * FaderScale::gain(1. - fader_step(deep, notches)).log10();
            assert!(
                db > -72. && db < -70.9,
                "{notches} notch up from -90 dB landed at {db} dB"
            );
        }
    }

    #[test]
    fn fader_step_clamps_at_the_loud_ceiling() {
        // Stepping above the +12 dB scale top clamps back to the top value
        // through FaderScale::position, and the stepped gain stays inside
        // the amplitude-16 validity bound of MixerState.
        let top = 1. - FaderScale::position(10f64.powf(12. / 20.));
        assert_eq!(fader_step(top, 1.), top);
    }

    #[test]
    fn parse_gain_db_reads_the_entry_domain_and_silence_tokens() {
        assert_eq!(parse_gain_db("0"), Some(1.));
        assert_eq!(parse_gain_db(" -6 "), Some(10f64.powf(-6. / 20.)));
        assert_eq!(parse_gain_db("+24"), Some(10f64.powf(24. / 20.)));
        // Domain edges: the loudest boost is accepted, one tenth of a dB
        // more is rejected; the quiet floor mirrors the send editor.
        assert_eq!(parse_gain_db("-120"), Some(10f64.powf(-6.)));
        assert_eq!(parse_gain_db("24.1"), None);
        assert_eq!(parse_gain_db("-120.1"), None);
        // Silence accepts every spelling the readouts and users produce.
        assert_eq!(parse_gain_db("-inf"), Some(0.));
        assert_eq!(parse_gain_db("-INF"), Some(0.));
        assert_eq!(parse_gain_db("−Inf"), Some(0.));
        assert_eq!(parse_gain_db("-∞"), Some(0.));
        assert_eq!(parse_gain_db("−∞"), Some(0.));
        assert_eq!(parse_gain_db("−infinity"), Some(0.));
        // Plain "+inf" is loudness without a bound: rejected, not silence.
        assert_eq!(parse_gain_db("+inf"), None);
        assert_eq!(parse_gain_db("inf"), None);
        assert_eq!(parse_gain_db("nan"), None);
        assert_eq!(parse_gain_db("abc"), None);
        assert_eq!(parse_gain_db(""), None);
        assert_eq!(parse_gain_db("99"), None);
    }

    #[test]
    fn parse_balance_reads_center_and_percent_offsets() {
        assert_eq!(parse_balance("C"), Some(0.));
        assert_eq!(parse_balance("c"), Some(0.));
        assert_eq!(parse_balance("0"), Some(0.));
        assert_eq!(parse_balance("L 50"), Some(-0.5));
        assert_eq!(parse_balance("R25"), Some(0.25));
        assert_eq!(parse_balance("l100"), Some(-1.));
        assert_eq!(parse_balance("-25"), Some(-0.25));
        assert_eq!(parse_balance("+100"), Some(1.));
        assert_eq!(parse_balance("100"), Some(1.));
        assert_eq!(parse_balance("-100"), Some(-1.));
        // Out of range is rejected, never clamped into range.
        assert_eq!(parse_balance("150"), None);
        assert_eq!(parse_balance("L101"), None);
        assert_eq!(parse_balance("-100.5"), None);
        assert_eq!(parse_balance("NaN"), None);
        assert_eq!(parse_balance("abc"), None);
        assert_eq!(parse_balance(""), None);
    }

    #[test]
    fn parse_pan_reads_positions_and_the_percent_form() {
        assert_eq!(parse_pan("0"), Some(0.));
        assert_eq!(parse_pan("-1"), Some(-1.));
        assert_eq!(parse_pan("1"), Some(1.));
        assert_eq!(parse_pan(" 0.25 "), Some(0.25));
        assert_eq!(parse_pan("−0.5"), Some(-0.5));
        assert_eq!(parse_pan("R25"), Some(0.25));
        assert_eq!(parse_pan("L 50"), Some(-0.5));
        assert_eq!(parse_pan("C"), Some(0.));
        // A bare number is a position, not a percent: out of −1..=1 it is
        // rejected even though "100" would be valid percent form.
        assert_eq!(parse_pan("1.01"), None);
        assert_eq!(parse_pan("150"), None);
        assert_eq!(parse_pan("inf"), None);
        assert_eq!(parse_pan("abc"), None);
        assert_eq!(parse_pan(""), None);
    }
}
