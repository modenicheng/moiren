use super::*;
use crate::service::{GraphEdit, GraphSnapshot, RequestId};
use crate::ui::editors::{
    SliderEdit, SliderSlot, balance_text, commit_mixer_balance, commit_mixer_gain,
    commit_node_gain, commit_node_pan, commit_send_gain, fader_rows, gain_db_text, sync_faders,
    sync_sliders, sync_value_inputs,
};
use crate::ui::widgets::fader::{send_gain, send_value};
use rust_i18n::t;
use std::collections::BTreeMap;

pub(crate) struct MixerPresentation {
    pub order: Vec<NodeId>,
    pub faders: BTreeMap<NodeId, SliderSlot>,
    pub sends: BTreeMap<EdgeId, SliderSlot>,
    /// One cached panel host per strip (design 17's per-item split): a
    /// publish during a fader drag rebuilds only the strip whose content
    /// changed. Created and retained by [`StudioShell::sync_mixer_controls`].
    pub strips: BTreeMap<NodeId, Entity<super::page::StripPanelHost>>,
    /// The strip's routing send list's scroll container. The I/O block
    /// renders live (outside the cached body), so this handle tracks its
    /// wheel and thumb drags for the overlay bar; retained like the strip
    /// hosts. The input side has no list — one row per strip — so it keeps
    /// no scroll handle.
    pub(super) row_scrolls: BTreeMap<NodeId, RowScrolls>,
    /// The inspector's channel-strip fader row gets its own slider slot per
    /// node: the kit's `SliderState` stores the pointer-mapping bounds of
    /// whichever view registered last, so the strip's vertical fader and the
    /// inspector's horizontal row cannot share one state — the row's
    /// horizontal bounds would own the strip's vertical drag and slam it to
    /// a rail (the selected-strip -inf lock). Two slots, one snapshot: both
    /// dispatch the same edits and the published graph reconciles them,
    /// exactly like the tray flyout's own slot set.
    pub inspector_faders: BTreeMap<NodeId, SliderSlot>,
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
    pub sends_collapsed: bool,
    /// The one open input dropdown (checkbox multi-select) and the input row
    /// rect it hangs from, in window pixels. `None` = every strip's dropdown
    /// is closed; opening one strip's closes the others. Dismissed by the
    /// occluding page (outside press / Escape), like the binding menu.
    pub input_popover: Option<InputPopover>,
    /// The dropdown list's scroll container (one dropdown open at a time).
    pub input_popover_scroll: ScrollHandle,
    /// The routing row the pointer is over, keyed by the strip and its edge
    /// (the same edge owns a send row on its source strip and an input row
    /// on its target strip, and they reveal independently). Drives the
    /// hover-revealed affordances that CSS pseudo-hover cannot reach across
    /// elements: a send row's dB readout and a single-input row's remove
    /// button both fade in on their row's hover.
    pub hovered_route: Option<(NodeId, EdgeId)>,
    /// The dock pane's current width, dragged on the divider from
    /// [`Self::DOCK_WIDTH`]. Session presentation state, like the collapse
    /// flags above: it is not persisted.
    pub dock_width: f32,
    /// The snapshot the controls last synced from: its graph revision and
    /// the newest request it consumed. The consumed request rides next to
    /// the revision because a slider's revision fence lifts on it — an edit
    /// the service refused leaves the revision alone, and the slider that
    /// sent it must still fall back to the published value.
    synced: Option<(u64, RequestId)>,
}

/// The mixer strip's open input dropdown: which strip it belongs to and the
/// strip's input-row rect it hangs from, in window pixels.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct InputPopover {
    pub node: NodeId,
    pub anchor: Bounds<Pixels>,
}

/// The send list's scroll container, keyed like the strip's other per-node
/// state.
#[derive(Default)]
pub(super) struct RowScrolls {
    pub send: ScrollHandle,
}

impl Default for MixerPresentation {
    fn default() -> Self {
        Self {
            dock_width: Self::DOCK_WIDTH,
            ..Self::zeroed()
        }
    }
}

impl MixerPresentation {
    /// The derived-zero baseline `Default` overrides only `dock_width` on:
    /// every field spelled once so the manual default stays obviously in
    /// sync with the struct.
    fn zeroed() -> Self {
        Self {
            order: Vec::new(),
            faders: BTreeMap::new(),
            sends: BTreeMap::new(),
            strips: BTreeMap::new(),
            row_scrolls: BTreeMap::new(),
            inspector_faders: BTreeMap::new(),
            send_inputs: BTreeMap::new(),
            editing_send: None,
            value_inputs: BTreeMap::new(),
            editing_value: None,
            balance_inputs: BTreeMap::new(),
            editing_balance: None,
            node_gain_inputs: BTreeMap::new(),
            editing_node_gain: None,
            node_pan_inputs: BTreeMap::new(),
            editing_node_pan: None,
            sends_collapsed: false,
            input_popover: None,
            input_popover_scroll: ScrollHandle::default(),
            hovered_route: None,
            dock_width: 0.,
            synced: None,
        }
    }

