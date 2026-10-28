//! Inspector-only integer editors with independent native draft state.
//!
//! Plain `Editors` belong to NodeControls: sync before rendering and include
//! `owns_focus` in typing focus. Reset them on project load because NodeId alone
//! is not cross-document identity.
use super::*;
use crate::service::{AppSnapshot, BuiltinState, RequestId};
use component::input::{InputEvent, InputState};
use moiren_engine::recorder::RecorderSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::ui::window::graph) enum Field {
    MeterWindow,
    RecorderCapacity,
}

impl Field {
    fn kind(self) -> NodeKind {
        match self {
            Self::MeterWindow => NodeKind::Meter,
            Self::RecorderCapacity => NodeKind::Recorder,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::MeterWindow => "Integration window",
            Self::RecorderCapacity => "Recording buffer capacity",
        }
    }
}

type Key = (NodeId, Field);

struct Draft {
    baseline: String,
    last_observed: String,
    edited: bool,
    error: Option<String>,
    pending: Option<RequestId>,
}

impl Draft {
    fn new(baseline: String) -> Self {
        Self {
            last_observed: baseline.clone(),
            baseline,
            edited: false,
            error: None,
            pending: None,
        }
    }

    fn dirty(&self, text: &str) -> bool {
        self.edited || self.baseline != text
    }

    fn can_sync(&self, text: &str, focused: bool) -> bool {
        !focused && !self.dirty(text) && self.error.is_none() && self.pending.is_none()
    }

    fn submitted(&mut self, text: String, request: RequestId) {
        self.last_observed = text.clone();
        self.baseline = text;
        self.edited = false;
        self.error = None;
        self.pending = Some(request);
    }

    fn acknowledge(&mut self, consumed: RequestId) {
        if self.pending.is_some_and(|request| request <= consumed) {
            self.pending = None;
        }
    }
}

#[derive(Default)]
pub(super) struct Editors {
    inputs: BTreeMap<Key, Entity<InputState>>,
    drafts: BTreeMap<Key, Draft>,
}

impl Editors {
    pub(super) fn owns_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.inputs
            .values()
            .any(|input| input.read(cx).focus_handle(cx) == *focused)
    }
}

pub(super) fn sync(shell: &mut StudioShell, window: &mut Window, cx: &mut Context<StudioShell>) {
    let snapshot = shell.state.read(cx).snapshot.clone();
    let editors = &mut shell.node_controls.integer;
    editors
        .inputs
        .retain(|key, _| value_and_maximum(&snapshot, *key).is_some());
    editors
        .drafts
        .retain(|key, _| editors.inputs.contains_key(key));
    for node in &snapshot.graph.nodes {
        for field in [Field::MeterWindow, Field::RecorderCapacity] {
            let key = (node.id, field);
            let Some((value, _)) = value_and_maximum(&snapshot, key) else {
                continue;
            };
            let text = value.to_string();
            let draft = editors
                .drafts
                .entry(key)
                .or_insert_with(|| Draft::new(text.clone()));
            draft.acknowledge(snapshot.consumed_request_id);
            let input = editors.inputs.entry(key).or_insert_with(|| {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
                cx.subscribe(&input, move |shell, input, event: &InputEvent, cx| {
                    if matches!(
                        event,
                        InputEvent::Change | InputEvent::PressEnter { .. } | InputEvent::Blur
                    ) {
                        commit(shell, input, key, event, cx);
                    }
                })
                .detach();
                input
            });
            if draft.can_sync(
                input.read(cx).value().as_str(),
                input.read(cx).focus_handle(cx).is_focused(window),
            ) {
                if input.read(cx).value().as_str() != text {
                    input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                }
                draft.last_observed = text.clone();
                draft.baseline = text;
            }
        }
    }
}

fn commit(
    shell: &mut StudioShell,
    input: Entity<InputState>,
    key: Key,
    event: &InputEvent,
    cx: &mut Context<StudioShell>,
) {
    // Load can reuse this NodeId before the old input entity is retired.
    if shell.pending_project.is_some() {
        return;
    }
    // A removed input may still emit Blur after its replacement is mounted.
    if shell
        .node_controls
        .integer
        .inputs
        .get(&key)
        .map(Entity::entity_id)
        != Some(input.entity_id())
    {
        return;
    }
    let text = input.read(cx).value().to_string();
    let Some(draft) = shell.node_controls.integer.drafts.get_mut(&key) else {
        return;
    };
    if matches!(event, InputEvent::Change) {
        // IME/normalized empty replacements may emit Change without changing
        // text, including after Enter. Track actual transitions, keeping intent
        // sticky across backspace/retype until a successful submission.
        if draft.last_observed != text {
            draft.last_observed = text;
            draft.edited = true;
        }
        return;
    }
    if !draft.dirty(&text) {
        draft.error = None;
        cx.notify();
        return;
    }
    let snapshot = shell.state.read(cx).snapshot.clone();
    let edit = match edit_for(&snapshot, key, &text) {
        Ok(edit) => edit,
        Err(error) => {
            draft.error = Some(error);
            cx.notify();
            return;
        }
    };
    // Never skip a dirty value just because it equals an old snapshot: a
    // preceding queued edit may still need this final value to reverse it.
    match shell.state.update(cx, |state, _| {
        state.try_dispatch(AppRequest::EditGraph(edit))
    }) {
        Ok(request) => draft.submitted(text, request),
        Err(_) => draft.error = Some("Edit could not be sent; try again".into()),
    }
    cx.notify();
}

