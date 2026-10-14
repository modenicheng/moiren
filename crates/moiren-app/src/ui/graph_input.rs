use super::MainWindow;
use slint::winit_030::{
    EventResult, WinitWindowAccessor,
    winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
};
use slint::{ComponentHandle, LogicalPosition};

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[derive(Debug, PartialEq)]
enum ScrollAction {
    Pan(f32, f32),
    Zoom(f32),
}

// Keep wheel zoom smooth for both integer wheel notches and high-resolution
// pixel deltas. A 60 logical-pixel step is about 2%, while smaller deltas are
// still preserved instead of being rounded to a direction-only step.
const WHEEL_ZOOM_STEP: f32 = 60.0;
const WHEEL_ZOOM_FACTOR: f32 = 1.02;
const MAX_ZOOM_LOG_STEP: f32 = 0.5;

fn exponential_zoom_factor(log_delta: f32) -> f32 {
    log_delta.clamp(-MAX_ZOOM_LOG_STEP, MAX_ZOOM_LOG_STEP).exp()
}

fn wheel_zoom_factor(delta_y: f32) -> f32 {
    exponential_zoom_factor(delta_y / WHEEL_ZOOM_STEP * WHEEL_ZOOM_FACTOR.ln())
}

fn pinch_zoom_factor(delta: f32) -> f32 {
    // Winit reports pinch delta as a fractional scale change. Keep the
    // familiar 10% response at delta 0.1 while making repeated tiny updates
    // multiplicative and exactly reversible.
    exponential_zoom_factor(delta / 0.1 * 1.1_f32.ln())
}

fn scroll_action(
    delta: MouseScrollDelta,
    touchpad: bool,
    control: bool,
    scale_factor: f32,
) -> ScrollAction {
    let (delta_x, delta_y, continuous) = match delta {
        MouseScrollDelta::LineDelta(horizontal, vertical) => {
            (horizontal * 60.0, vertical * 60.0, touchpad)
        }
        MouseScrollDelta::PixelDelta(position) => (
            position.x as f32 / scale_factor,
            position.y as f32 / scale_factor,
            true,
        ),
    };
    if continuous && !control {
        ScrollAction::Pan(delta_x, delta_y)
    } else {
        ScrollAction::Zoom(wheel_zoom_factor(delta_y))
    }
}

#[derive(Default)]
struct GraphInput {
    cursor: Option<LogicalPosition>,
    drag: Option<(MouseButton, LogicalPosition)>,
    control: bool,
}

impl GraphInput {
    fn anchor(&self, window: &MainWindow) -> Option<LogicalPosition> {
        let cursor = self.cursor?;
        let viewport = window.get_graph_viewport();
        let anchor = LogicalPosition::new(cursor.x - viewport.left, cursor.y - viewport.top);
        (window.get_graph_input_enabled()
            && anchor.x >= 0.0
            && anchor.y >= 0.0
            && anchor.x < viewport.width
            && anchor.y < viewport.height)
            .then_some(anchor)
    }

    fn handle(&mut self, window: &MainWindow, event: &WindowEvent, touchpad: bool) -> EventResult {
        if !window.get_graph_input_enabled() {
            self.drag = None;
            window.invoke_cancel_graph_input();
        }
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.control = modifiers.state().control_key();
            }
            WindowEvent::Focused(false) => {
                self.drag = None;
                self.cursor = None;
                self.control = false;
                window.invoke_cancel_graph_input();
            }
            WindowEvent::CursorLeft { .. } if self.drag.is_none() => self.cursor = None,
            WindowEvent::CursorMoved { position, .. } => {
                let position = LogicalPosition::new(
                    position.x as f32 / window.window().scale_factor(),
                    position.y as f32 / window.window().scale_factor(),
                );
                self.cursor = Some(position);
                if let Some((button, previous)) = self.drag {
                    window.invoke_pan_graph(position.x - previous.x, position.y - previous.y);
                    self.drag = Some((button, position));
                    return EventResult::PreventDefault;
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if *state == ElementState::Released
                    && self.drag.is_some_and(|(pressed, _)| pressed == *button)
                {
                    self.drag = None;
                    return EventResult::PreventDefault;
                }
                if *state == ElementState::Pressed
                    && self.drag.is_none()
                    && (*button == MouseButton::Middle
                        || (*button == MouseButton::Left && window.get_graph_pan_with_left()))
                    && self
                        .anchor(window)
                        .is_some_and(|anchor| anchor.y >= window.get_graph_navigation_top())
                {
                    window.invoke_focus_graph();
                    self.drag = self.cursor.map(|position| (*button, position));
                    return EventResult::PreventDefault;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(anchor) = self.anchor(window) {
                    let control = self.control;
                    #[cfg(windows)]
                    let control = control || windows::wheel_control();
                    match scroll_action(*delta, touchpad, control, window.window().scale_factor()) {
                        ScrollAction::Pan(horizontal, vertical) => {
                            window.invoke_pan_graph(horizontal, vertical);
                        }
                        ScrollAction::Zoom(factor) => {
                            window.invoke_zoom_graph(factor, anchor.x, anchor.y);
                        }
                    }
                    return EventResult::PreventDefault;
                }
            }
            WindowEvent::PinchGesture { delta, .. } => {
                if let Some(anchor) = self.anchor(window) {
                    window.invoke_zoom_graph(pinch_zoom_factor(*delta as f32), anchor.x, anchor.y);
                    return EventResult::PreventDefault;
                }
            }
            _ => {}
        }
        EventResult::Propagate
    }
}

pub(super) fn install(window: &MainWindow) {
    let weak = window.as_weak();
    let mut input = GraphInput::default();
    #[cfg(windows)]
    let mut registration = windows::TouchpadRegistration::new();
    window.window().on_winit_window_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        #[cfg(windows)]
        let touchpad = {
            registration.attach(window.window());
            matches!(event, WindowEvent::MouseWheel { .. }) && registration.is_touchpad()
        };
        #[cfg(not(windows))]
        let touchpad = false;
        input.handle(&window, event, touchpad)
    });
}

#[cfg(test)]
mod tests;
