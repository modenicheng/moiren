//! Real native events verify the action contract independently of audio hardware.

use crate::ui::{
    ActionButton, ActionIcon, ControlSize, Density, IconButton, ToggleIconButton, WindowAction,
    WindowControlButton, WorkspaceTab,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px};
use std::{cell::Cell, rc::Rc};

struct Actions {
    activations: Rc<Cell<usize>>,
    request: Rc<Cell<Option<bool>>>,
    selected: bool,
    disabled: bool,
}

impl Render for Actions {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let count = self.activations.clone();
        let icon_count = count.clone();
        let window_count = count.clone();
        let disabled_count = count.clone();
        let disabled_icon_count = count.clone();
        let request = self.request.clone();
        let disabled_request = request.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                ActionButton::new("action", "Apply")
                    .on_click(move |_, _, _| count.set(count.get() + 1)),
            )
            .child(
                IconButton::new("icon", "Settings", ActionIcon::Settings)
                    .on_click(move |_, _, _| icon_count.set(icon_count.get() + 1)),
            )
            .child(
                WindowControlButton::new("window", WindowAction::Minimize)
                    .on_click(move |_, _, _| window_count.set(window_count.get() + 1)),
            )
            .child(
                ActionButton::new("disabled", "Disabled")
                    .disabled(self.disabled)
                    .on_click(move |_, _, _| disabled_count.set(disabled_count.get() + 1)),
            )
            .child(
                IconButton::new("disabled-icon", "Disabled settings", ActionIcon::Settings)
                    .disabled(self.disabled)
                    .on_click(move |_, _, _| {
                        disabled_icon_count.set(disabled_icon_count.get() + 1)
                    }),
            )
            .child(
                ToggleIconButton::new("toggle", "Monitor", ActionIcon::Settings)
                    .selected(self.selected)
                    .on_change(move |value, _, _, _| request.set(Some(value))),
            )
            .child(
                ToggleIconButton::new("disabled-toggle", "Disabled monitor", ActionIcon::Settings)
                    .selected(true)
                    .disabled(true)
                    .on_change(move |value, _, _, _| disabled_request.set(Some(value))),
            )
            .child(WorkspaceTab::new("selected-tab", "Graph").active(true))
            .child(WorkspaceTab::new("disabled-tab", "Devices").enabled(false))
    }
}

struct Harness<'a> {
    view: gpui_kit::Entity<Actions>,
    cx: &'a mut gpui_kit::VisualTestContext,
    count: Rc<Cell<usize>>,
    request: Rc<Cell<Option<bool>>>,
}

fn harness(cx: &mut gpui_kit::TestAppContext) -> Harness<'_> {
    cx.update(gpui_kit::init);
    let count = Rc::new(Cell::new(0));
    let request = Rc::new(Cell::new(None));
    let (view, cx) = cx.add_window_view(|_, _| Actions {
        activations: count.clone(),
        request: request.clone(),
        selected: false,
        disabled: true,
    });
    cx.update(|window, cx| window.render_frame(cx));
    Harness {
        view,
        cx,
        count,
        request,
    }
}

#[gpui_kit::test]
fn action_family_uses_pointer_enter_and_space_once(cx: &mut gpui_kit::TestAppContext) {
    let Harness { cx, count, .. } = harness(cx);
    for id in ["action", "icon", "window"] {
        let before = count.get();
        cx.update(|window, cx| {
            window.click(id, cx);
            assert_eq!(window.find(id).focused(), Some(true));
            window.press("enter", cx);
            window.press("space", cx);
        });
        assert_eq!(count.get(), before + 3, "{id} must activate once per input");
    }
    cx.update(|window, cx| {
        window.click("disabled", cx);
        window.click("disabled-icon", cx);
    });
    assert_eq!(count.get(), 9);
}

#[gpui_kit::test]
fn action_buttons_reenable_with_the_same_identity(cx: &mut gpui_kit::TestAppContext) {
    let Harness {
        view, cx, count, ..
    } = harness(cx);
    cx.update(|window, cx| {
        let bounds = [
            window.find("disabled").bounds(),
            window.find("disabled-icon").bounds(),
        ];
        for disabled in [true, false, true, false] {
            view.update(cx, |view, _| view.disabled = disabled);
            window.render_frame(cx);
            window.click("action", cx);
            for (id, bounds) in ["disabled", "disabled-icon"].into_iter().zip(bounds) {
                assert_eq!(window.find(id).bounds(), bounds);
                let before = count.get();
                window.click(id, cx);
                if disabled {
                    assert_eq!(count.get(), before);
                    assert_eq!(window.find("action").focused(), Some(true));
                } else {
                    assert_eq!(window.find(id).focused(), Some(true));
                    window.press("enter", cx);
                    window.press("space", cx);
                    assert_eq!(count.get(), before + 3);
                }
            }
        }
    });
}

