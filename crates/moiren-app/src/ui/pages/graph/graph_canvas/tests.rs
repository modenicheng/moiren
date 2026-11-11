use std::sync::Arc;

use super::{
    Camera, CanvasFrame, CanvasWire, CanvasWorld, HitTarget, MeterLayout, NodeLayout, WireCurve,
    node,
};
use crate::ui::theme;
use gpui_kit::{Bounds, Path, Pixels, point, px, size};

/// Real node and port identities for wire fixtures; the IDs are opaque
/// outside moiren-core, so harvest them from one throwaway graph (its id
/// counter makes the two nodes' ids distinct).
fn id_fixture() -> (
    (moiren_core::graph::NodeId, moiren_core::graph::PortId),
    (moiren_core::graph::NodeId, moiren_core::graph::PortId),
) {
    use moiren_core::graph::NodeKind;
    let mut graph = moiren_core::graph::LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    let source_port = graph.get_node(source).unwrap().outputs()[0].id();
    let sink_port = graph.get_node(sink).unwrap().inputs()[0].id();
    ((source, source_port), (sink, sink_port))
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
}

fn assert_position(actual: (f32, f32), expected: (f32, f32)) {
    assert_close(actual.0, expected.0);
    assert_close(actual.1, expected.1);
}

fn viewport() -> Bounds<Pixels> {
    Bounds {
        origin: point(px(83.), px(117.)),
        size: size(px(1000.), px(700.)),
    }
}

fn world() -> Arc<CanvasWorld> {
    let mut source = node("source", "Input", "Device")
        .at(-200., 40.)
        .ports(false, true);
    let mut sink = node("sink", "Output", "Device")
        .at(420., 340.)
        .ports(true, false);
    source.input_count = 0;
    sink.input_count = 1;
    let ((src, _src_port), (dst, dst_port)) = id_fixture();
    sink.input_count = 1;
    sink.input_ports = vec![super::CanvasPort {
        id: dst_port,
        channels: 2,
    }];
    let wire = CanvasWire {
        id: "edge".into(),
        source: source.id.clone(),
        destination: sink.id.clone(),
        src,
        dst,
        dst_port,
        from: NodeLayout::new(&source).output.unwrap(),
        to: NodeLayout::new(&sink).inputs[0],
        tint: None,
    };
    Arc::new(CanvasWorld::new(vec![source, sink], vec![wire]))
}

fn triangle_area(path: &Path<Pixels>) -> f32 {
    path.vertices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|vertices| {
            let first = vertices[0].xy_position;
            let second = vertices[1].xy_position;
            let third = vertices[2].xy_position;
            ((f32::from(second.x - first.x) * f32::from(third.y - first.y)
                - f32::from(second.y - first.y) * f32::from(third.x - first.x))
            .abs())
                / 2.
        })
        .sum()
}

#[test]
fn curved_wire_mesh_does_not_fill_the_curve_interior() {
    let path = WireCurve::new((200., 600.), (220., 100.))
        .project(Camera::default())
        .stroke(1.6, (0., 0.))
        .unwrap();
    let area = triangle_area(&path);
    assert!(
        area < 1200.,
        "a thin 500px wire must not fill a fan: {area}"
    );
}

#[test]
fn normal_manual_zoom_obeys_the_same_limits_for_direct_and_relative_steps() {
    let mut camera = Camera::default();
    camera.zoom_at(20., (300., 200.));
    assert_eq!(camera.zoom, 2.);
    camera.zoom_at(0., (300., 200.));
    assert_eq!(camera.zoom, 0.25);
    camera.zoom_by(0.5, (300., 200.));
    assert_eq!(camera.zoom, 0.25);
    camera.zoom_by(100., (300., 200.));
    assert_eq!(camera.zoom, 2.);
}

#[test]
fn relative_zoom_does_not_reverse_direction_for_legacy_saved_camera() {
    let mut camera = Camera {
        zoom: 4.,
        adjusted: true,
        ..Camera::default()
    };
    let anchor = (418.25, 271.75);
    let world = camera.canvas_to_world(anchor);
    camera.zoom_by(1.1, anchor);
    assert_eq!(camera.zoom, 4.);
    assert_position(camera.world_to_canvas(world), anchor);
    camera.zoom_by(1. / 1.1, anchor);
    assert_close(camera.zoom, 4. / 1.1);
    assert_position(camera.world_to_canvas(world), anchor);
}

