//! Inspector-only precision fields; drafts never overwrite a newer queued edit.
use super::*;
use component::input::{InputEvent, InputState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::ui::window::graph::node_controls) enum Field {
    Fft,
    Hop,
}

#[derive(Default)]
pub(in crate::ui::window::graph::node_controls) struct Editors {
    pub(in crate::ui::window::graph::node_controls) inputs:
        BTreeMap<(NodeId, Field), Entity<InputState>>,
    baselines: BTreeMap<(NodeId, Field), String>,
    pub(in crate::ui::window::graph::node_controls) errors: BTreeMap<(NodeId, Field), String>,
    pending_fft: BTreeMap<NodeId, (crate::service::RequestId, usize)>,
}

impl Editors {
    pub(in crate::ui::window::graph::node_controls) fn owns_focus(
        &self,
        focused: &FocusHandle,
        cx: &App,
    ) -> bool {
        self.inputs
            .values()
            .any(|input| input.read(cx).focus_handle(cx) == *focused)
    }
}

pub(in crate::ui::window::graph::node_controls) fn sync(
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let consumed = shell.state.read(cx).snapshot.consumed_request_id;
    let editors = &mut shell.node_controls.spectrum_editors;
    editors.pending_fft.retain(|id, (request, _)| {
        *request > consumed && graph.kind_of(*id) == Some(NodeKind::SpectrumAnalyzer)
    });
    editors
        .inputs
        .retain(|(id, _), _| graph.kind_of(*id) == Some(NodeKind::SpectrumAnalyzer));
    editors
        .baselines
        .retain(|key, _| editors.inputs.contains_key(key));
    editors
        .errors
        .retain(|key, _| editors.inputs.contains_key(key));
    for node in &graph.nodes {
        let Some(settings) = node.builtin.spectrum() else {
            continue;
        };
        for (field, value) in [
            (Field::Fft, settings.fft_size),
            (Field::Hop, settings.hop_frames),
        ] {
            let key = (node.id, field);
            let text = value.to_string();
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
            if !input.read(cx).focus_handle(cx).is_focused(window)
                && clean
                && !editors.errors.contains_key(&key)
            {
                if input.read(cx).value().as_str() != text {
                    input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                }
                editors.baselines.insert(key, text);
            }
        }
    }
}

fn commit(
    shell: &mut StudioShell,
    input: Entity<InputState>,
    key: (NodeId, Field),
    cx: &mut Context<StudioShell>,
) {
    if shell
        .node_controls
        .spectrum_editors
        .inputs
        .get(&key)
        .map(Entity::entity_id)
        != Some(input.entity_id())
    {
        return;
    }
    let text = input.read(cx).value().to_string();
    if shell.node_controls.spectrum_editors.baselines.get(&key) == Some(&text) {
        shell.node_controls.spectrum_editors.errors.remove(&key);
        cx.notify();
        return;
    }
    let graph = &shell.state.read(cx).snapshot.graph;
    let Some(settings) = graph.node(key.0).and_then(|node| node.builtin.spectrum()) else {
        return;
    };
    let value = text.trim().parse::<usize>().ok();
    // An FFT edit may already be queued ahead of this Hop edit. Validate
    // against that ordered intent, then let the service merge the actual state.
    let fft = shell
        .node_controls
        .spectrum_editors
        .pending_fft
        .get(&key.0)
        .filter(|(request, _)| *request > shell.state.read(cx).snapshot.consumed_request_id)
        .map_or(settings.fft_size, |(_, fft)| *fft);
    let valid = value.is_some_and(|value| match key.1 {
        Field::Fft => (16..=8192).contains(&value) && value.is_power_of_two(),
        Field::Hop => (1..=fft).contains(&value),
    });
    if !valid {
        let error = match key.1 {
            Field::Fft => t!("Power of two from 16 to 8192").to_string(),
            Field::Hop => format!("1 .. {} {}", fft, t!("frames")),
        };
        shell
            .node_controls
            .spectrum_editors
            .errors
            .insert(key, error);
        cx.notify();
        return;
    }
    let value = value.expect("validated");
    let edit = match key.1 {
        Field::Fft => GraphEdit::SetSpectrumFft {
            node: key.0,
            fft_size: value,
        },
        Field::Hop => GraphEdit::SetSpectrumHop {
            node: key.0,
            hop_frames: value,
        },
    };
    if dispatch(shell, edit, cx) {
        shell
            .node_controls
            .spectrum_editors
            .baselines
            .insert(key, text);
        shell.node_controls.spectrum_editors.errors.remove(&key);
    } else {
        shell
            .node_controls
            .spectrum_editors
            .errors
            .insert(key, t!("Edit could not be sent; try again").to_string());
    }
    cx.notify();
}

pub(super) fn dispatch(
    shell: &mut StudioShell,
    edit: GraphEdit,
    cx: &mut Context<StudioShell>,
) -> bool {
    let fft = match &edit {
        GraphEdit::SetSpectrumFft { node, fft_size } => Some((*node, *fft_size)),
        _ => None,
    };
    match shell.state.update(cx, |state, _| {
        state.try_dispatch(AppRequest::EditGraph(edit))
    }) {
        Ok(request) => {
            if let Some((node, fft)) = fft {
                shell
                    .node_controls
                    .spectrum_editors
                    .pending_fft
                    .insert(node, (request, fft));
            }
            true
        }
        Err(_) => false,
    }
}

pub(super) fn render(shell: &StudioShell, id: NodeId) -> AnyElement {
    let mut fields = div().flex().gap(px(8.)).w_full();
    for (field, label) in [(Field::Fft, "FFT points"), (Field::Hop, "Hop frames")] {
        let key = (id, field);
        let Some(input) = shell.node_controls.spectrum_editors.inputs.get(&key) else {
            continue;
        };
        fields = fields.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(note(label))
                .child(Styled::h(
                    component::input::Input::new(input)
                        .id(format!("spectrum-exact-{id:?}-{field:?}"))
                        .text_size(px(11.))
                        .px(px(4.))
                        .py(px(0.))
                        .bg(theme::SURFACE_3),
                    px(26.),
                ))
                .when_some(
                    shell.node_controls.spectrum_editors.errors.get(&key),
                    |view, error| view.child(note(error.clone()).text_color(theme::DANGER)),
                ),
        );
    }
    fields.into_any_element()
}