pub(in crate::ui::window::graph) fn render(
    shell: &StudioShell,
    node: NodeId,
    field: Field,
    cx: &App,
) -> AnyElement {
    let key = (node, field);
    let snapshot = &shell.state.read(cx).snapshot;
    let Some((_, maximum)) = value_and_maximum(snapshot, key) else {
        return div().into_any_element();
    };
    let Some(input) = shell.node_controls.integer.inputs.get(&key) else {
        return div().into_any_element();
    };
    let busy = recording_busy(snapshot, key);
    let mut view = div()
        .id(format!("integer-precision-{node:?}-{field:?}"))
        .flex()
        .flex_col()
        .flex_none()
        .min_w_0()
        .w_full()
        .gap(px(4.))
        .child(note(field.label()))
        .child(Styled::h(
            component::input::Input::new(input)
                .id(format!("integer-exact-{node:?}-{field:?}"))
                .disabled(busy)
                .font_family(theme::FONT_MONO)
                .text_size(px(12.))
                .px(px(6.))
                .py(px(0.))
                .bg(theme::SURFACE_3),
            px(28.),
        ))
        .child(note(format!("1 .. {maximum} frames")));
    if busy {
        view = view.child(note(
            "Stop and finalize recording before changing buffer capacity",
        ));
    }
    if let Some(error) = shell
        .node_controls
        .integer
        .drafts
        .get(&key)
        .and_then(|draft| draft.error.as_ref())
    {
        view = view.child(
            note(error.clone())
                .id(format!("integer-error-{node:?}-{field:?}"))
                .text_color(theme::DANGER)
                .gpui_test_observe(),
        );
    }
    view.gpui_test_observe().into_any_element()
}

fn parse_integer(text: &str) -> Option<usize> {
    let text = text.trim();
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<usize>().ok())
        .flatten()
}

fn value_and_maximum(snapshot: &AppSnapshot, key: Key) -> Option<(usize, usize)> {
    let topology = snapshot.graph.graph.get_node(key.0).ok()?;
    if topology.kind() != key.1.kind() {
        return None;
    }
    let state = snapshot
        .graph
        .node(key.0)?
        .builtin
        .resolved(topology.kind());
    match (key.1, state) {
        (Field::MeterWindow, BuiltinState::Meter(settings)) => {
            Some((settings.window_frames, 1_048_576))
        }
        (Field::RecorderCapacity, BuiltinState::Recorder(settings)) => Some((
            settings.capacity_frames,
            RecorderSettings::max_capacity_frames(topology.channels())?,
        )),
        _ => None,
    }
}

fn recording_busy(snapshot: &AppSnapshot, key: Key) -> bool {
    key.1 == Field::RecorderCapacity
        && snapshot
            .recordings
            .iter()
            .any(|recording| recording.node == key.0 && recording.phase.is_busy())
}

fn edit_for(snapshot: &AppSnapshot, key: Key, text: &str) -> Result<GraphEdit, String> {
    let (_, maximum) =
        value_and_maximum(snapshot, key).ok_or("The integer field is unavailable")?;
    if recording_busy(snapshot, key) {
        return Err("Stop and finalize recording before changing buffer capacity".into());
    }
    let value = parse_integer(text)
        .filter(|value| (1..=maximum).contains(value))
        .ok_or_else(|| format!("Enter a whole number from 1 to {maximum} frames"))?;
    let topology = snapshot
        .graph
        .graph
        .get_node(key.0)
        .map_err(|error| error.to_string())?;
    let node = snapshot
        .graph
        .node(key.0)
        .ok_or("The node does not exist")?;
    let mut state = node.builtin.resolved(topology.kind());
    match (key.1, &mut state) {
        (Field::MeterWindow, BuiltinState::Meter(settings)) => settings.window_frames = value,
        (Field::RecorderCapacity, BuiltinState::Recorder(settings)) => {
            settings.capacity_frames = value
        }
        _ => return Err("The integer field is unavailable".into()),
    }
    state.validate(
        topology.kind(),
        topology.inputs().first().map_or(0, |port| port.channels()),
        topology.channels(),
    )?;
    // A queued Start may not be in this snapshot yet. The service's busy guard
    // remains authoritative when it consumes SetBuiltin; no optimistic graph edit.
    Ok(GraphEdit::SetBuiltin { node: key.0, state })
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "gpui-test"))]
mod native_tests;
