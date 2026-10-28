//! Inspector-only numeric editors. The card never mounts these input entities.
use super::*;
use component::input::{InputEvent, InputState};

#[cfg(all(test, feature = "gpui-test"))]
#[path = "precision_tests.rs"]
mod tests;

type Key = (NodeId, Parameter);

#[derive(Default)]
pub(super) struct Editors {
    pub(super) inputs: BTreeMap<Key, Entity<InputState>>,
    baselines: BTreeMap<Key, String>,
    observed: BTreeMap<Key, String>,
    edited: BTreeSet<Key>,
    pub(super) errors: BTreeMap<Key, String>,
    pending: BTreeMap<Key, crate::service::RequestId>,
}

impl Editors {
    pub(super) fn owns_focus(&self, focused: &FocusHandle, cx: &App) -> bool {
        self.inputs
            .values()
            .any(|input| input.read(cx).focus_handle(cx) == *focused)
    }
}

const EQ_PARAMETERS: [Parameter; 3] = [Parameter::EqFrequency, Parameter::EqGain, Parameter::EqQ];
const GATE_PARAMETERS: [Parameter; 5] = [
    Parameter::GateThreshold,
    Parameter::GateRange,
    Parameter::GateAttack,
    Parameter::GateHold,
    Parameter::GateRelease,
];
const LIMITER_PARAMETERS: [Parameter; 3] = [
    Parameter::LimitInput,
    Parameter::LimitThreshold,
    Parameter::LimitRelease,
];
const DELAY_PARAMETERS: [Parameter; 3] = [
    Parameter::DelayTime,
    Parameter::DelayFeedback,
    Parameter::DelayMix,
];

fn parameters(kind: NodeKind, node: &GraphNodeState) -> &'static [Parameter] {
    match kind {
        NodeKind::Source if matches!(node.source, Some(InputSelection::Tone { .. })) => {
            &[Parameter::Frequency]
        }
        NodeKind::Eq if node.builtin.eq().is_some() => &EQ_PARAMETERS,
        NodeKind::Gate if node.builtin.gate().is_some() => &GATE_PARAMETERS,
        NodeKind::Limiter if node.builtin.limiter().is_some() => &LIMITER_PARAMETERS,
        NodeKind::Delay if node.builtin.delay().is_some() => &DELAY_PARAMETERS,
        _ => &[],
    }
}

fn target(graph: &crate::service::GraphSnapshot, key: Key) -> Option<(NodeKind, &GraphNodeState)> {
    let node = graph.node(key.0)?;
    let kind = graph.kind_of(key.0)?;
    parameters(kind, node)
        .contains(&key.1)
        .then_some((kind, node))
}

