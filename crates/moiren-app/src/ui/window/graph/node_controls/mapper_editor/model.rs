//! Bounded visible fields, retained drafts and latest-candidate cell intents.
use super::*;
use component::input::{InputEvent, InputState};
use moiren_core::graph::PortId;
use std::{collections::BTreeSet, rc::Rc};

pub(super) const PAGE_SIZE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Surface {
    Card,
    Inspector,
}

impl Surface {
    pub fn id(self, node: NodeId) -> String {
        let surface = match self {
            Self::Card => "card",
            Self::Inspector => "inspector",
        };
        format!("mapper-{surface}-{node:?}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub surface: Surface,
    pub node: NodeId,
    pub incarnation: u64,
    pub output: usize,
    pub input: usize,
}

/// A retained button callback must not match a newly loaded document whose
/// local node IDs and incarnation counter happen to start at the same values.
#[derive(Clone)]
pub(super) struct Target {
    node: NodeId,
    incarnation: u64,
    lifetime: Rc<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    input: PortId,
    output: PortId,
    inputs: usize,
    outputs: usize,
}

pub(super) struct Routing<'a> {
    layout: Layout,
    settings: &'a ChannelMapperSettings,
}

impl Routing<'_> {
    pub fn inputs(&self) -> usize {
        self.layout.inputs
    }
    pub fn outputs(&self) -> usize {
        self.layout.outputs
    }

    pub fn weight(&self, output: usize, input: usize) -> Option<f64> {
        if output >= self.outputs() || input >= self.inputs() {
            return None;
        }
        Some(self.settings.matrix.get(output).map_or_else(
            || {
                f64::from(u8::from(
                    self.settings.mapping.get(output).copied().unwrap_or(output) == input,
                ))
            },
            |row| row[input],
        ))
    }

    pub fn preset_allowed(&self, swap: bool) -> bool {
        self.inputs() == self.outputs() && (!swap || self.inputs() >= 2)
    }
}

pub(super) fn routing(graph: &GraphSnapshot, node: NodeId) -> Option<Routing<'_>> {
    static IDENTITY: ChannelMapperSettings = ChannelMapperSettings {
        mapping: Vec::new(),
        matrix: Vec::new(),
    };
    let topology = graph.graph.get_node(node).ok()?;
    if topology.kind() != NodeKind::ChannelMapper {
        return None;
    }
    let ([input], [output]) = (topology.inputs(), topology.outputs()) else {
        return None;
    };
    let settings = match &graph.node(node)?.builtin {
        BuiltinState::None => &IDENTITY,
        BuiltinState::ChannelMapper(settings) => settings,
        _ => return None,
    };
    settings
        .valid_layout(input.channels(), output.channels())
        .then_some(Routing {
            layout: Layout {
                input: input.id(),
                output: output.id(),
                inputs: input.channels(),
                outputs: output.channels(),
            },
            settings,
        })
}

pub(super) fn page(inputs: usize, outputs: usize, page: (usize, usize)) -> (usize, usize) {
    (
        page.0.min(outputs.saturating_sub(1) / PAGE_SIZE),
        page.1.min(inputs.saturating_sub(1) / PAGE_SIZE),
    )
}

#[derive(Default)]
pub(in crate::ui::window::graph::node_controls) struct Editors {
    pub(super) inputs: BTreeMap<Key, Entity<InputState>>,
    pub(super) baselines: BTreeMap<Key, String>,
    pub(super) errors: BTreeMap<Key, String>,
    pub(super) pages: BTreeMap<(Surface, NodeId), (usize, usize)>,
    pub(super) action_errors: BTreeMap<NodeId, String>,
    inspector_open: BTreeMap<NodeId, bool>,
    lifetime: Rc<()>,
    pending: BTreeMap<Key, RequestId>,
    presets: BTreeMap<NodeId, (RequestId, bool)>,
    layouts: BTreeMap<NodeId, (Layout, u64)>,
    active: BTreeSet<Key>,
    next_incarnation: u64,
    fence: Option<RequestId>,
}

impl Editors {
    pub(in crate::ui::window::graph::node_controls) fn owns_focus(
        &self,
        focused: &FocusHandle,
        cx: &App,
    ) -> bool {
        self.inputs.iter().any(|(key, input)| {
            self.active.contains(key) && input.read(cx).focus_handle(cx) == *focused
        })
    }

    pub(super) fn incarnation(&self, node: NodeId) -> Option<u64> {
        self.layouts.get(&node).map(|(_, incarnation)| *incarnation)
    }

    pub(super) fn target(&self, node: NodeId) -> Option<Target> {
        Some(Target {
            node,
            incarnation: self.incarnation(node)?,
            lifetime: self.lifetime.clone(),
        })
    }

    pub(super) fn disclosure_open(&self, node: NodeId) -> bool {
        self.inspector_open.get(&node).copied().unwrap_or(true)
    }

    fn clear_node(&mut self, node: NodeId) {
        self.inputs.retain(|key, _| key.node != node);
        self.baselines.retain(|key, _| key.node != node);
        self.errors.retain(|key, _| key.node != node);
        self.pending.retain(|key, _| key.node != node);
        self.active.retain(|key| key.node != node);
        self.action_errors.remove(&node);
    }