#[test]
fn zoom_after_large_graph_fit_preserves_direction_and_anchor() {
    let mut camera = Camera::default();
    camera.fit(
        viewport().size,
        Bounds {
            origin: point(-10000., -9000.),
            size: size(20000., 18000.),
        },
    );
    let anchor = (418.25, 271.75);
    let world = camera.canvas_to_world(anchor);
    let fitted_zoom = camera.zoom;
    camera.zoom_by(1. / 1.1, anchor);
    assert!(camera.zoom < fitted_zoom);
    assert_close(camera.zoom, fitted_zoom / 1.1);
    assert_position(camera.world_to_canvas(world), anchor);
    camera.zoom_by(1.1, anchor);
    assert_close(camera.zoom, fitted_zoom);
    assert_position(camera.world_to_canvas(world), anchor);
    assert!(camera.adjusted);
}

#[test]
fn large_graph_fit_camera_round_trips_through_project_storage() {
    use crate::project::{
        GraphDocument, PROJECT_VERSION, ProjectDocument, ProjectLayout, load_project, save_project,
    };

    let mut camera = Camera::default();
    camera.fit(
        viewport().size,
        Bounds {
            origin: point(-10000., -9000.),
            size: size(20000., 18000.),
        },
    );
    let mut document = ProjectDocument {
        exports: Vec::new(),
        processing: None,
        version: PROJECT_VERSION,
        name: "Large graph layout".into(),
        session: None,
        graph: Some(GraphDocument::default()),
        layout: ProjectLayout {
            camera_x: camera.pan_x,
            camera_y: camera.pan_y,
            zoom: camera.zoom,
            ..ProjectLayout::default()
        },
    };
    let directory = std::env::temp_dir().join(format!(
        "moiren-graph-layout-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("large-graph.moiren");
    let result = save_project(&path, &document).and_then(|()| load_project(&path));
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(result.unwrap(), document);
    for zoom in [f32::EPSILON, 0.05, 0.25, 2., 4.] {
        document.layout.zoom = zoom;
        assert!(document.layout.validate().is_ok(), "zoom {zoom}");
    }
    for zoom in [0., -0.1, f32::EPSILON / 2., 4.1, f32::NAN, f32::INFINITY] {
        document.layout.zoom = zoom;
        assert!(document.layout.validate().is_err(), "zoom {zoom}");
    }
}

#[test]
fn camera_round_trips_world_points_and_drag_deltas_at_fractional_zoom() {
    for zoom in [0.25, 0.89, 1., 2.] {
        let camera = Camera {
            pan_x: -137.5,
            pan_y: 221.25,
            zoom,
            adjusted: true,
        };
        for world in [(-450., 230.), (0., 0.), (1024.5, -781.25)] {
            let screen = camera.world_to_canvas(world);
            assert_position(camera.canvas_to_world(screen), world);
            let delta = camera.canvas_delta_to_world((37., -19.));
            assert_position(
                camera.world_to_canvas((world.0 + delta.0, world.1 + delta.1)),
                (screen.0 + 37., screen.1 - 19.),
            );
        }
    }
}

#[test]
fn zoom_preserves_pointer_and_toolbar_center_at_clamped_limits() {
    let anchor = (418.25, 271.75);
    let mut camera = Camera {
        pan_x: -120.,
        pan_y: 54.,
        zoom: 0.89,
        adjusted: true,
    };
    let world = camera.canvas_to_world(anchor);
    for zoom in [0.25, 1.13, 20., 0.] {
        camera.zoom_at(zoom, anchor);
        assert_position(camera.world_to_canvas(world), anchor);
    }
    let center = camera.canvas_to_world((500., 350.));
    camera.zoom_around_center(Some((1000., 700.)), 1.1);
    assert_position(camera.world_to_canvas(center), (500., 350.));
}

#[test]
fn nonfinite_input_cannot_poison_camera_geometry() {
    let mut camera = Camera::default();
    for factor in [f32::NAN, f32::INFINITY, -1., 0.] {
        camera.zoom_by(factor, (100., 100.));
    }
    camera.zoom_at(f32::NAN, (100., 100.));
    camera.pan_by(f32::INFINITY, 0.);
    assert_eq!(camera, Camera::default());
}

#[test]
fn fit_uses_actual_graph_bounds_without_including_world_origin() {
    let mut source = node("source", "Input", "Device").at(5000., 6000.);
    source.has_output = true;
    let graph = CanvasWorld::new(vec![source], Vec::new());
    let mut camera = Camera::default();
    camera.fit(viewport().size, graph.graph_bounds);
    let projected = camera.project_rect(graph.graph_bounds);
    assert!(camera.zoom >= 0.99);
    assert!(projected.origin.x >= 24. && projected.origin.y >= 72.);
    assert!(projected.origin.x + projected.size.width <= 976.);
    assert!(projected.origin.y + projected.size.height <= 676.);
}

#[test]
fn fit_can_show_graphs_larger_than_manual_zoom_range_and_tiny_viewports() {
    for canvas in [size(px(1000.), px(700.)), size(px(40.), px(30.))] {
        let graph = Bounds {
            origin: point(-10000., -9000.),
            size: size(20000., 18000.),
        };
        let mut camera = Camera::default();
        camera.fit(canvas, graph);
        assert!(camera.zoom < 0.25 && camera.zoom > 0.);
        let projected = camera.project_rect(graph);
        assert!(projected.origin.x >= 0. && projected.origin.y >= 0.);
        assert!(projected.origin.x + projected.size.width <= f32::from(canvas.width) + 0.001);
        assert!(projected.origin.y + projected.size.height <= f32::from(canvas.height) + 0.001);
    }
}

#[test]
fn layout_scales_fonts_ports_and_card_together_without_font_floors() {
    let mut bus = node("bus", "Bus", "3 inputs")
        .at(180., -60.)
        .ports(true, true);
    bus.input_count = 3;
    let world = NodeLayout::new(&bus);
    assert!(world.card.size.height > 105.);
    for zoom in [0.25, 0.89, 2.] {
        let camera = Camera {
            pan_x: 71.,
            pan_y: -24.,
            zoom,
            adjusted: true,
        };
        let layout = world.project(camera);
        assert_close(layout.title.font_size, world.title.font_size * zoom);
        assert_close(layout.detail.font_size, world.detail.font_size * zoom);
        assert_close(
            layout.input_hints[0].font_size,
            world.input_hints[0].font_size * zoom,
        );
        assert_close(layout.card.size.height, world.card.size.height * zoom);
        for (index, anchor) in layout.inputs.iter().enumerate() {
            assert_position(*anchor, camera.world_to_canvas(world.inputs[index]));
            assert!(layout.card.contains(&point(anchor.0, anchor.1)));
        }
        assert_position(
            layout.output.unwrap(),
            camera.world_to_canvas(world.output.unwrap()),
        );
    }
}

#[test]
fn node_layout_matches_design18_header_body_and_port_measurements() {
    let card = node("input", "Physical Input", "48kHz L/R").ports(false, true);
    let layout = NodeLayout::new(&card);
    // Header + one-line body: the ports settle at the base rail and the
    // card closes one PORT_BOTTOM below them.
    assert_eq!(layout.card.size, size(270., 94.));
    assert_eq!(
        layout.icon,
        Bounds {
            origin: point(11., 7.),
            size: size(16., 16.)
        }
    );
    assert_position(
        (layout.title.bounds.origin.x, layout.title.bounds.origin.y),
        (33., 8.),
    );
    assert_close(layout.title.font_size, 12.);
    assert_close(layout.detail.font_size, 10.);
    assert_close(layout.detail.bounds.origin.y, theme::card::BODY_TOP);
    let MeterLayout::Horizontal { bounds: meter } = layout.meter else {
        panic!("shell bodies use the horizontal meter");
    };
    assert_close(
        meter.origin.y,
        theme::card::BODY_TOP + card.body_height + theme::card::METER_CHANNEL_GAP,
    );
    assert_close(
        meter.size.width,
        270. - theme::card::RAIL_W - 2. * theme::card::PADDING,
    );
    assert_eq!(
        layout.rail,
        Bounds {
            origin: point(0., 0.),
            size: size(theme::card::RAIL_W, 94.)
        }
    );
    assert_position(layout.output.unwrap(), (270., 80.));
    assert_eq!(
        layout.badge,
        Bounds {
            origin: point(228., 8.),
            size: size(14., 14.)
        }
    );
    assert_eq!(
        layout.expand,
        Bounds {
            origin: point(246., 8.),
            size: size(14., 14.)
        }
    );
    // A taller body pushes the port rail below the meter strip.
    let mut tall = node("eq", "EQ", "1 kHz").ports(true, true);
    tall.body_height = 96.;
    tall.meter_vertical = true;
    let layout = NodeLayout::new(&tall);
    let MeterLayout::Vertical { .. } = layout.meter else {
        panic!("curve bodies use the vertical meter");
    };
    let meter_total =
        theme::card::METER_CHANNELS as f32 * theme::card::METER_H + theme::card::METER_CHANNEL_GAP;
    assert_close(
        layout.inputs[0].1,
        (theme::card::BODY_TOP + 96. + meter_total + theme::card::PORT_BOTTOM).max(80.),
    );
}

#[test]
fn ports_ride_the_card_wherever_it_sits_in_the_world() {
    // The port rail is an offset from the card top, never from the world
    // origin: a card deep in the world keeps its dots on its own edges. The
    // old formula added `node.y` twice, which the clamp hid at y <= 0.
    for (x, y) in [(0., 0.), (320., 540.), (-410., 1280.)] {
        let mut card = node("gain", "Gain", "L/R").at(x, y).ports(true, true);
        card.body_height = theme::card::KNOB_BODY_H;
        let layout = NodeLayout::new(&card);
        let expected = y + card.port_offset();
        assert_position(layout.inputs[0], (x, expected));
        assert_position(layout.output.unwrap(), (x + card.width(), expected));
        assert_close(
            layout.card.origin.y + layout.card.size.height,
            expected + theme::card::PORT_BOTTOM,
        );
        assert_close(
            layout.input_hints[0].bounds.origin.y,
            expected - theme::card::HINT_SIZE / 2. - 1.,
        );
        assert!(layout.card.contains(&point(x + 1., expected)));
    }
}

#[test]
fn adaptive_wire_geometry_tracks_the_projected_cubic_at_every_zoom() {
    for (from, to) in [
        ((0., 0.), (12., 700.)),
        ((500., 200.), (-200., 20.)),
        ((0., 0.), (800., 0.)),
        ((200., 500.), (200., 0.)),
    ] {
        let curve = WireCurve::new(from, to);
        for zoom in [0.25, 0.89, 2.] {
            let camera = Camera {
                zoom,
                adjusted: true,
                ..Camera::default()
            };
            let geometry = curve.project(camera);
            assert_position(geometry.points[0], camera.world_to_canvas(from));
            assert_position(*geometry.points.last().unwrap(), camera.world_to_canvas(to));
            for sample in 0..=100 {
                let fraction = sample as f32 / 100.;
                let inverse = 1. - fraction;
                let weights = [
                    inverse.powi(3),
                    3. * inverse.powi(2) * fraction,
                    3. * inverse * fraction.powi(2),
                    fraction.powi(3),
                ];
                let exact =
                    curve
                        .controls
                        .iter()
                        .zip(weights)
                        .fold((0., 0.), |sum, (position, weight)| {
                            (sum.0 + position.0 * weight, sum.1 + position.1 * weight)
                        });
                assert!(geometry.distance(camera.world_to_canvas(exact)) <= 0.21);
            }
            let path = geometry.stroke(1.6, (0., 0.)).unwrap();
            let length = geometry
                .points
                .windows(2)
                .map(|segment| (segment[1].0 - segment[0].0).hypot(segment[1].1 - segment[0].1))
                .sum::<f32>();
            assert!((triangle_area(&path) / length - 1.6).abs() < 0.04);
        }
    }
}

#[test]
fn degenerate_wire_is_finite_and_does_not_create_a_phantom_hit() {
    let geometry = WireCurve::new((10., 10.), (10., 10.)).project(Camera::default());
    assert_eq!(geometry.points, vec![(10., 10.)]);
    assert!(geometry.stroke(1.6, (0., 0.)).unwrap().vertices.is_empty());
    assert!(!geometry.distance((10., 10.)).is_finite());
}

#[test]
fn first_frame_fit_and_hits_use_the_same_camera_and_window_origin() {
    let frame = CanvasFrame::new(world(), Camera::default(), viewport());
    let node = &frame.nodes[0];
    let local = (
        node.card.origin.x + node.card.size.width / 2.,
        node.card.origin.y + 12.,
    );
    let position = (
        f32::from(frame.bounds.origin.x) + local.0,
        f32::from(frame.bounds.origin.y) + local.1,
    );
    assert_eq!(frame.hit_test(position), Some(HitTarget::Node("source")));
    let geometry = &frame.wires[0];
    let center = geometry.points[geometry.points.len() / 2];
    let window = (
        center.0 + f32::from(frame.bounds.origin.x),
        center.1 + f32::from(frame.bounds.origin.y),
    );
    assert_eq!(frame.hit_test(window), Some(HitTarget::Wire("edge")));
    assert_eq!(
        frame.hit_test((window.0, f32::from(frame.bounds.origin.y) - 1.)),
        None
    );
}

#[test]
fn wire_hits_keep_a_screen_pixel_tolerance_at_fractional_zoom() {
    for zoom in [0.25, 0.89, 2.] {
        let ((src, _src_port), (dst, dst_port)) = id_fixture();
        let wire = CanvasWire {
            id: "wire".into(),
            source: "source".into(),
            destination: "sink".into(),
            src,
            dst,
            dst_port,
            from: (0., 0.),
            to: (200., 0.),
            tint: None,
        };
        let world = Arc::new(CanvasWorld::new(Vec::new(), vec![wire]));
        let frame = CanvasFrame::new(
            world,
            Camera {
                zoom,
                pan_x: 20.,
                pan_y: 100.,
                adjusted: true,
            },
            viewport(),
        );
        let center = frame.camera.world_to_canvas((100., 0.));
        let window = (
            center.0 + f32::from(frame.bounds.origin.x),
            center.1 + f32::from(frame.bounds.origin.y),
        );
        assert_eq!(
            frame.hit_test((window.0, window.1 + 5.9)),
            Some(HitTarget::Wire("wire"))
        );
        assert_eq!(frame.hit_test((window.0, window.1 + 6.1)), None);
    }
}

#[test]
fn overview_uses_camera_visible_world_and_stays_centered_inside_its_viewport() {
    for pan in [(-1400., 900.), (0., 0.), (700., -500.)] {
        let frame = CanvasFrame::new(
            world(),
            Camera {
                zoom: 0.89,
                pan_x: pan.0,
                pan_y: pan.1,
                adjusted: true,
            },
            viewport(),
        );
        let overview = frame.overview(size(148., 64.));
        let visible = frame.camera.visible_world(frame.viewport().size);
        let projected = overview.camera.project_rect(visible);
        assert_eq!(projected, overview.viewport);
        let content = frame.world.graph_bounds.union(&visible);
        let projected_content = overview.camera.project_rect(content);
        assert_close(
            projected_content.origin.x + projected_content.size.width / 2.,
            74.,
        );
        assert_close(
            projected_content.origin.y + projected_content.size.height / 2.,
            32.,
        );
        assert!(projected.origin.x >= 2.9 && projected.origin.y >= 2.9);
        assert!(projected.origin.x + projected.size.width <= 145.1);
        assert!(projected.origin.y + projected.size.height <= 61.1);
    }
}

#[test]
fn graph_bounds_include_reversed_wire_handle_excursions() {
    let ((src, _src_port), (dst, dst_port)) = id_fixture();
    let wire = CanvasWire {
        id: "wire".into(),
        source: "source".into(),
        destination: "sink".into(),
        src,
        dst,
        dst_port,
        from: (500., 0.),
        to: (0., 100.),
        tint: None,
    };
    let world = CanvasWorld::new(Vec::new(), vec![wire]);
    assert!(world.graph_bounds.origin.x < -190.);
    assert!(world.graph_bounds.origin.x + world.graph_bounds.size.width > 690.);
}

#[test]
fn hit_testing_respects_node_z_order_and_clips_offscreen_cards() {
    let first = node("first", "Gain", "0 dB").at(-80., 30.);
    let second = node("second", "Balance", "0.00").at(-80., 30.);
    let world = Arc::new(CanvasWorld::new(vec![first, second], Vec::new()));
    let camera = Camera {
        adjusted: true,
        ..Camera::default()
    };
    let frame = CanvasFrame::new(world, camera, viewport());
    let origin = (
        f32::from(frame.bounds.origin.x),
        f32::from(frame.bounds.origin.y),
    );
    assert_eq!(
        frame.hit_test((origin.0 + 20., origin.1 + 50.)),
        Some(HitTarget::Node("second"))
    );
    assert_eq!(frame.hit_test((origin.0 - 20., origin.1 + 50.)), None);
}

#[test]
fn painted_wire_endpoints_match_projected_node_ports_after_drag() {
    let mut world = Arc::try_unwrap(world()).ok().unwrap();
    world.nodes[0].x += 73.5;
    world.nodes[0].y -= 39.;
    world.wires[0].from = NodeLayout::new(&world.nodes[0]).output.unwrap();
    for zoom in [0.25, 0.89, 2.] {
        let frame = CanvasFrame::new(
            Arc::new(CanvasWorld::new(world.nodes.clone(), world.wires.clone())),
            Camera {
                zoom,
                adjusted: true,
                ..Camera::default()
            },
            viewport(),
        );
        assert_position(frame.wires[0].points[0], frame.nodes[0].output.unwrap());
        assert_position(
            *frame.wires[0].points.last().unwrap(),
            frame.nodes[1].inputs[0],
        );
    }
}

#[test]
fn fit_keeps_resolving_on_resize_until_camera_is_manually_adjusted() {
    let first = CanvasFrame::new(world(), Camera::default(), viewport());
    let smaller = Bounds {
        size: size(px(500.), px(350.)),
        ..viewport()
    };
    let resized = CanvasFrame::new(world(), first.camera, smaller);
    assert!(!resized.camera.adjusted);
    assert!(resized.camera.zoom < first.camera.zoom);
    let mut camera = resized.camera;
    camera.pan_by(30., -10.);
    let manual = CanvasFrame::new(world(), camera, viewport());
    assert_eq!(manual.camera, camera);
}

/// source -> gain -> pan with real identities and geometry, mirroring how
/// [`CanvasWorld::positioned`] projects a service snapshot, for connection
/// candidate tests.
struct ChainWorld {
    world: Arc<CanvasWorld>,
    source_out: super::PortTarget,
    gain_in: super::PortTarget,
    gain_out: super::PortTarget,
    pan_in: super::PortTarget,
    gain: moiren_core::graph::NodeId,
    pan: moiren_core::graph::NodeId,
}

fn chain_world() -> ChainWorld {
    use super::{CanvasPort, PortDirection, PortTarget};
    use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let gain = graph.create_node(NodeKind::Gain, 2).unwrap();
    let pan = graph.create_node(NodeKind::Pan, 2).unwrap();
    let source_out = graph.get_node(source).unwrap().outputs()[0].id();
    let gain_in = graph.get_node(gain).unwrap().inputs()[0].id();
    let gain_out = graph.get_node(gain).unwrap().outputs()[0].id();
    let pan_in = graph.get_node(pan).unwrap().inputs()[0].id();
    graph
        .connect(source_out, gain_in, SendParams::default())
        .unwrap();
    graph
        .connect(gain_out, pan_in, SendParams::default())
        .unwrap();
    let mut nodes = Vec::new();
    for (id, at) in [
        (source, (-320., 40.)),
        (gain, (0., 40.)),
        (pan, (320., 40.)),
    ] {
        let logical = graph.get_node(id).unwrap();
        let mut card = node(&format!("{id:?}"), "n", "d")
            .at(at.0, at.1)
            .ports(!logical.inputs().is_empty(), !logical.outputs().is_empty());
        card.node_id = Some(id);
        card.input_ports = logical
            .inputs()
            .iter()
            .map(|port| CanvasPort {
                id: port.id(),
                channels: port.channels(),
            })
            .collect();
        card.output_port = logical.outputs().first().map(|port| CanvasPort {
            id: port.id(),
            channels: port.channels(),
        });
        nodes.push(card);
    }
    let wires = graph
        .edges()
        .iter()
        .map(|edge| {
            let a = nodes
                .iter()
                .find(|n| n.node_id == Some(edge.src()))
                .unwrap();
            let b = nodes
                .iter()
                .find(|n| n.node_id == Some(edge.dst()))
                .unwrap();
            CanvasWire {
                id: format!("{:?}", edge.id()),
                source: a.id.clone(),
                destination: b.id.clone(),
                src: edge.src(),
                dst: edge.dst(),
                dst_port: edge.dst_port(),
                from: NodeLayout::new(a).output.unwrap(),
                to: NodeLayout::new(b).inputs[0],
                tint: None,
            }
        })
        .collect();
    ChainWorld {
        world: Arc::new(CanvasWorld::new(nodes, wires)),
        source_out: PortTarget {
            id: source_out,
            direction: PortDirection::Output,
        },
        gain_in: PortTarget {
            id: gain_in,
            direction: PortDirection::Input,
        },
        gain_out: PortTarget {
            id: gain_out,
            direction: PortDirection::Output,
        },
        pan_in: PortTarget {
            id: pan_in,
            direction: PortDirection::Input,
        },
        gain,
        pan,
    }
}

#[test]
fn would_cycle_mirrors_the_core_chain_rule() {
    let chain = chain_world();
    // pan -> gain closes the loop over the existing gain -> pan; the
    // forward direction is that edge itself.
    assert!(chain.world.would_cycle(chain.pan, chain.gain));
    assert!(!chain.world.would_cycle(chain.gain, chain.pan));
}

#[test]
fn resolve_candidate_covers_displacement_cycles_and_directions() {
    use super::CandidateStatus;
    let chain = chain_world();
    let gain = chain.gain;
    let pan = chain.pan;
    // Occupied input: displaces whichever end was dragged.
    assert_eq!(
        chain
            .world
            .resolve_candidate(chain.source_out, chain.pan_in, pan, false)
            .status,
        CandidateStatus::Displacing
    );
    assert_eq!(
        chain
            .world
            .resolve_candidate(chain.pan_in, chain.source_out, gain, false)
            .status,
        CandidateStatus::Displacing
    );
    // pan's output back into gain closes pan -> gain -> pan.
    assert_eq!(
        chain
            .world
            .resolve_candidate(chain.gain_out, chain.gain_in, gain, false)
            .status,
        CandidateStatus::Cycles
    );
    // Same-direction drops are incompatible, not connectable.
    assert_eq!(
        chain
            .world
            .resolve_candidate(chain.source_out, chain.gain_out, gain, false)
            .status,
        CandidateStatus::Incompatible
    );
    // A free compatible input connects cleanly: disconnect gain -> pan
    // first, then pan's input is free.
    let world = Arc::new(CanvasWorld::new(
        chain.world.nodes.clone(),
        chain
            .world
            .wires
            .iter()
            .filter(|wire| wire.dst != pan)
            .cloned()
            .collect(),
    ));
    assert_eq!(
        world
            .resolve_candidate(chain.source_out, chain.pan_in, pan, false)
            .status,
        CandidateStatus::Valid
    );
}

#[test]
fn connection_candidate_resolves_body_drops_to_the_nearest_input() {
    let chain = chain_world();
    let world = Arc::new(CanvasWorld::new(chain.world.nodes.clone(), Vec::new()));
    let frame = CanvasFrame::new(
        world,
        Camera {
            adjusted: true,
            ..Default::default()
        },
        Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(1200.), px(600.)),
        },
    );
    // Release at the gain card's center: no port within 8 px, so the body
    // resolves to the single compatible input.
    let gain_card = frame.nodes[1].card;
    let center = (
        gain_card.origin.x + gain_card.size.width / 2.,
        gain_card.origin.y + gain_card.size.height / 2.,
    );
    let candidate = frame
        .connection_candidate(center, chain.source_out)
        .unwrap();
    assert!(candidate.from_body);
    assert_eq!(candidate.port.id, chain.gain_in.id);
    // A release over the source card (no inputs) resolves nothing.
    let source_card = frame.nodes[0].card;
    let source_center = (
        source_card.origin.x + source_card.size.width / 2.,
        source_card.origin.y + source_card.size.height / 2.,
    );
    assert!(
        frame
            .connection_candidate(source_center, chain.source_out)
            .is_none()
    );
}