pub(super) fn sync(shell: &mut StudioShell, window: &mut Window, cx: &mut Context<StudioShell>) {
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let consumed = shell.state.read(cx).snapshot.consumed_request_id;
    let editors = &mut shell.node_controls.precision;
    editors
        .inputs
        .retain(|key, _| target(&graph, *key).is_some());
    editors
        .baselines
        .retain(|key, _| editors.inputs.contains_key(key));
    editors
        .observed
        .retain(|key, _| editors.inputs.contains_key(key));
    editors
        .edited
        .retain(|key| editors.inputs.contains_key(key));
    editors
        .errors
        .retain(|key, _| editors.inputs.contains_key(key));
    editors
        .pending
        .retain(|key, request| *request > consumed && editors.inputs.contains_key(key));
    for node in &graph.nodes {
        let Some(kind) = graph.kind_of(node.id) else {
            continue;
        };
        for &parameter in parameters(kind, node) {
            let key = (node.id, parameter);
            // Shortest round-trippable text preserves legal precision values
            // even when a separate card dial is clamped to ergonomic travel.
            let text = parameter.value(kind, node).to_string();
            editors.baselines.entry(key).or_insert_with(|| text.clone());
            editors.observed.entry(key).or_insert_with(|| text.clone());
            let input = editors.inputs.entry(key).or_insert_with(|| {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
                cx.subscribe(&input, move |shell, input, event: &InputEvent, cx| {
                    if matches!(
                        event,
                        InputEvent::Change | InputEvent::Blur | InputEvent::PressEnter { .. }
                    ) {
                        commit(shell, input, key, event, cx);
                    }
                })
                .detach();
                input
            });
            let clean = editors.baselines.get(&key).map(String::as_str)
                == Some(input.read(cx).value().as_str());
            if clean
                && !editors.edited.contains(&key)
                && !input.read(cx).focus_handle(cx).is_focused(window)
                && !editors.errors.contains_key(&key)
                && !editors.pending.contains_key(&key)
            {
                if input.read(cx).value().as_str() != text {
                    input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                }
                editors.baselines.insert(key, text.clone());
                editors.observed.insert(key, text);
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
    // Load may reuse this NodeId before the old input entity is retired.
    if shell.pending_project.is_some() {
        return;
    }
    // Delayed Blur from a removed/rebound input must not act on its replacement.
    if shell
        .node_controls
        .precision
        .inputs
        .get(&key)
        .map(Entity::entity_id)
        != Some(input.entity_id())
    {
        return;
    }
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let Some((kind, node)) = target(&graph, key) else {
        return;
    };
    let text = input.read(cx).value().to_string();
    if matches!(event, InputEvent::Change) {
        // Change is also emitted for no-op replacements; only a transition
        // marks intent, including delete/retype back to the original baseline.
        let editors = &mut shell.node_controls.precision;
        if editors.observed.get(&key) != Some(&text) {
            editors.observed.insert(key, text);
            editors.edited.insert(key);
        }
        return;
    }
    // Untouched Enter and Blur preserve exact values and newer card intent.
    if !shell.node_controls.precision.edited.contains(&key)
        && shell.node_controls.precision.baselines.get(&key) == Some(&text)
    {
        shell.node_controls.precision.errors.remove(&key);
        cx.notify();
        return;
    }
    let value = parameter::parse(&text, key.1.unit());
    let edit = value.and_then(|value| key.1.edit_precision(value, kind, node));
    let (Some(value), Some(edit)) = (value, edit) else {
        shell
            .node_controls
            .precision
            .errors
            .insert(key, key.1.precision_error(node));
        cx.notify();
        return;
    };
    match shell.state.update(cx, |state, _| {
        state.try_dispatch(AppRequest::EditGraph(edit))
    }) {
        Ok(request) => {
            let editors = &mut shell.node_controls.precision;
            editors.baselines.insert(key, text.clone());
            editors.observed.insert(key, text);
            editors.edited.remove(&key);
            editors.errors.remove(&key);
            // Do not repaint a successfully submitted draft with the older
            // snapshot during the gap before the service consumes its request.
            editors.pending.insert(key, request);
            rotary::remember_input_admission(shell, key.0, key.1, value, request, cx);
        }
        Err(_) => {
            shell
                .node_controls
                .precision
                .errors
                .insert(key, t!("Edit could not be sent; try again").to_string());
        }
    }
    cx.notify();
}

pub(in crate::ui::window::graph) fn tone_inspector(shell: &StudioShell, id: NodeId) -> AnyElement {
    precision_field(
        shell,
        (id, Parameter::Frequency),
        [
            format!("tone-precision-{id:?}"),
            format!("tone-frequency-exact-{id:?}"),
            format!("tone-frequency-error-{id:?}"),
        ],
    )
}

pub(in crate::ui::window::graph) fn eq_inspector(shell: &StudioShell, id: NodeId) -> AnyElement {
    inspector_fields(shell, id, format!("eq-precision-{id:?}"), &EQ_PARAMETERS)
}

pub(in crate::ui::window::graph) fn gate_inspector(shell: &StudioShell, id: NodeId) -> AnyElement {
    inspector_fields(
        shell,
        id,
        format!("gate-precision-{id:?}"),
        &GATE_PARAMETERS,
    )
}

pub(in crate::ui::window::graph) fn limiter_inspector(
    shell: &StudioShell,
    id: NodeId,
) -> AnyElement {
    inspector_fields(
        shell,
        id,
        format!("limiter-precision-{id:?}"),
        &LIMITER_PARAMETERS,
    )
}

pub(in crate::ui::window::graph) fn delay_inspector(shell: &StudioShell, id: NodeId) -> AnyElement {
    inspector_fields(
        shell,
        id,
        format!("delay-precision-{id:?}"),
        &DELAY_PARAMETERS,
    )
}

fn inspector_fields(
    shell: &StudioShell,
    id: NodeId,
    root_id: String,
    parameters: &[Parameter],
) -> AnyElement {
    div()
        .id(root_id)
        .flex()
        .flex_col()
        .min_w_0()
        .w_full()
        .gap(px(8.))
        .children(parameters.iter().copied().map(|parameter| {
            precision_field(
                shell,
                (id, parameter),
                [
                    format!("inspector-precision-{id:?}-{parameter:?}"),
                    format!("inspector-precision-input-{id:?}-{parameter:?}"),
                    format!("inspector-precision-error-{id:?}-{parameter:?}"),
                ],
            )
        }))
        .gpui_test_observe()
        .into_any_element()
}

fn precision_field(shell: &StudioShell, key: Key, ids: [String; 3]) -> AnyElement {
    let [root_id, input_id, error_id] = ids;
    let parameter = key.1;
    let mut field = div()
        .id(root_id)
        .flex()
        .flex_col()
        .flex_none()
        .min_w_0()
        .w_full()
        .gap(px(4.))
        .child(note(parameter.label()));
    if let Some(input) = shell.node_controls.precision.inputs.get(&key) {
        field = field.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .w_full()
                .child(
                    div().flex_1().min_w_0().child(Styled::h(
                        component::input::Input::new(input)
                            .id(input_id)
                            .font_family(theme::FONT_MONO)
                            .text_size(px(12.))
                            .px(px(6.))
                            .py(px(0.))
                            .bg(theme::SURFACE_3),
                        px(28.),
                    )),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(theme::TEXT_MUTED)
                        .child(parameter.unit()),
                ),
        );
    }
    if let Some(error) = shell.node_controls.precision.errors.get(&key) {
        field = field.child(
            note(error.clone())
                .id(error_id)
                .text_color(theme::DANGER)
                .gpui_test_observe(),
        );
    }
    field.gpui_test_observe().into_any_element()
}
