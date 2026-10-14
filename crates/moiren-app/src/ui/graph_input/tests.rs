use super::*;
use slint::winit_030::winit::dpi::PhysicalPosition;

#[test]
fn mouse_wheel_zooms_by_distance_and_inverts_exactly() {
    let ScrollAction::Zoom(forward) =
        scroll_action(MouseScrollDelta::LineDelta(0.0, 2.0), false, false, 1.0)
    else {
        panic!("mouse wheel must zoom");
    };
    let ScrollAction::Zoom(backward) =
        scroll_action(MouseScrollDelta::LineDelta(0.0, -2.0), false, false, 1.0)
    else {
        panic!("mouse wheel must zoom");
    };
    assert!((forward - 1.0404).abs() < 0.0001);
    assert!((forward * backward - 1.0).abs() < 0.0001);
}

#[test]
fn pixel_zoom_preserves_fractional_increments() {
    let ScrollAction::Zoom(factor) = scroll_action(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 1.0)),
        false,
        true,
        1.0,
    ) else {
        panic!("control pixel scroll must zoom");
    };
    assert!(factor > 1.0);
    assert!(factor < 1.001);
    assert!((factor * wheel_zoom_factor(-1.0) - 1.0).abs() < 0.000001);
}

#[test]
fn precision_touchpad_line_events_pan_both_axes() {
    assert_eq!(
        scroll_action(MouseScrollDelta::LineDelta(0.25, -0.5), true, false, 2.0),
        ScrollAction::Pan(15.0, -30.0)
    );
}

#[test]
fn pixel_scroll_preserves_fractional_distance_and_dpi() {
    assert_eq!(
        scroll_action(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(-6.5, 9.0)),
            false,
            false,
            2.0,
        ),
        ScrollAction::Pan(-3.25, 4.5)
    );
}

#[test]
fn control_wheel_pinch_zooms_instead_of_panning() {
    assert!(matches!(
        scroll_action(MouseScrollDelta::LineDelta(0.0, 0.5), true, true, 1.0),
        ScrollAction::Zoom(factor) if factor > 1.0
    ));
    assert!(matches!(
        scroll_action(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -12.0)),
            false,
            true,
            1.0,
        ),
        ScrollAction::Zoom(factor) if factor < 1.0
    ));
}