/// The spectrum analyzer is the one card that is not a strip: a wide 4:3
/// card as tall as a strip is wide, its meter running vertically down the
/// right edge, and the plot keeping clear of both meter channels.
#[test]
fn spectrum_card_is_four_by_three_with_a_vertical_meter_and_a_clear_plot() {
    let mut card = node("spectrum", "Spectrum analyzer", "FFT 1024 · hop 512");
    card.spectrum = true;
    card.body_height = theme::card::SPECTRUM_BODY_H;
    card.meter_vertical = true;
    card.input_count = 1;
    card.has_output = true;
    card.x = 120.;
    card.y = -40.;
    assert_close(theme::card::SPECTRUM_BODY_H, 198.);
    assert_close(card.width(), theme::card::SPECTRUM_W);
    assert_close(card.height(), theme::NODE_WIDTH);
    assert_close(card.width() / card.height(), 4. / 3.);
    // A strip of the same kind of body stays strip-wide.
    assert_close(node("gain", "Gain", "L/R").width(), theme::NODE_WIDTH);
    let layout = NodeLayout::new(&card);
    assert_close(layout.card.size.width, theme::card::SPECTRUM_W);
    // The output port rides the wide card's right edge.
    assert_close(
        layout.output.expect("output port").0,
        card.x + theme::card::SPECTRUM_W,
    );
    let plot = layout.spectrum.expect("the analyzer lays out a plot");
    assert!(layout.curve.is_none(), "a plot is not a curve thumbnail");
    let MeterLayout::Vertical { bounds: meter } = layout.meter else {
        panic!("the analyzer's meter runs vertically");
    };
    let stride = theme::card::METER_H + theme::card::METER_CHANNEL_GAP;
    assert_close(plot.origin.x, layout.body.origin.x);
    assert_close(plot.origin.y, layout.body.origin.y);
    assert_close(plot.size.height, theme::card::SPECTRUM_BODY_H);
    assert!(
        plot.origin.x + plot.size.width <= meter.origin.x - stride,
        "the plot stops short of the second meter channel"
    );
    assert_close(meter.size.height, plot.size.height);
    // The plot projects with the rest of the card.
    let camera = Camera {
        pan_x: 10.,
        pan_y: 20.,
        zoom: 0.5,
        adjusted: true,
    };
    let projected = layout.project(camera);
    let plot = projected.spectrum.expect("projected plot");
    assert_close(plot.size.height, theme::card::SPECTRUM_BODY_H * 0.5);
    assert_close(plot.origin.x, camera.project_rect(layout.body).origin.x);
}