#[gpui_kit::test]
fn disabled_actions_expose_disabled_accessibility_without_activation(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{Element, RenderOnce, accesskit, canvas};
    use std::sync::{Arc, Mutex};

    type Captured = Arc<Mutex<Vec<(bool, accesskit::Node)>>>;
    struct AccessibilityProbe(Captured);
    impl Render for AccessibilityProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let captured = self.0.clone();
            canvas(
                move |_, window, cx| {
                    let mut nodes = captured.lock().unwrap();
                    nodes.clear();
                    for disabled in [false, true, false] {
                        let action =
                            ActionButton::new("apply", "Apply measured budget and restart")
                                .disabled(disabled)
                                .on_click(|_, _, _| {})
                                .render(window, cx)
                                .into_element();
                        assert_eq!(action.id(), Some("apply".into()));
                        assert_eq!(action.a11y_role(), Some(accesskit::Role::Button));
                        let mut node = accesskit::Node::new(action.a11y_role().unwrap());
                        action.write_a11y_info(&mut node);
                        assert_eq!(node.label(), Some("Apply measured budget and restart"));
                        nodes.push((disabled, node));
                        let icon = IconButton::new("settings", "Settings", ActionIcon::Settings)
                            .disabled(disabled)
                            .on_click(|_, _, _| {})
                            .render(window, cx)
                            .into_element();
                        assert_eq!(icon.id(), Some("settings".into()));
                        assert_eq!(icon.a11y_role(), Some(accesskit::Role::Button));
                        let mut node = accesskit::Node::new(icon.a11y_role().unwrap());
                        icon.write_a11y_info(&mut node);
                        assert_eq!(node.label(), Some("Settings"));
                        nodes.push((disabled, node));
                    }
                },
                |_, _, _, _| {},
            )
        }
    }
    cx.update(gpui_kit::init);
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let result = captured.clone();
    let (_, cx) = cx.add_window_view(move |_, _| AccessibilityProbe(captured));
    cx.update(|window, cx| window.render_frame(cx));
    let nodes = result.lock().unwrap();
    assert_eq!(nodes.len(), 6);
    for (disabled, node) in nodes.iter() {
        assert_eq!(node.role(), accesskit::Role::Button);
        assert!(node.label().is_some());
        assert_eq!(node.supports_action(accesskit::Action::Click), !disabled);
        assert_eq!(node.supports_action(accesskit::Action::Focus), !disabled);
        assert_eq!(node.is_disabled(), *disabled);
    }
}

#[gpui_kit::test]
fn toggle_waits_for_owner_and_disabled_toggle_is_inert(cx: &mut gpui_kit::TestAppContext) {
    let Harness {
        view, cx, request, ..
    } = harness(cx);
    cx.update(|window, cx| {
        window.click("toggle", cx);
        assert_eq!(request.get(), Some(true));
        assert_eq!(
            window.find("toggle").checked(),
            Some(false),
            "requests do not imply confirmation"
        );
        window.press("space", cx);
        assert_eq!(request.get(), Some(true));
        view.update(cx, |view, _| view.selected = true);
        window.render_frame(cx);
        assert_eq!(window.find("toggle").checked(), Some(true));
        window.press("enter", cx);
        assert_eq!(request.get(), Some(false));
        window.click("disabled-toggle", cx);
        assert_eq!(request.get(), Some(false));
        assert_eq!(window.find("toggle").focused(), Some(true));
        assert_eq!(window.find("disabled-toggle").checked(), Some(true));
    });
}

#[gpui_kit::test]
fn workspace_selection_is_exposed_and_disabled_tab_cannot_focus(cx: &mut gpui_kit::TestAppContext) {
    let Harness { cx, .. } = harness(cx);
    cx.update(|window, cx| {
        assert_eq!(window.find("selected-tab").selected(), Some(true));
        window.click("action", cx);
        window.click("disabled-tab", cx);
        assert_eq!(window.find("action").focused(), Some(true));
    });
}

struct Sizes;
impl Render for Sizes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().children(
            [Density::Comfortable, Density::Compact]
                .into_iter()
                .flat_map(|density| {
                    [ControlSize::Small, ControlSize::Medium, ControlSize::Large]
                        .into_iter()
                        .map(move |size| {
                            ActionButton::new(
                                format!("{density:?}-{size:?}"),
                                "Apply on next start",
                            )
                            .size(size)
                            .density(density)
                        })
                }),
        )
    }
}

#[gpui_kit::test]
fn density_resolves_control_heights_without_shrinking_long_labels(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|_, _| Sizes);
    cx.update(|window, cx| {
        window.render_frame(cx);
        for (id, height) in [
            ("Comfortable-Small", 28.),
            ("Comfortable-Medium", 32.),
            ("Comfortable-Large", 36.),
            ("Compact-Small", 28.),
            ("Compact-Medium", 30.),
            ("Compact-Large", 32.),
        ] {
            let bounds = window.find(id).bounds();
            assert_eq!(bounds.size.height, px(height));
            assert!(
                bounds.size.width >= px(110.),
                "the label must retain its natural width"
            );
        }
    });
}
