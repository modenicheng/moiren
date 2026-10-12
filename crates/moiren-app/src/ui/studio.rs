use super::{Backend, GraphNodeData, GraphWireData, LibraryEntry};
use crate::session_graph::SessionGraph;
use moiren_core::graph::{EdgeId, NodeId, NodeKind};
use slint::{Color, Model, VecModel};
use std::{collections::HashMap, rc::Rc};

const BLUE: Color = Color::from_rgb_u8(0x86, 0xb9, 0xec);
const VIOLET: Color = Color::from_rgb_u8(0xb1, 0xa0, 0xdf);
const NODE_WIDTH: f32 = 186.0;
const OUTPUT_WIDTH: f32 = 226.0;
const NODE_HEIGHT: f32 = 105.0;
const PORT_Y: f32 = 83.0;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
enum Selection {
    #[default]
    None,
    Node(NodeId),
    Wire(EdgeId),
}

pub(super) struct StudioState {
    session: SessionGraph,
    positions: HashMap<NodeId, (f32, f32)>,
    selection: Selection,
    pub gain: f32,
    pub pan: f32,
}

impl Default for StudioState {
    fn default() -> Self {
        let session = SessionGraph::new().expect("fixed stereo session topology");
        let positions = [session.input, session.gain, session.pan, session.output]
            .into_iter()
            .enumerate()
            .map(|(index, node)| (node, (24.0 + index as f32 * 206.0, 160.0)))
            .collect();
        Self {
            session,
            positions,
            selection: Selection::None,
            gain: 0.01,
            pan: 0.0,
        }
    }
}

#[derive(Default, Clone)]
pub(super) struct StudioModels {
    nodes: Rc<VecModel<GraphNodeData>>,
    wires: Rc<VecModel<GraphWireData>>,
}

pub(super) struct Projection {
    nodes: Vec<GraphNodeData>,
    wires: Vec<GraphWireData>,
    selected_node: String,
    selected_wire: String,
    title: String,
    source: String,
    destination: String,
    pub focus: Option<(f32, f32)>,
    pub gain_label: String,
    pub pan_label: String,
}

impl StudioState {
    fn node_for_key(&self, key: &str) -> Option<NodeId> {
        self.session
            .graph
            .nodes()
            .iter()
            .find(|node| format!("{:?}", node.id()) == key)
            .map(|node| node.id())
    }

    pub fn select_node(&mut self, key: &str) -> bool {
        if let Some(node) = self.node_for_key(key) {
            self.selection = Selection::Node(node);
            true
        } else {
            false
        }
    }