    pub(super) fn retire_until(&mut self, request: RequestId) {
        let next_incarnation = self.next_incarnation;
        *self = Self {
            next_incarnation,
            fence: Some(self.fence.map_or(request, |old| old.max(request))),
            ..Default::default()
        };
    }
}

fn blocked(shell: &StudioShell, cx: &App) -> bool {
    shell.pending_project.is_some()
        || shell
            .node_controls
            .mapper
            .fence
            .is_some_and(|request| request > shell.state.read(cx).snapshot.consumed_request_id)
}

fn current(shell: &StudioShell, node: NodeId, incarnation: u64, cx: &App) -> bool {
    if blocked(shell, cx) {
        return false;
    }
    let state = &shell.node_controls.mapper;
    let Some((layout, current)) = state.layouts.get(&node) else {
        return false;
    };
    *current == incarnation
        && routing(&shell.state.read(cx).snapshot.graph, node)
            .is_some_and(|routing| routing.layout == *layout)
}

fn current_target(shell: &StudioShell, target: &Target, cx: &App) -> bool {
    Rc::ptr_eq(&shell.node_controls.mapper.lifetime, &target.lifetime)
        && current(shell, target.node, target.incarnation, cx)
}

fn dirty(state: &Editors, key: Key, text: &str) -> bool {
    state.baselines.get(&key).map(String::as_str) != Some(text)
}

fn commit(
    shell: &mut StudioShell,
    input: &Entity<InputState>,
    key: Key,
    cx: &mut Context<StudioShell>,
) {
    if !current(shell, key.node, key.incarnation, cx)
        || shell
            .node_controls
            .mapper
            .inputs
            .get(&key)
            .map(Entity::entity_id)
            != Some(input.entity_id())
    {
        return;
    }
    let text = input.read(cx).value().to_string();
    if !dirty(&shell.node_controls.mapper, key, &text) {
        shell.node_controls.mapper.errors.remove(&key);
        cx.notify();
        return;
    }
    let Some(weight) = super::super::parameter::parse(&text, "") else {
        shell
            .node_controls
            .mapper
            .errors
            .insert(key, "Enter a finite routing weight".into());
        cx.notify();
        return;
    };
    let result = shell.state.update(cx, |state, _| {
        state.try_dispatch(AppRequest::EditGraph(GraphEdit::SetMapperWeight {
            node: key.node,
            input: key.input,
            output: key.output,
            weight,
        }))
    });
    let state = &mut shell.node_controls.mapper;
    match result {
        Ok(request) => {
            state.baselines.insert(key, text);
            state.errors.remove(&key);
            state.pending.insert(key, request);
        }
        Err(_) => {
            state
                .errors
                .insert(key, "Edit could not be sent; try again".into());
        }
    }
    cx.notify();
}

pub(super) fn preset(
    shell: &mut StudioShell,
    target: &Target,
    swap: bool,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    let node = target.node;
    if !current_target(shell, target, cx)
        || !routing(&shell.state.read(cx).snapshot.graph, node)
            .is_some_and(|routing| routing.preset_allowed(swap))
    {
        return;
    }
    match shell.state.update(cx, |state, _| {
        state.try_dispatch(AppRequest::EditGraph(GraphEdit::MapperPreset {
            node,
            swap,
        }))
    }) {
        Ok(request) => {
            // Presets intentionally replace all cells, including unsubmitted
            // drafts. Retiring entities fences any already scheduled Blur.
            shell.node_controls.mapper.clear_node(node);
            shell
                .node_controls
                .mapper
                .presets
                .insert(node, (request, swap));
            sync(shell, window, cx);
        }
        Err(_) => {
            shell
                .node_controls
                .mapper
                .action_errors
                .insert(node, "Edit could not be sent; try again".into());
        }
    }
    cx.notify();
}

pub(super) fn navigate(
    shell: &mut StudioShell,
    target: &Target,
    surface: Surface,
    horizontal: bool,
    delta: isize,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    if !current_target(shell, target, cx) {
        return;
    }
    let page = shell
        .node_controls
        .mapper
        .pages
        .entry((surface, target.node))
        .or_default();
    let axis = if horizontal { &mut page.1 } else { &mut page.0 };
    *axis = axis.saturating_add_signed(delta);
    sync(shell, window, cx);
    cx.notify();
}

pub(super) fn toggle_disclosure(
    shell: &mut StudioShell,
    target: &Target,
    open: bool,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    if !current_target(shell, target, cx) {
        return;
    }
    shell
        .node_controls
        .mapper
        .inspector_open
        .insert(target.node, open);
    // Flush departure now, even when unmounting never produces InputEvent::Blur.
    sync(shell, window, cx);
    cx.notify();
}