#[test]
fn badge_hits_on_a_faulted_card_and_falls_through_to_the_card_without_one() {
    // Faulted: the badge rect resolves to HitTarget::Badge ahead of the
    // card check, so the popover anchor hangs off the same rect.
    let mut source = node("source", "Input", "Device")
        .at(-200., 40.)
        .ports(false, true);
    source.faulted = true;
    source.faults.push(super::CardFault {
        cause: "cause".into(),
        hint: None,
    });
    let world = Arc::new(CanvasWorld::new(vec![source], Vec::new()));
    let frame = CanvasFrame::new(world, Camera::default(), viewport());
    let badge = frame.nodes[0].badge;
    let position = (
        f32::from(frame.bounds.origin.x) + badge.origin.x + badge.size.width / 2.,
        f32::from(frame.bounds.origin.y) + badge.origin.y + badge.size.height / 2.,
    );
    assert_eq!(
        frame.hit_test(position),
        Some(HitTarget::Badge { node: "source" })
    );

    // Not faulted: no badge is painted, so the same rect falls through to
    // the card — gate and paint stay in agreement.
    let plain = node("sink", "Output", "Device")
        .at(-200., 40.)
        .ports(false, true);
    let world = Arc::new(CanvasWorld::new(vec![plain], Vec::new()));
    let frame = CanvasFrame::new(world, Camera::default(), viewport());
    let badge = frame.nodes[0].badge;
    let position = (
        f32::from(frame.bounds.origin.x) + badge.origin.x + badge.size.width / 2.,
        f32::from(frame.bounds.origin.y) + badge.origin.y + badge.size.height / 2.,
    );
    assert_eq!(frame.hit_test(position), Some(HitTarget::Node("sink")));
}