    pub fn select_wire(&mut self, key: &str) {
        if let Some(edge) = self
            .session
            .graph
            .edges()
            .iter()
            .find(|edge| format!("{:?}", edge.id()) == key)
        {
            self.selection = Selection::Wire(edge.id());
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = Selection::None;
    }

    pub fn move_node(&mut self, key: &str, x: f32, y: f32) {
        if x.is_finite()
            && y.is_finite()
            && let Some(node) = self.node_for_key(key)
        {
            self.positions
                .insert(node, (x.clamp(0.0, 1900.0), y.clamp(60.0, 1050.0)));
        }
    }

    pub fn select_library(&mut self, name: &str) {
        let node = match name {
            "Gain" => self.session.gain,
            "Pan" => self.session.pan,
            "Physical output" => self.session.output,
            _ => self.session.input,
        };
        self.selection = Selection::Node(node);
    }

    pub fn set_parameters(&mut self, gain: f32, pan: f32) -> bool {
        if !gain.is_finite()
            || !pan.is_finite()
            || !(0.0..=1.0).contains(&gain)
            || !(-1.0..=1.0).contains(&pan)
        {
            return false;
        }
        self.gain = gain;
        self.pan = pan;
        true
    }

    pub fn projection(&self, source: &str, source_kind: &str, destination: &str) -> Projection {
        let name = |node| {
            if node == self.session.input {
                source.to_owned()
            } else if node == self.session.output {
                destination.to_owned()
            } else if node == self.session.gain {
                "Gain".to_owned()
            } else {
                "Pan".to_owned()
            }
        };
        let gain_label = if self.gain == 0.0 {
            "−∞ dB".to_owned()
        } else {
            format!("{:.1} dB", 20.0 * self.gain.log10())
        };
        let pan_label = if self.pan == 0.0 {
            "Center".to_owned()
        } else {
            format!(
                "{} {:.0}%",
                if self.pan < 0.0 { "L" } else { "R" },
                self.pan.abs() * 100.0
            )
        };
        let nodes = self
            .session
            .graph
            .nodes()
            .iter()
            .map(|node| {
                let (x, y) = self.positions[&node.id()];
                let (type_label, detail, value, tint) = match node.kind() {
                    NodeKind::Source => (
                        source_kind,
                        match source_kind {
                            "TEST TONE" => "440 Hz · 10 seconds",
                            "APPLICATION AUDIO" => "Process loopback · stereo",
                            _ => "WASAPI capture · stereo",
                        },
                        String::new(),
                        BLUE,
                    ),
                    NodeKind::Sink => (
                        "PHYSICAL OUTPUT",
                        "WASAPI shared · stereo",
                        String::new(),
                        BLUE,
                    ),
                    NodeKind::Gain => {
                        ("GAIN", "Linear amplitude · 0…1", gain_label.clone(), VIOLET)
                    }
                    _ => ("PAN", "Stereo balance", pan_label.clone(), VIOLET),
                };
                let level = match node.kind() {
                    NodeKind::Gain => self.gain,
                    NodeKind::Pan => (self.pan + 1.0) / 2.0,
                    _ => 0.0,
                };
                GraphNodeData {
                    id: format!("{:?}", node.id()).into(),
                    name: name(node.id()).into(),
                    type_label: type_label.into(),
                    icon: match node.kind() {
                        NodeKind::Source => match source_kind {
                            "TEST TONE" => "radio",
                            "APPLICATION AUDIO" => "app-window",
                            _ => "mic",
                        },
                        NodeKind::Sink => "headphones",
                        _ => "sliders",
                    }
                    .into(),
                    detail: detail.into(),
                    value: value.into(),
                    has_input: !node.inputs().is_empty(),
                    has_output: !node.outputs().is_empty(),
                    input_label: "L / R".into(),
                    output_label: "L / R".into(),
                    tint_color: tint,
                    wide: node.kind() == NodeKind::Sink,
                    level,
                    x,
                    y,
                    card_width: if node.kind() == NodeKind::Sink {
                        OUTPUT_WIDTH
                    } else {
                        NODE_WIDTH
                    },
                    card_height: NODE_HEIGHT,
                    port_y: PORT_Y,
                    ..Default::default()
                }
            })
            .collect();
        let wires = self
            .session
            .graph
            .edges()
            .iter()
            .map(|edge| {
                let (x1, y1) = self.positions[&edge.src()];
                let (x2, y2) = self.positions[&edge.dst()];
                GraphWireData {
                    id: format!("{:?}", edge.id()).into(),
                    tint_color: VIOLET,
                    x1: x1 + NODE_WIDTH,
                    y1: y1 + PORT_Y,
                    x2,
                    y2: y2 + PORT_Y,
                }
            })
            .collect();
        let (selected_node, selected_wire, title, from, to, focus) = match self.selection {
            Selection::None => (
                String::new(),
                String::new(),
                "Session mix".to_owned(),
                source.to_owned(),
                destination.to_owned(),
                None,
            ),
            Selection::Node(node) => (
                format!("{node:?}"),
                String::new(),
                name(node),
                source.to_owned(),
                destination.to_owned(),
                Some(self.positions[&node]),
            ),
            Selection::Wire(id) => {
                let edge = self
                    .session
                    .graph
                    .get_edge(id)
                    .expect("selected edge exists");
                (
                    String::new(),
                    format!("{id:?}"),
                    format!("{} → {}", name(edge.src()), name(edge.dst())),
                    name(edge.src()),
                    name(edge.dst()),
                    Some(self.positions[&edge.src()]),
                )
            }
        };
        Projection {
            nodes,
            wires,
            selected_node,
            selected_wire,
            title,
            source: from,
            destination: to,
            focus,
            gain_label,
            pan_label,
        }
    }
}

impl StudioModels {
    pub fn install(&self, backend: &Backend) {
        backend.set_graph_nodes(self.nodes.clone().into());
        backend.set_graph_wires(self.wires.clone().into());
        let entry = |icon: &str, name: &str, detail: &str| LibraryEntry {
            icon: icon.into(),
            name: name.into(),
            subtitle: "Select session node".into(),
            detail: detail.into(),
        };
        backend.set_library_sources(
            Rc::new(VecModel::from(vec![
                entry("mic", "Physical input", "WASAPI capture"),
                entry("app-window", "Application audio", "Process loopback"),
                entry("radio", "Test tone", "440 Hz · finite playback"),
            ]))
            .into(),
        );
        backend.set_library_processing(
            Rc::new(VecModel::from(vec![
                entry("sliders", "Gain", "Session amplitude"),
                entry("sliders", "Pan", "Stereo balance"),
            ]))
            .into(),
        );
        backend.set_library_destinations(
            Rc::new(VecModel::from(vec![entry(
                "headphones",
                "Physical output",
                "WASAPI render",
            )]))
            .into(),
        );
    }