fn surface_visible(shell: &StudioShell, surface: Surface, node: NodeId) -> bool {
    if shell.settings_open {
        return false;
    }
    match surface {
        Surface::Card => {
            shell.presentation.page == pages::Page::Graph
                && shell.camera.zoom >= 0.85
                && !shell.hand_tool
                && shell.node_drag.is_none()
                && shell.connection_drag.is_none()
                && shell.pan_grab.is_none()
                && shell.selection_drag.is_none()
        }
        Surface::Inspector => {
            shell.inspector_open
                && shell.selected_node == Some(node)
                && matches!(
                    shell.presentation.page,
                    pages::Page::Graph | pages::Page::Mixer
                )
                && shell.node_controls.mapper.disclosure_open(node)
        }
    }
}

pub(in crate::ui::window::graph::node_controls) fn sync(
    shell: &mut StudioShell,
    window: &mut Window,
    cx: &mut Context<StudioShell>,
) {
    if let Some((request, _, _)) = &shell.pending_project {
        shell.node_controls.mapper.retire_until(*request);
        return;
    }
    if blocked(shell, cx) {
        return;
    }
    let graph = shell.state.read(cx).snapshot.graph.clone();
    let consumed = shell.state.read(cx).snapshot.consumed_request_id;
    let state = &mut shell.node_controls.mapper;
    state.fence = None;
    state.pending.retain(|_, request| *request > consumed);
    state.presets.retain(|_, (request, _)| *request > consumed);
    let layouts: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter_map(|node| routing(&graph, node.id).map(|routing| (node.id, routing.layout)))
        .collect();
    let replaced: Vec<_> = state
        .layouts
        .iter()
        .filter(|(id, (layout, _))| layouts.get(id) != Some(layout))
        .map(|(id, _)| *id)
        .collect();
    for node in replaced {
        state.clear_node(node);
        state.layouts.remove(&node);
        state.presets.remove(&node);
        state.inspector_open.remove(&node);
        state.pages.retain(|(_, id), _| *id != node);
    }
    for (node, layout) in layouts {
        if !state.layouts.contains_key(&node) {
            state.next_incarnation = state
                .next_incarnation
                .checked_add(1)
                .expect("Mapper editor identity exhausted");
            state.layouts.insert(node, (layout, state.next_incarnation));
        }
    }
    let mut fields = Vec::new();
    let mut active = BTreeSet::new();
    for node in &graph.nodes {
        let Some(routing) = routing(&graph, node.id) else {
            continue;
        };
        let incarnation = shell
            .node_controls
            .mapper
            .incarnation(node.id)
            .expect("valid layout has an identity");
        for surface in [Surface::Card, Surface::Inspector] {
            if !surface_visible(shell, surface, node.id) {
                continue;
            }
            let state = &mut shell.node_controls.mapper;
            let page = state.pages.entry((surface, node.id)).or_default();
            *page = self::page(routing.inputs(), routing.outputs(), *page);
            for output in page.0 * PAGE_SIZE..((page.0 + 1) * PAGE_SIZE).min(routing.outputs()) {
                for input in page.1 * PAGE_SIZE..((page.1 + 1) * PAGE_SIZE).min(routing.inputs()) {
                    let key = Key {
                        surface,
                        node: node.id,
                        incarnation,
                        output,
                        input,
                    };
                    let value = state.presets.get(&node.id).map_or_else(
                        || {
                            routing
                                .weight(output, input)
                                .expect("visible cell is in bounds")
                        },
                        |(_, swap)| {
                            f64::from(u8::from(
                                input
                                    == if *swap && output < 2 {
                                        1 - output
                                    } else {
                                        output
                                    },
                            ))
                        },
                    );
                    active.insert(key);
                    fields.push((key, value));
                }
            }
        }
    }
    let outgoing: Vec<_> = shell
        .node_controls
        .mapper
        .active
        .difference(&active)
        .filter_map(|key| {
            let state = &shell.node_controls.mapper;
            let input = state.inputs.get(key)?;
            dirty(state, *key, input.read(cx).value().as_str()).then(|| (*key, input.clone()))
        })
        .collect();
    // Unmounting a pane does not guarantee Blur. Flush only on departure, never
    // every render of an invalid or busy retained draft.
    for (key, input) in outgoing {
        commit(shell, &input, key, cx);
    }
    let state = &mut shell.node_controls.mapper;
    state.inputs.retain(|key, input| {
        active.contains(key)
            || state.pending.contains_key(key)
            || state.errors.contains_key(key)
            || state.baselines.get(key).map(String::as_str) != Some(input.read(cx).value().as_str())
    });
    state
        .baselines
        .retain(|key, _| state.inputs.contains_key(key));
    state.errors.retain(|key, _| state.inputs.contains_key(key));
    state.active = active;
    for (key, value) in fields {
        let text = value.to_string();
        state.baselines.entry(key).or_insert_with(|| text.clone());
        let input = state.inputs.entry(key).or_insert_with(|| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
            cx.subscribe(&input, move |shell, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    commit(shell, &input, key, cx);
                }
            })
            .detach();
            input
        });
        if !input.read(cx).focus_handle(cx).is_focused(window)
            && state.baselines.get(&key).map(String::as_str)
                == Some(input.read(cx).value().as_str())
            && !state.errors.contains_key(&key)
            && !state.pending.contains_key(&key)
        {
            if input.read(cx).value().as_str() != text {
                input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
            }
            state.baselines.insert(key, text);
        }
    }
}
