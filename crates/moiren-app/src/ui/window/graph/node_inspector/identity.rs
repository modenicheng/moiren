//! Selected-node identity and inline rename lifecycle.
use super::*;

pub(super) fn identity(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    entry: &crate::service::GraphNodeState,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let id = node.id();
    let display =
        crate::ui::state::display_label(&shell.state.read(cx).snapshot, entry, node.kind());
    let editing = shell.node_name_editing == Some(id);
    // One inline name control: a plain heading until clicked, then the editor
    // in the same place. There is no separate rename row.
    let name_control: AnyElement = if editing {
        div()
            .id("node-name-editor")
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                div().flex_1().min_w_0().child(
                    component::input::Input::new(&shell.node_name)
                        .id("node-name")
                        .bg(theme::SURFACE_3)
                        .text_color(theme::TEXT)
                        .border_color(theme::BORDER),
                ),
            )
            .child(button("node-name-confirm", "✓").on_click({
                let this = cx.entity();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.confirm_node_rename(cx));
                }
            }))
            .child(button("node-name-cancel", "✗").on_click({
                let this = cx.entity();
                move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.cancel_node_rename(cx));
                }
            }))
            .into_any_element()
    } else {
        div()
            .id("node-name-display")
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .child(
                heading(display.clone())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis(),
            )
            .child(
                div()
                    .id("node-name-edit-hint")
                    .text_size(px(theme::TEXT_XS))
                    .text_color(theme::TEXT_FAINT)
                    .child(t!("rename")),
            )
            .on_click({
                let this = cx.entity();
                let initial = display.clone();
                move |_, window, cx| {
                    this.update(cx, |shell, cx| {
                        shell.node_name_editing = Some(id);
                        shell
                            .node_name
                            .update(cx, |input, cx| input.set_value(initial.clone(), window, cx));
                        cx.notify();
                    });
                    let input = this.read(cx).node_name.clone();
                    let handle = input.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                }
            })
            .gpui_test_observe()
            .into_any_element()
    };
    div()
        .id("inspector-identity")
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(theme::SPACE_2))
        .pb(px(theme::SPACE_3))
        .child(name_control)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(badge(
                    t!(crate::service::default_label(node.kind())),
                    crate::ui::window::mixer::node_kind_color(node.kind()),
                ))
                .child(label(format!("{} ch", node.channels()))),
        )
        .gpui_test_observe()
        .into_any_element()
}

impl StudioShell {
    /// Per-render graph-page housekeeping: exit a name edit whose node is no
    /// longer selected, drop selection members the graph no longer has, and
    /// keep the primary selection inside the canvas multi-selection.
    pub(in crate::ui::window) fn sync_node_name(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.node_name_editing
            && self.selected_node != Some(id)
        {
            self.node_name_editing = None;
        }
        let graph = self.state.read(cx).snapshot.graph.clone();
        self.selected_nodes.retain(|id| graph.node(*id).is_some());
        if let Some(id) = self.selected_node {
            if graph.node(id).is_some() {
                self.selected_nodes.insert(id);
            } else {
                self.selected_node = None;
            }
        }
    }

    /// Commits the inline name edit: dispatches SetLabel unless the value is
    /// empty (treated as cancel). Enter, ✓ and focus loss land here.
    pub(in crate::ui::window) fn confirm_node_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.node_name_editing.take() else {
            return;
        };
        let label = self.node_name.read(cx).value().trim().to_string();
        if !label.is_empty() {
            self.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::SetLabel {
                    node: id,
                    label,
                }))
            });
        }
        cx.notify();
    }

    /// Discards the inline name edit without touching the stored label.
    pub(in crate::ui::window) fn cancel_node_rename(&mut self, cx: &mut Context<Self>) {
        if self.node_name_editing.take().is_some() {
            cx.notify();
        }
    }
}
