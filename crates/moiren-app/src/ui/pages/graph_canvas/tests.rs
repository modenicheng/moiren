use std::sync::Arc;

use super::{Camera, CanvasFrame, CanvasWire, CanvasWorld, HitTarget, NodeLayout, WireCurve, node};
use gpui_kit::{Bounds, Path, Pixels, point, px, size};

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
}

#[test]
fn workstation_geometry_follows_native_content_measurements() {
    let mut card = node("input", "Physical input", "Device").ports(false, true);
    card.content_width = 356.;
    card.measured_height = Some(246.);
    assert_eq!(card.width(), 356.);
    assert_eq!(card.height(), 246.);
    card.measured_height = Some(318.);
    assert_eq!(NodeLayout::new(&card).card.size.height, 318.);
    assert_eq!(NodeLayout::new(&card).output.unwrap().0, 356.);
}

#[test]
fn rack_width_follows_configured_parameter_budget_not_stage_count() {
    use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};
    let settings = |processor| RackSettings {
        slots: vec![RackSlot {
            id: 1,
            bypass: false,
            processor,
        }],
        ..Default::default()
    };
    assert_eq!(super::rack_content_width(None), 248.);
    let gain = settings(RackProcessor::Gain { initial: 1. });
    let eq = settings(RackProcessor::Eq(Default::default()));
    let mapper = settings(RackProcessor::ChannelMapper(Default::default()));
    assert!(super::rack_content_width(Some(&gain)) < super::rack_content_width(Some(&eq)));
    assert!(super::rack_content_width(Some(&eq)) < super::rack_content_width(Some(&mapper)));
    let mut many = gain.clone();
    many.slots.extend((2..=64).map(|id| RackSlot {
        id,
        ..gain.slots[0].clone()
    }));
    assert_eq!(
        super::rack_content_width(Some(&gain)),
        super::rack_content_width(Some(&many))
    );
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
    let wire = CanvasWire {
        id: "edge".into(),
        source: source.id.clone(),
        destination: sink.id.clone(),
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
        assert_close(layout.gain_value.font_size, 10. * zoom);
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
fn node_layout_keeps_shared_header_and_ports_above_workstation_body() {
    let mut card = node("input", "Physical Input", "48kHz L/R").ports(false, true);
    card.measured_height = Some(287.);
    let layout = NodeLayout::new(&card);
    assert_eq!(layout.card.size, size(320., 287.));
    assert_eq!(
        layout.icon,
        Bounds {
            origin: point(12., 18.),
            size: size(28., 28.)
        }
    );
    assert_position(
        (layout.title.bounds.origin.x, layout.title.bounds.origin.y),
        (44., 20.),
    );
    assert_close(layout.title.font_size, 12.);
    assert_close(layout.detail.font_size, 10.);
    assert_close(layout.detail.bounds.origin.y, 35.);
    assert_close(layout.meter.origin.y, 60.);
    assert_eq!(
        layout.fader,
        Bounds {
            origin: point(12., 68.),
            size: size(211., 4.)
        }
    );
    assert_eq!(
        layout.balance,
        Bounds {
            origin: point(12., 88.),
            size: size(211., 4.)
        }
    );
    assert_position(layout.output.unwrap(), (320., 80.));
    assert_eq!(layout.gain_value.bounds.size, size(25., 12.));
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
        let wire = CanvasWire {
            id: "wire".into(),
            source: "source".into(),
            destination: "sink".into(),
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
    let wire = CanvasWire {
        id: "wire".into(),
        source: "source".into(),
        destination: "sink".into(),
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