    pub fn publish(&self, backend: &Backend, projection: Projection) {
        let left = projection
            .nodes
            .iter()
            .map(|node| node.x)
            .fold(f32::INFINITY, f32::min);
        let top = projection
            .nodes
            .iter()
            .map(|node| node.y)
            .fold(f32::INFINITY, f32::min);
        let right = projection
            .nodes
            .iter()
            .map(|node| node.x + node.card_width)
            .fold(0.0, f32::max);
        let bottom = projection
            .nodes
            .iter()
            .map(|node| node.y + node.card_height)
            .fold(0.0, f32::max);
        backend.set_graph_left(left);
        backend.set_graph_top(top);
        backend.set_graph_width(right - left);
        backend.set_graph_height(bottom - top);
        backend.set_graph_input_index(
            projection
                .nodes
                .iter()
                .position(|node| !node.has_input && node.has_output)
                .map_or(-1, |index| index as i32),
        );
        backend.set_graph_output_index(
            projection
                .nodes
                .iter()
                .position(|node| node.has_input && !node.has_output)
                .map_or(-1, |index| index as i32),
        );
        update_rows(&self.nodes, projection.nodes);
        update_rows(&self.wires, projection.wires);
        backend.set_selected_node(projection.selected_node.into());
        backend.set_selected_wire(projection.selected_wire.into());
        backend.set_inspector_title(projection.title.into());
        backend.set_inspector_source(projection.source.into());
        backend.set_inspector_destination(projection.destination.into());
        backend.set_gain_label(projection.gain_label.into());
        backend.set_pan_label(projection.pan_label.into());
        backend.set_inspector_mapping("1 → 1, 2 → 2".into());
        backend.set_inspector_subtitle("Session Gain / Pan".into());
        backend.set_inspector_state("Candidate · stereo".into());
        backend.set_inspector_notes(
            "Gain and Pan control the whole session. Device changes apply on the next Start."
                .into(),
        );
    }
}

fn update_rows<Row: Clone + PartialEq + 'static>(model: &VecModel<Row>, rows: Vec<Row>) {
    for (index, row) in rows.iter().enumerate() {
        if index >= model.row_count() {
            model.push(row.clone());
        } else if model.row_data(index).as_ref() != Some(row) {
            model.set_row_data(index, row.clone());
        }
    }
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_matches_session_ports_and_tracks_drag() {
        let mut studio = StudioState::default();
        let before = studio.projection("Input", "INPUT", "Output");
        assert_eq!((before.nodes.len(), before.wires.len()), (4, 3));
        let key = before.nodes[1].id.to_string();
        studio.move_node(&key, 320.0, 450.0);
        let after = studio.projection("Input", "INPUT", "Output");
        assert_eq!((after.nodes[1].x, after.nodes[1].y), (320.0, 450.0));
        assert_eq!((after.wires[0].x2, after.wires[0].y2), (320.0, 533.0));
        assert_eq!((after.wires[1].x1, after.wires[1].y1), (506.0, 533.0));
        assert_eq!(studio.session.graph.edges().len(), 3);
        studio.move_node(&key, f32::NAN, 300.0);
        assert_eq!(studio.positions[&studio.session.gain], (320.0, 450.0));
    }

    #[test]
    fn selection_is_exclusive_and_unknown_ids_are_ignored() {
        let mut studio = StudioState::default();
        let initial = studio.projection("Input", "INPUT", "Output");
        assert!(studio.select_node(&initial.nodes[1].id));
        assert!(!studio.select_node("missing"));
        studio.select_wire(&initial.wires[1].id);
        let selected = studio.projection("Input", "INPUT", "Output");
        assert!(selected.selected_node.is_empty());
        assert_eq!(selected.title, "Gain → Pan");
        studio.clear_selection();
        assert_eq!(studio.selection, Selection::None);
    }

    #[test]
    fn parameter_validation_preserves_last_candidate() {
        let mut studio = StudioState::default();
        assert!(studio.set_parameters(0.5, -1.0));
        for (gain, pan) in [(2.0, 0.0), (0.5, f32::NAN), (0.5, 2.0)] {
            assert!(!studio.set_parameters(gain, pan));
            assert_eq!((studio.gain, studio.pan), (0.5, -1.0));
        }
        let projection = studio.projection("Input", "INPUT", "Output");
        assert_eq!(projection.pan_label, "L 100%");
        assert_eq!(projection.gain_label, "-6.0 dB");
    }

    #[test]
    fn parameter_card_positions_follow_candidate_values() {
        let mut studio = StudioState::default();
        assert!(studio.set_parameters(0.25, -0.5));
        let projection = studio.projection("Input", "INPUT", "Output");
        assert_eq!(projection.nodes[1].level, 0.25);
        assert_eq!(projection.nodes[2].level, 0.25);
        assert_eq!(projection.nodes[3].level, 0.0);
    }
}
