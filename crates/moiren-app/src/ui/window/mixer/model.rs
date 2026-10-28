use super::*;
use crate::service::{GraphEdit, GraphSnapshot};
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
    /// Click-to-type editors for the fader dB readouts; the strip readout
    /// and the inspector Fader row share one family.
    pub value_inputs: BTreeMap<NodeId, Entity<component::input::InputState>>,
    pub editing_value: Option<NodeId>,
    /// Balance-label editors, created for stereo nodes only: the label
    /// renders only there.
    pub balance_inputs: BTreeMap<NodeId, Entity<component::input::InputState>>,
    pub editing_balance: Option<NodeId>,
    /// Gain-node "Requested" editors. Kept apart from [`Self::value_inputs`]
    /// because a Gain node also has a strip fader, and the two edit
    /// different values (`mixer.gain` vs the node's own `gain` parameter).
    pub node_gain_inputs: BTreeMap<NodeId, Entity<component::input::InputState>>,
    pub editing_node_gain: Option<NodeId>,
    /// Pan-node position editors, likewise separate from the strip fader.
    pub node_pan_inputs: BTreeMap<NodeId, Entity<component::input::InputState>>,
    pub editing_node_pan: Option<NodeId>,
    readout_drafts: BTreeMap<(EditableValueFamily, NodeId), ReadoutDraft>,
    readout_sync: Option<(u64, crate::service::RequestId)>,
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

    pub(in crate::ui::window) fn readout_error(
        &self,
        family: EditableValueFamily,
        id: NodeId,
    ) -> Option<&str> {
        self.readout_drafts
            .get(&(family, id))
            .and_then(|draft| draft.error.as_deref())
    }
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

/// Loudness domain of the click-to-type dB entries. +24 dB keeps the parsed
/// amplitude inside [`crate::service::MixerState::valid`]'s 0..=16 bound (10^(24/20) ≈ 15.8);
/// the −120 dB floor matches the send readouts' editing domain.
const ENTRY_MIN_DB: f64 = -120.;
const ENTRY_MAX_DB: f64 = 24.;

/// Parses a fader/gain dB entry into an amplitude. Digital silence accepts
/// the `-inf` spellings (any case, ASCII or U+2212 minus, `∞`); otherwise
/// the text must be a finite number inside the entry domain. Out-of-range or
/// garbage input is `None` — never clamped — so an invalid entry changes
/// nothing.
pub(in crate::ui::window) fn parse_gain_db(text: &str) -> Option<f64> {
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
pub(in crate::ui::window) fn parse_balance(text: &str) -> Option<f64> {
    parse_centered(text, true)
}

/// Parses a pan-node position entry: a bare finite number in −1..=1, or the
/// same `C`/`L`/`R` percent grammar as [`parse_balance`].
pub(in crate::ui::window) fn parse_pan(text: &str) -> Option<f64> {
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
pub(in crate::ui::window) fn gain_db_text(gain: f64) -> String {
    if gain <= 0. {
        "-inf".into()
    } else {
        format!("{:.1}", 20. * gain.log10())
    }
}

/// Balance label text: `C` at center, otherwise `L`/`R` with the percent
/// offset. Shared by the strip label and its editor prefill so the shown
/// value always round-trips through [`parse_balance`].
pub(in crate::ui::window) fn balance_text(balance: f64) -> String {
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
pub(in crate::ui::window) type SliderEdit = Box<dyn Fn(f32, &GraphSnapshot) -> Option<AppRequest>>;

/// Loudest gain a [`crate::service::MixerState`] accepts. The fader scale tops out at +12 dB,
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

/// Which click-to-type editor family a readout belongs to. The strip fader
/// readout and the inspector Fader row share [`Self::Fader`]; the Gain and
/// Pan node fields stay separate families because a Gain/Pan node also has a
/// strip fader, and those edit a different value (`mixer.gain` vs the node's
/// own `gain`/`pan` parameter).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::ui::window) enum EditableValueFamily {
    Fader,
    Balance,
    NodeGain,
    NodePan,
}

struct ReadoutDraft {
    baseline: String,
    pending: Option<crate::service::RequestId>,
    error: Option<String>,
}

impl ReadoutDraft {
    fn new(baseline: String) -> Self {
        Self {
            baseline,
            pending: None,
            error: None,
        }
    }
}

impl EditableValueFamily {
    /// Display labels may round; entering edit mode must not inherit that loss.
    fn exact_text(self, graph: &GraphSnapshot, id: NodeId) -> Option<String> {
        let node = graph.node(id)?;
        let gain = match self {
            Self::Fader => node.mixer.gain,
            Self::NodeGain if graph.kind_of(id) == Some(NodeKind::Gain) => node.gain,
            Self::NodeGain => return None,
            Self::NodePan => {
                return (graph.kind_of(id) == Some(NodeKind::Pan)).then(|| node.pan.to_string());
            }
            Self::Balance => {
                return graph
                    .graph
                    .get_node(id)
                    .ok()
                    .filter(|node| node.channels() == 2)
                    .map(|_| (node.mixer.balance * 100.).to_string());
            }
        };
        Some(if gain == 0. {
            "-inf".into()
        } else {
            (20. * gain.log10()).to_string()
        })
    }

    fn parse(self, text: &str) -> Option<f64> {
        match self {
            Self::Fader | Self::NodeGain => parse_gain_db(text),
            Self::Balance => parse_balance(text),
            Self::NodePan => parse_pan(text),
        }
    }

    fn error(self) -> String {
        match self {
            Self::Fader | Self::NodeGain => "-120 .. 24 dB, or -inf",
            Self::Balance => "-100 .. 100 %",
            Self::NodePan => "-1 .. 1, or L/R percent",
        }
        .into()
    }

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
                &mut shell.mixer.editing_value,
                &mut shell.mixer.value_inputs,
            ),
            Self::Balance => (
                &mut shell.mixer.editing_balance,
                &mut shell.mixer.balance_inputs,
            ),
            Self::NodeGain => (
                &mut shell.mixer.editing_node_gain,
                &mut shell.mixer.node_gain_inputs,
            ),
            Self::NodePan => (
                &mut shell.mixer.editing_node_pan,
                &mut shell.mixer.node_pan_inputs,
            ),
        }
    }
}

