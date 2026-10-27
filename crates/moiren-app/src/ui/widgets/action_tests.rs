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
}

impl Render for Actions {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let count = self.activations.clone();
        let icon_count = count.clone();
        let window_count = count.clone();
        let disabled_count = count.clone();
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
                    .disabled(true)
                    .on_click(move |_, _, _| disabled_count.set(disabled_count.get() + 1)),
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
    cx.update(|window, cx| window.click("disabled", cx));
    assert_eq!(count.get(), 9);
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

struct LongResource {
    activations: Rc<Cell<usize>>,
}

impl Render for LongResource {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let count = self.activations.clone();
        div().w(px(220.)).child(
            ActionButton::new(
                "long-resource",
                "Studio interface / a very long endpoint name / channel pair 31-32",
            )
            .full_width()
            .justify_start()
            .truncate_label()
            .trailing("v")
            .on_click(move |_, _, _| count.set(count.get() + 1)),
        )
    }
}

#[gpui_kit::test]
fn opt_in_resource_label_stays_bounded_without_losing_keyboard_activation(
    cx: &mut gpui_kit::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let count = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|_, _| LongResource {
        activations: count.clone(),
    });
    cx.update(|window, cx| {
        window.render_frame(cx);
        let bounds = window.find("long-resource").bounds();
        assert_eq!(bounds.size.width, px(220.));
        assert_eq!(bounds.size.height, px(32.));
        window.click("long-resource", cx);
        window.press("enter", cx);
        window.press("space", cx);
    });
    assert_eq!(count.get(), 3);
}
