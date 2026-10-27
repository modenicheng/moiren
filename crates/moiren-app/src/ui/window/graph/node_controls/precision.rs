//! Inspector-only numeric editors. The card never mounts these input entities.
use super::*;
use component::input::{InputEvent, InputState};

type Key = (NodeId, Parameter);

#[derive(Default)]
pub(super) struct Editors {
    pub(super) inputs: BTreeMap<Key, Entity<InputState>>,
    baselines: BTreeMap<Key, String>,
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

fn tone_node(graph: &crate::service::GraphSnapshot, id: NodeId) -> Option<&GraphNodeState> {
    let node = graph.node(id)?;
    (graph.kind_of(id) == Some(NodeKind::Source)
        && matches!(node.source, Some(InputSelection::Tone { .. })))
    .then_some(node)
}

pub(super) fn sync(shell: &mut StudioShell, window: &mut Window, cx: &mut Context<StudioShell>) {
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let consumed = shell.state.read(cx).snapshot.consumed_request_id;
    let editors = &mut shell.node_controls.precision;
    editors
        .inputs
        .retain(|(id, _), _| tone_node(&graph, *id).is_some());
    editors
        .baselines
        .retain(|key, _| editors.inputs.contains_key(key));
    editors
        .errors
        .retain(|key, _| editors.inputs.contains_key(key));
    editors
        .pending
        .retain(|key, request| *request > consumed && editors.inputs.contains_key(key));
    for node in &graph.nodes {
        if tone_node(&graph, node.id).is_none() {
            continue;
        }
        let key = (node.id, Parameter::Frequency);
        // Shortest round-trippable text preserves tiny and fractional legal
        // service values even when the separate dial is clamped to its travel.
        let text = Parameter::Frequency
            .value(NodeKind::Source, node)
            .to_string();
        editors.baselines.entry(key).or_insert_with(|| text.clone());
        let input = editors.inputs.entry(key).or_insert_with(|| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
            cx.subscribe(&input, move |shell, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    commit(shell, input, key, cx);
                }
            })
            .detach();
            input
        });
        let clean = editors.baselines.get(&key).map(String::as_str)
            == Some(input.read(cx).value().as_str());
        if clean
            && !input.read(cx).focus_handle(cx).is_focused(window)
            && !editors.errors.contains_key(&key)
            && !editors.pending.contains_key(&key)
        {
            if input.read(cx).value().as_str() != text {
                input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
            }
            editors.baselines.insert(key, text);
        }
    }
}

fn commit(
    shell: &mut StudioShell,
    input: Entity<InputState>,
    key: Key,
    cx: &mut Context<StudioShell>,
) {
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
    let Some(node) = tone_node(&graph, key.0) else {
        return;
    };
    let text = input.read(cx).value().to_string();
    // Compare against what this surface displayed, not the newest graph value:
    // an untouched Inspector Blur must never undo an intervening card edit.
    if shell.node_controls.precision.baselines.get(&key) == Some(&text) {
        shell.node_controls.precision.errors.remove(&key);
        cx.notify();
        return;
    }
    let edit = parameter::parse(&text, key.1.unit())
        .and_then(|value| key.1.edit_precision(value, NodeKind::Source, node));
    let Some(edit) = edit else {
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
            editors.baselines.insert(key, text);
            editors.errors.remove(&key);
            // Do not repaint a successfully submitted draft with the older
            // snapshot during the gap before the service consumes its request.
            editors.pending.insert(key, request);
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
    let key = (id, Parameter::Frequency);
    let mut field = div()
        .id(format!("tone-precision-{id:?}"))
        .flex()
        .flex_col()
        .flex_none()
        .min_w_0()
        .w_full()
        .gap(px(4.))
        .child(note(Parameter::Frequency.label()));
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
                            .id(format!("tone-frequency-exact-{id:?}"))
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
                        .child(Parameter::Frequency.unit()),
                ),
        );
    }
    if let Some(error) = shell.node_controls.precision.errors.get(&key) {
        field = field.child(
            note(error.clone())
                .id(format!("tone-frequency-error-{id:?}"))
                .text_color(theme::DANGER)
                .gpui_test_observe(),
        );
    }
    field.gpui_test_observe().into_any_element()
}