    /// Fixed strip width; strips are not user-resizable.
    pub(crate) const STRIP_WIDTH: f32 = 160.;
    /// The dock pane's default width; the divider drag resizes it live down
    /// to one strip and up to [`Self::DOCK_MAX_FRACTION`] of the strip row.
    const DOCK_WIDTH: f32 = 360.;
    /// The dock pane may never exceed this fraction of the strip row. The
    /// pane's `max_w` enforces it in layout; the divider drag clamps to the
    /// same fraction so the drawn border tracks the cursor instead of
    /// lagging behind the cap.
    pub(crate) const DOCK_MAX_FRACTION: f32 = 0.55;

    /// Every click-to-type text editor family in one walk: the send-gain
    /// editors plus the four value families (fader readout, balance label,
    /// Gain "Requested", Pan position). [`StudioShell::typing_focus`] and
    /// the inspector panel's nested observers both consume this single list,
    /// so a newly added family cannot be wired into one enumeration site and
    /// forgotten in the other.
    pub(crate) fn text_input_entities(
        &self,
    ) -> impl Iterator<Item = &Entity<component::input::InputState>> + '_ {
        self.send_inputs
            .values()
            .chain(self.value_inputs.values())
            .chain(self.balance_inputs.values())
            .chain(self.node_gain_inputs.values())
            .chain(self.node_pan_inputs.values())
    }
}