/// Click handler that opens one editable numeric readout: enters the family's
/// edit mode, prefills the editor from the live snapshot and focuses it.
/// Parsing and dispatch live in the family's input subscription, created once
/// by [`sync_value_inputs`].
pub(in crate::ui::window) fn start_value_edit(
    this: Entity<StudioShell>,
    family: EditableValueFamily,
    id: NodeId,
    prefill: impl Fn(&GraphSnapshot) -> String + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        let focused = this.update(cx, |shell, cx| {
            if shell.pending_project.is_some() {
                return None;
            }
            let graph = &shell.state.read(cx).snapshot.graph;
            let text = family
                .exact_text(graph, id)
                .unwrap_or_else(|| prefill(graph));
            let consumed = shell.state.read(cx).snapshot.consumed_request_id;
            shell.mixer.readout_sync = None;
            let (editing, inputs) = family.parts(shell);
            *editing = Some(id);
            let focused = inputs.get(&id).cloned();
            if let Some(input) = focused.clone() {
                let draft = shell
                    .mixer
                    .readout_drafts
                    .entry((family, id))
                    .or_insert_with(|| ReadoutDraft::new(text.clone()));
                if draft.pending.is_some_and(|request| request <= consumed) {
                    draft.pending = None;
                }
                // Reopening before publication or after rejected input must
                // retain the submitted/draft text, not the old display label.
                if draft.pending.is_none()
                    && draft.error.is_none()
                    && input.read(cx).value().as_str() == draft.baseline
                {
                    draft.baseline = text.clone();
                    input.update(cx, |input, cx| input.set_value(text, window, cx));
                }
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

/// Sync only clean, inactive text. Drafts and accepted-but-unpublished edits
/// must survive a render caused by another control or a focus transition.
fn sync_value_inputs(
    window: &mut Window,
    cx: &mut Context<StudioShell>,
    shell: &mut StudioShell,
    family: EditableValueFamily,
    rows: Vec<(NodeId, String)>,
) {
    for (id, text) in rows {
        let key = (family, id);
        let (editing, map) = family.parts(shell);
        let active = *editing == Some(id);
        let input = map
            .entry(id)
            .or_insert_with(|| {
                let input = cx.new(|cx| {
                    component::input::InputState::new(window, cx).default_value(text.clone())
                });
                cx.subscribe(
                    &input,
                    move |shell, input, event: &component::input::InputEvent, cx| {
                        if matches!(
                            event,
                            component::input::InputEvent::PressEnter { .. }
                                | component::input::InputEvent::Blur
                        ) {
                            if family.parts(shell).1.get(&id).map(Entity::entity_id)
                                != Some(input.entity_id())
                            {
                                return;
                            }
                            let text = input.read(cx).value().to_string();
                            commit_value(shell, family, id, &text, cx);
                            cx.notify();
                        }
                    },
                )
                .detach();
                input
            })
            .clone();
        let draft = shell
            .mixer
            .readout_drafts
            .entry(key)
            .or_insert_with(|| ReadoutDraft::new(text.clone()));
        if !active
            && draft.pending.is_none()
            && draft.error.is_none()
            && input.read(cx).value().as_str() == draft.baseline
        {
            draft.baseline = text.clone();
            if input.read(cx).value().as_str() != text {
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
    }
}

fn sync_readouts(shell: &mut StudioShell, window: &mut Window, cx: &mut Context<StudioShell>) {
    let snapshot = shell.state.read(cx).snapshot.clone();
    let graph = &snapshot.graph;
    let consumed = snapshot.consumed_request_id;
    let token = (graph.candidate_revision, consumed);
    if shell.mixer.readout_sync == Some(token) {
        return;
    }
    shell.mixer.readout_sync = Some(token);
    shell.mixer.readout_drafts.retain(|(family, id), draft| {
        if draft.pending.is_some_and(|request| request <= consumed) {
            draft.pending = None;
        }
        family.exact_text(graph, *id).is_some()
    });
    for family in [
        EditableValueFamily::Fader,
        EditableValueFamily::Balance,
        EditableValueFamily::NodeGain,
        EditableValueFamily::NodePan,
    ] {
        let (editing, inputs) = family.parts(shell);
        inputs.retain(|id, _| family.exact_text(graph, *id).is_some());
        if editing.is_some_and(|id| !inputs.contains_key(&id)) {
            *editing = None;
        }
        let rows = graph
            .nodes
            .iter()
            .filter_map(|node| {
                family
                    .exact_text(graph, node.id)
                    .map(|text| (node.id, text))
            })
            .collect();
        sync_value_inputs(window, cx, shell, family, rows);
    }
}

fn commit_value(
    shell: &mut StudioShell,
    family: EditableValueFamily,
    id: NodeId,
    text: &str,
    cx: &mut App,
) {
    if shell.pending_project.is_some() {
        return;
    }
    let (editing, _) = family.parts(shell);
    if *editing != Some(id) {
        return;
    }
    // Preserve the existing click-to-type close behavior, including invalid
    // outside clicks. The text/error survives separately for a later retry.
    *editing = None;
    shell.mixer.readout_sync = None;
    let key = (family, id);
    if shell
        .mixer
        .readout_drafts
        .get(&key)
        .is_some_and(|draft| draft.baseline == text)
    {
        if let Some(draft) = shell.mixer.readout_drafts.get_mut(&key) {
            draft.error = None;
        }
        return;
    }
    let graph = &shell.state.read(cx).snapshot.graph;
    let Some(baseline) = family.exact_text(graph, id) else {
        return;
    };
    let Some(value) = family.parse(text) else {
        shell
            .mixer
            .readout_drafts
            .entry(key)
            .or_insert_with(|| ReadoutDraft::new(baseline))
            .error = Some(family.error());
        return;
    };
    let edit = match family {
        EditableValueFamily::NodeGain => GraphEdit::SetGain {
            node: id,
            gain: value,
        },
        EditableValueFamily::NodePan => GraphEdit::SetPan {
            node: id,
            pan: value,
        },
        EditableValueFamily::Fader => GraphEdit::EditMixer {
            node: id,
            edit: MixerEdit::SetGain(value),
        },
        EditableValueFamily::Balance => GraphEdit::EditMixer {
            node: id,
            edit: MixerEdit::SetBalance(value),
        },
    };
    let result = shell.state.update(cx, |state, _| {
        state.try_dispatch(AppRequest::EditGraph(edit))
    });
    let draft = shell
        .mixer
        .readout_drafts
        .entry(key)
        .or_insert_with(|| ReadoutDraft::new(baseline));
    match result {
        Ok(request) => {
            draft.baseline = text.to_owned();
            draft.pending = Some(request);
            draft.error = None;
        }
        Err(_) => draft.error = Some(t!("Edit could not be sent; try again").to_string()),
    }
}

fn commit_mixer_gain(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    commit_value(shell, EditableValueFamily::Fader, *id, text, cx);
}

fn commit_mixer_balance(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    commit_value(shell, EditableValueFamily::Balance, *id, text, cx);
}

fn commit_node_gain(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    commit_value(shell, EditableValueFamily::NodeGain, *id, text, cx);
}

fn commit_node_pan(shell: &mut StudioShell, id: &NodeId, text: &str, cx: &mut App) {
    commit_value(shell, EditableValueFamily::NodePan, *id, text, cx);
}

/// Enter/blur commit for the send gain editor; the input subscription and
/// [`commit_active_editor`] share it.
fn commit_send_gain(shell: &mut StudioShell, id: EdgeId, text: &str, cx: &mut App) {
    if shell.mixer.editing_send != Some(id) {
        return;
    }
    shell.mixer.editing_send = None;
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
}

/// Commits whichever inline editor is open, exactly as its Enter or blur
/// would — same guard, parse, and invalid-input semantics. The window root
/// listens for mouse presses and the editors stop presses inside their own
/// bounds from propagating, so a press that reaches the root landed outside
/// the active editor. Returns whether an editor was open (the caller
/// notifies only then).
pub(in crate::ui::window) fn commit_active_editor(shell: &mut StudioShell, cx: &mut App) -> bool {
    let mut active = false;
    if let Some(id) = shell.mixer.editing_value {
        active = true;
        if let Some(input) = shell.mixer.value_inputs.get(&id) {
            let text = input.read(cx).value().to_string();
            commit_mixer_gain(shell, &id, &text, cx);
        } else {
            shell.mixer.editing_value = None;
        }
    }
    if let Some(id) = shell.mixer.editing_balance {
        active = true;
        if let Some(input) = shell.mixer.balance_inputs.get(&id) {
            let text = input.read(cx).value().to_string();
            commit_mixer_balance(shell, &id, &text, cx);
        } else {
            shell.mixer.editing_balance = None;
        }
    }
    if let Some(id) = shell.mixer.editing_node_gain {
        active = true;
        if let Some(input) = shell.mixer.node_gain_inputs.get(&id) {
            let text = input.read(cx).value().to_string();
            commit_node_gain(shell, &id, &text, cx);
        } else {
            shell.mixer.editing_node_gain = None;
        }
    }
    if let Some(id) = shell.mixer.editing_node_pan {
        active = true;
        if let Some(input) = shell.mixer.node_pan_inputs.get(&id) {
            let text = input.read(cx).value().to_string();
            commit_node_pan(shell, &id, &text, cx);
        } else {
            shell.mixer.editing_node_pan = None;
        }
    }
    if let Some(id) = shell.mixer.editing_send {
        active = true;
        if let Some(input) = shell.mixer.send_inputs.get(&id) {
            let text = input.read(cx).value().to_string();
            commit_send_gain(shell, id, &text, cx);
        } else {
            shell.mixer.editing_send = None;
        }
    }
    active
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
                    graph.node(id).map(|_| {
                        AppRequest::EditGraph(GraphEdit::EditMixer {
                            node: id,
                            edit: MixerEdit::SetGain(FaderScale::gain(1. - value)),
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
        // Request acknowledgements and focus transitions can occur without a
        // new graph revision. Keep input lifecycle outside the slider cache.
        sync_readouts(self, window, cx);
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
        // The click-to-type editor families drop their vanished members like
        // the send inputs; balance only exists for stereo nodes and the Gain
        // /Pan node fields only for their kind, mirroring the surfaces that
        // render them.
        let channels_of = |id: NodeId| graph.graph.get_node(id).ok().map(|node| node.channels());
        let kind_of = |id: NodeId| graph.kind_of(id);
        self.mixer
            .value_inputs
            .retain(|id, _| graph.node(*id).is_some());
        self.mixer
            .balance_inputs
            .retain(|id, _| graph.node(*id).is_some() && channels_of(*id) == Some(2));
        self.mixer
            .node_gain_inputs
            .retain(|id, _| graph.node(*id).is_some() && kind_of(*id) == Some(NodeKind::Gain));
        self.mixer
            .node_pan_inputs
            .retain(|id, _| graph.node(*id).is_some() && kind_of(*id) == Some(NodeKind::Pan));
        if self
            .mixer
            .editing_value
            .is_some_and(|id| graph.node(id).is_none())
        {
            self.mixer.editing_value = None;
        }
        if self
            .mixer
            .editing_balance
            .is_some_and(|id| graph.node(id).is_none() || channels_of(id) != Some(2))
        {
            self.mixer.editing_balance = None;
        }
        if self
            .mixer
            .editing_node_gain
            .is_some_and(|id| kind_of(id) != Some(NodeKind::Gain))
        {
            self.mixer.editing_node_gain = None;
        }
        if self
            .mixer
            .editing_node_pan
            .is_some_and(|id| kind_of(id) != Some(NodeKind::Pan))
        {
            self.mixer.editing_node_pan = None;
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
                        if matches!(
                            event,
                            component::input::InputEvent::PressEnter { .. }
                                | component::input::InputEvent::Blur
                        ) {
                            let text = input.read(cx).value().to_string();
                            commit_send_gain(shell, id, &text, cx);
                            cx.notify();
                        }
                    },
                )
                .detach();
                input
            });
            if self.mixer.editing_send != Some(id) {
                let text = gain_db_text(edge.params().gain);
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
    use super::{
        DispatchGate, Duration, FaderScale, Instant, fader_step, parse_balance, parse_gain_db,
        parse_pan,
    };

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