/// The hot layer paints a dragged card from its cached layout shifted by the
/// drag offset. Projection is affine, so the shifted layout must equal the
/// projection of the node at its moved world position — this is the
/// zero-visual-change guarantee for the drag paint (audit A-1). The fixture
/// carries every layout-bearing body kind so a field added to `NodeLayout`
/// and missed by `translated` fails here.
#[test]
fn translated_layout_matches_the_projection_of_the_moved_node() {
    // A knob card and a binding-chip card cover every layout-bearing body
    // kind, with ports; the fixture mirrors what `CanvasWorld::positioned`
    // builds for a gain and a sink.
    let mut gain = node("gain", "Gain", "L/R").at(40., 60.).ports(true, true);
    gain.knobs.push(super::CardKnob {
        control: super::KnobControl::GainLevel,
        label: "Gain".into(),
        value_text: "0.0 dB".into(),
        position: 0.5,
        origin: 0.,
    });
    gain.body_height = theme::card::KNOB_BODY_H;
    let mut sink = node("sink", "Output", "Device")
        .at(480., 90.)
        .ports(true, false);
    sink.binding = Some(super::BindingChip {
        label: "Speakers".into(),
        icon: "speaker",
        unbound: false,
    });
    sink.body_height = theme::card::CHIP_BODY_H;
    let world = Arc::new(CanvasWorld::new(vec![gain, sink], Vec::new()));
    let camera = Camera {
        pan_x: 30.,
        pan_y: 70.,
        zoom: 0.8,
        adjusted: true,
    };
    let (dx, dy) = (23., -17.);
    for node in &world.nodes {
        let moved = {
            let mut moved = node.clone();
            // The shift runs in canvas space; the moved node carries the
            // same offset divided back into world units.
            moved.x += dx / camera.zoom;
            moved.y += dy / camera.zoom;
            NodeLayout::new(&moved).project(camera)
        };
        let shifted = NodeLayout::new(node).project(camera).translated(dx, dy);
        let close = |a: Bounds<f32>, b: Bounds<f32>| {
            assert_close(a.origin.x, b.origin.x);
            assert_close(a.origin.y, b.origin.y);
            assert_close(a.size.width, b.size.width);
            assert_close(a.size.height, b.size.height);
        };
        close(shifted.card, moved.card);
        close(shifted.rail, moved.rail);
        close(shifted.icon, moved.icon);
        close(shifted.badge, moved.badge);
        close(shifted.expand, moved.expand);
        close(shifted.body, moved.body);
        close(shifted.title.bounds, moved.title.bounds);
        assert_close(shifted.title.font_size, moved.title.font_size);
        close(shifted.detail.bounds, moved.detail.bounds);
        match (&shifted.meter, &moved.meter) {
            (MeterLayout::Horizontal { bounds: a }, MeterLayout::Horizontal { bounds: b })
            | (MeterLayout::Vertical { bounds: a }, MeterLayout::Vertical { bounds: b }) => {
                close(*a, *b)
            }
            _ => panic!("meter axis changed"),
        }
        assert_eq!(shifted.knobs.len(), moved.knobs.len(), "knobs");
        for (shifted, moved) in shifted.knobs.iter().zip(&moved.knobs) {
            close(shifted.dial, moved.dial);
            close(shifted.label.bounds, moved.label.bounds);
            close(shifted.value.bounds, moved.value.bounds);
        }
        match (&shifted.chip, &moved.chip) {
            (Some(a), Some(b)) => {
                close(a.bounds, b.bounds);
                close(a.icon, b.icon);
                close(a.label.bounds, b.label.bounds);
                close(a.chevron, b.chevron);
            }
            (None, None) => {}
            _ => panic!("chip presence changed"),
        }
        assert_eq!(shifted.inputs.len(), moved.inputs.len(), "input anchors");
        for (shifted, moved) in shifted.inputs.iter().zip(&moved.inputs) {
            assert_position(*shifted, *moved);
        }
        match (shifted.output, moved.output) {
            (Some(shifted), Some(moved)) => assert_position(shifted, moved),
            (None, None) => {}
            _ => panic!("output anchor presence changed"),
        }
        close(
            shifted.matrix.unwrap_or(shifted.body),
            moved.matrix.unwrap_or(moved.body),
        );
        close(
            shifted.curve.unwrap_or(shifted.body),
            moved.curve.unwrap_or(moved.body),
        );
        assert_close(shifted.scale, moved.scale);
    }
}