#[test]
fn graph_navigation_keeps_nodes_fixed_and_resets_on_focus_loss() -> anyhow::Result<()> {
    use super::super::{Backend, GraphNodeData};
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent as UiEvent};
    use slint::winit_030::winit::event::{DeviceId, TouchPhase};
    use slint::{LogicalSize, ModelRc, Rgb8Pixel, VecModel};
    use std::{cell::Cell, rc::Rc};

    struct TestPlatform(Rc<MinimalSoftwareWindow>);
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
    }
    let surface = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(surface.clone())))?;
    let window = MainWindow::new()?;
    surface.set_size(LogicalSize::new(1600.0, 956.0));
    let backend = window.global::<Backend>();
    backend.set_graph_left(-1000.0);
    backend.set_graph_top(-800.0);
    backend.set_graph_width(186.0);
    backend.set_graph_height(105.0);
    backend.set_graph_nodes(ModelRc::new(VecModel::from(vec![GraphNodeData {
        id: "negative-node".into(),
        name: "Negative node".into(),
        x: -1000.0,
        y: -800.0,
        card_width: 186.0,
        card_height: 105.0,
        ..Default::default()
    }])));
    let moved_node = Rc::new(Cell::new(false));
    let moved = moved_node.clone();
    backend.on_move_node(move |_, _, _| moved.set(true));
    window.show()?;
    let mut pixels = vec![Rgb8Pixel::default(); 1600 * 956];
    surface.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1600);
    });
    window.invoke_pan_graph(
        1000.0 * window.get_graph_zoom() + 64.0 - window.get_graph_pan_x(),
        800.0 * window.get_graph_zoom() + 150.0 - window.get_graph_pan_y(),
    );
    let viewport = window.get_graph_viewport();
    let position = LogicalPosition::new(viewport.left + 90.0, viewport.top + 170.0);
    window.window().dispatch_event(UiEvent::PointerPressed {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    window.window().dispatch_event(UiEvent::PointerReleased {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(UiEvent::KeyPressed { text: " ".into() });
    assert!(window.get_graph_pan_with_left());
    let mut input = GraphInput {
        cursor: Some(position),
        ..Default::default()
    };
    let button_event = |button, state| WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        button,
        state,
    };
    assert_eq!(
        input.handle(
            &window,
            &button_event(MouseButton::Left, ElementState::Pressed),
            false
        ),
        EventResult::PreventDefault
    );
    let before = window.get_graph_pan_x();
    assert_eq!(
        input.handle(
            &window,
            &WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(1800.0, f64::from(position.y))
            },
            false
        ),
        EventResult::PreventDefault
    );
    assert!((window.get_graph_pan_x() - before - (1800.0 - position.x)).abs() < 0.01);
    input.handle(
        &window,
        &button_event(MouseButton::Right, ElementState::Released),
        false,
    );
    assert!(input.drag.is_some());
    input.handle(
        &window,
        &button_event(MouseButton::Left, ElementState::Released),
        false,
    );
    window
        .window()
        .dispatch_event(UiEvent::KeyReleased { text: " ".into() });
    assert!(!window.get_graph_pan_with_left() && input.drag.is_none());
    input.cursor = Some(position);
    assert_eq!(
        input.handle(
            &window,
            &button_event(MouseButton::Middle, ElementState::Pressed),
            false
        ),
        EventResult::PreventDefault
    );
    input.handle(
        &window,
        &button_event(MouseButton::Middle, ElementState::Released),
        false,
    );
    let wheel = |delta| WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta,
        phase: TouchPhase::Moved,
    };
    let zoom = window.get_graph_zoom();
    let anchor = (position.x - viewport.left - window.get_graph_pan_x()) / zoom;
    input.handle(
        &window,
        &wheel(MouseScrollDelta::LineDelta(0.0, 1.0)),
        false,
    );
    assert!(
        ((position.x - viewport.left - window.get_graph_pan_x()) / window.get_graph_zoom()
            - anchor)
            .abs()
            < 0.01
    );
    let zoom = window.get_graph_zoom();
    let pan = (window.get_graph_pan_x(), window.get_graph_pan_y());
    input.handle(
        &window,
        &wheel(MouseScrollDelta::LineDelta(0.25, -0.5)),
        true,
    );
    assert_eq!(window.get_graph_zoom(), zoom);
    assert_eq!(window.get_graph_pan_x(), pan.0 + 15.0);
    assert_eq!(window.get_graph_pan_y(), pan.1 - 30.0);
    input.handle(
        &window,
        &WindowEvent::PinchGesture {
            device_id: DeviceId::dummy(),
            delta: 0.1,
            phase: TouchPhase::Moved,
        },
        false,
    );
    assert!((window.get_graph_zoom() - zoom * pinch_zoom_factor(0.1)).abs() < 0.001);
    window.invoke_focus_graph();
    window
        .window()
        .dispatch_event(UiEvent::KeyPressed { text: " ".into() });
    input.handle(
        &window,
        &button_event(MouseButton::Left, ElementState::Pressed),
        false,
    );
    input.handle(&window, &WindowEvent::Focused(false), false);
    assert!(!window.get_graph_pan_with_left() && input.drag.is_none());
    input.cursor = Some(position);
    backend.set_view(1);
    assert_eq!(
        input.handle(
            &window,
            &wheel(MouseScrollDelta::LineDelta(0.0, 1.0)),
            false
        ),
        EventResult::Propagate
    );
    assert!(!moved_node.get());
    window.hide()?;
    Ok(())
}