impl StudioShell {
    pub(crate) fn sync_mixer_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = &self.state.read(cx).snapshot;
        let synced = (
            snapshot.graph.candidate_revision,
            snapshot.consumed_request_id,
        );
        if self.mixer.presentation.synced == Some(synced) {
            return;
        }
        self.mixer.presentation.synced = Some(synced);
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.mixer
            .presentation
            .order
            .retain(|id| graph.node(*id).is_some());
        self.mixer
            .presentation
            .faders
            .retain(|id, _| graph.node(*id).is_some());
        self.mixer
            .presentation
            .sends
            .retain(|id, _| graph.graph.get_edge(*id).is_ok());
        self.mixer
            .presentation
            .send_inputs
            .retain(|id, _| graph.graph.get_edge(*id).is_ok());
        if self
            .mixer
            .presentation
            .editing_send
            .is_some_and(|id| graph.graph.get_edge(id).is_err())
        {
            self.mixer.presentation.editing_send = None;
        }
        // The input dropdown never outlives its strip (deleted or undone
        // away), and a hover whose edge vanished clears like the editors
        // above — the next pointer move re-arms it.
        if self
            .mixer
            .presentation
            .input_popover
            .is_some_and(|popover| graph.node(popover.node).is_none())
        {
            self.mixer.presentation.input_popover = None;
        }
        if self
            .mixer
            .presentation
            .hovered_route
            .is_some_and(|(_, id)| graph.graph.get_edge(id).is_err())
        {
            self.mixer.presentation.hovered_route = None;
        }
        // The click-to-type editor families drop their vanished members like
        // the send inputs; balance only exists for stereo nodes and the Gain
        // /Pan node fields only for their kind, mirroring the surfaces that
        // render them.
        let channels_of = |id: NodeId| graph.graph.get_node(id).ok().map(|node| node.channels());
        let kind_of = |id: NodeId| graph.kind_of(id);
        self.mixer
            .presentation
            .value_inputs
            .retain(|id, _| graph.node(*id).is_some());
        self.mixer
            .presentation
            .balance_inputs
            .retain(|id, _| graph.node(*id).is_some() && channels_of(*id) == Some(2));
        self.mixer
            .presentation
            .node_gain_inputs
            .retain(|id, _| graph.node(*id).is_some() && kind_of(*id) == Some(NodeKind::Gain));
        self.mixer
            .presentation
            .node_pan_inputs
            .retain(|id, _| graph.node(*id).is_some() && kind_of(*id) == Some(NodeKind::Pan));
        if self
            .mixer
            .presentation
            .editing_value
            .is_some_and(|id| graph.node(id).is_none())
        {
            self.mixer.presentation.editing_value = None;
        }
        if self
            .mixer
            .presentation
            .editing_balance
            .is_some_and(|id| graph.node(id).is_none() || channels_of(id) != Some(2))
        {
            self.mixer.presentation.editing_balance = None;
        }
        if self
            .mixer
            .presentation
            .editing_node_gain
            .is_some_and(|id| kind_of(id) != Some(NodeKind::Gain))
        {
            self.mixer.presentation.editing_node_gain = None;
        }
        if self
            .mixer
            .presentation
            .editing_node_pan
            .is_some_and(|id| kind_of(id) != Some(NodeKind::Pan))
        {
            self.mixer.presentation.editing_node_pan = None;
        }
        for node in &graph.nodes {
            if !self.mixer.presentation.order.contains(&node.id) {
                self.mixer.presentation.order.push(node.id);
            }
        }
        // One cached strip panel per node; the page splices them as siblings.
        // Created after the slider maps are synced so the panel's nested
        // observers find their entities on the first rebuild.
        self.mixer
            .presentation
            .strips
            .retain(|id, _| graph.node(*id).is_some());
        self.mixer
            .presentation
            .row_scrolls
            .retain(|id, _| graph.node(*id).is_some());
        for node in &graph.nodes {
            let id = node.id;
            self.mixer.presentation.strips.entry(id).or_insert_with(|| {
                let panel = super::page::StripPanel { id };
                let shell = cx.entity();
                cx.new(|child_cx| super::page::StripPanelHost::new(panel, shell, child_cx))
            });
            self.mixer.presentation.row_scrolls.entry(id).or_default();
        }
        sync_faders(
            window,
            cx,
            &self.state,
            &mut self.mixer.presentation.faders,
            fader_rows(&graph),
        );
        // The inspector's fader row mirrors the snapshot through its own
        // slots; the strip's slots stay untouched by the inspector's drags.
        self.mixer
            .presentation
            .inspector_faders
            .retain(|id, _| graph.node(*id).is_some());
        sync_faders(
            window,
            cx,
            &self.state,
            &mut self.mixer.presentation.inspector_faders,
            fader_rows(&graph),
        );
        // The send family rides the same editor sync as the NodeId-keyed
        // families below; only its commit is EdgeId-addressed.
        sync_value_inputs(
            window,
            cx,
            &mut self.mixer.presentation.send_inputs,
            self.mixer.presentation.editing_send.as_ref(),
            graph
                .graph
                .edges()
                .iter()
                .map(|edge| (edge.id(), gain_db_text(edge.params().gain)))
                .collect(),
            commit_send_gain,
        );
        let sends = graph
            .graph
            .edges()
            .iter()
            .map(|edge| {
                let id = edge.id();
                (
                    id,
                    send_value(edge.params().gain),
                    Box::new(move |value: f32, graph: &GraphSnapshot| {
                        graph.graph.get_edge(id).ok().map(|edge| {
                            AppRequest::EditGraph(GraphEdit::SetSend {
                                edge: id,
                                params: SendParams {
                                    gain: send_gain(value),
                                    ..*edge.params()
                                },
                            })
                        })
                    }) as SliderEdit,
                )
            })
            .collect();
        sync_sliders(
            window,
            cx,
            &self.state,
            &mut self.mixer.presentation.sends,
            sends,
        );
        // Click-to-type editors, one family per value kind. Their text is
        // the raw editable form ("−6", "R 25") rather than the styled
        // readout, and stands down for the member being edited.
        sync_value_inputs(
            window,
            cx,
            &mut self.mixer.presentation.value_inputs,
            self.mixer.presentation.editing_value.as_ref(),
            graph
                .nodes
                .iter()
                .map(|node| (node.id, gain_db_text(node.mixer.gain)))
                .collect(),
            commit_mixer_gain,
        );
        sync_value_inputs(
            window,
            cx,
            &mut self.mixer.presentation.balance_inputs,
            self.mixer.presentation.editing_balance.as_ref(),
            graph
                .nodes
                .iter()
                .filter(|node| channels_of(node.id) == Some(2))
                .map(|node| (node.id, balance_text(node.mixer.balance)))
                .collect(),
            commit_mixer_balance,
        );
        sync_value_inputs(
            window,
            cx,
            &mut self.mixer.presentation.node_gain_inputs,
            self.mixer.presentation.editing_node_gain.as_ref(),
            graph
                .nodes
                .iter()
                .filter(|node| kind_of(node.id) == Some(NodeKind::Gain))
                .map(|node| (node.id, gain_db_text(node.gain)))
                .collect(),
            commit_node_gain,
        );
        sync_value_inputs(
            window,
            cx,
            &mut self.mixer.presentation.node_pan_inputs,
            self.mixer.presentation.editing_node_pan.as_ref(),
            graph
                .nodes
                .iter()
                .filter(|node| kind_of(node.id) == Some(NodeKind::Pan))
                .map(|node| (node.id, format!("{:+.2}", node.pan)))
                .collect(),
            commit_node_pan,
        );
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
            .p(px(theme::SPACE_3))
            .bg(theme::controls::RAISED)
            .text_color(theme::TEXT)
            .child(t!("Move channel"))
    }
}
