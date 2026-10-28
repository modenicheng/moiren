//! Quiet section boundaries, not a stack of competing cards.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Section {
    Binding,
    Strip,
    Advanced,
    Routing,
    Readings,
}

impl Section {
    fn id(self) -> &'static str {
        match self {
            Self::Binding => "inspector-binding",
            Self::Strip => "inspector-strip",
            Self::Advanced => "inspector-parameters",
            Self::Routing => "inspector-routing",
            Self::Readings => "inspector-meter-details",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Binding => "Resource binding",
            Self::Strip => "Channel strip",
            Self::Advanced => "Advanced parameters",
            Self::Routing => "Ports and routing",
            Self::Readings => "Channel readings",
        }
    }

    fn default_open(self) -> bool {
        matches!(self, Self::Advanced | Self::Routing)
    }
}

type ToggleHandler = Box<dyn Fn(bool, &mut Window, &mut App)>;

/// Window-owned disclosure state by default. An opt-in owner can share the
/// actual open state and flush departing editors before their inputs unmount.
#[derive(IntoElement)]
pub(super) struct Disclosure {
    kind: Section,
    node: moiren_core::graph::NodeId,
    summary: Option<SharedString>,
    content: AnyElement,
    toggle: Option<(bool, ToggleHandler)>,
}

impl Disclosure {
    pub(super) fn new(
        kind: Section,
        node: moiren_core::graph::NodeId,
        content: impl IntoElement,
    ) -> Self {
        Self {
            kind,
            node,
            summary: None,
            content: content.into_any_element(),
            toggle: None,
        }
    }

    pub(super) fn summary(mut self, summary: impl Into<SharedString>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub(super) fn on_toggle(
        mut self,
        open: bool,
        handler: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.toggle = Some((open, Box::new(handler)));
        self
    }
}

impl RenderOnce for Disclosure {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.kind.id();
        let default_open = self.kind.default_open();
        let (open, toggle): (bool, ToggleHandler) = if let Some(toggle) = self.toggle {
            toggle
        } else {
            let state = window.use_keyed_state(format!("{id}-open-{:?}", self.node), cx, |_, _| {
                default_open
            });
            let open = *state.read(cx);
            (
                open,
                Box::new(move |open, _, cx| {
                    state.update(cx, |state, cx| {
                        *state = open;
                        cx.notify();
                    });
                }),
            )
        };
        let header = button(format!("{id}-toggle"), t!(self.kind.title()))
            .variant(ButtonVariant::Ghost)
            .full_width()
            .justify_start()
            .trailing(if open { "-" } else { "+" })
            .tooltip(if open {
                t!("Collapse section")
            } else {
                t!("Expand section")
            })
            .on_click(move |_, window, cx| {
                toggle(!open, window, cx);
            });
        let mut disclosure = component::collapsible::Collapsible::new()
            .open(open)
            .w_full()
            .gap(px(theme::SPACE_1))
            .child(header);
        if let Some(summary) = self.summary {
            disclosure = disclosure.child(
                div()
                    .id(format!("{id}-summary"))
                    .child(
                        label(summary.clone())
                            .px(px(theme::SPACE_2))
                            .text_size(px(theme::TEXT_XS))
                            .overflow_hidden()
                            .text_ellipsis(),
                    )
                    .tooltip(move |window, cx| {
                        component::tooltip::Tooltip::new(summary.clone()).build(window, cx)
                    }),
            );
        }
        // No reveal animation: closed inputs must actually unmount, not stay
        // keyboard-focusable in a zero-height region. Child IDs stay unchanged.
        div()
            .id(id)
            .flex()
            .flex_col()
            .flex_none()
            .min_w_0()
            .pt(px(theme::SPACE_2))
            .border_t_1()
            .border_color(theme::HAIRLINE)
            .child(disclosure.content(self.content))
            .gpui_test_observe()
    }
}

pub(super) fn body() -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(px(theme::SPACE_2))
        .pt(px(theme::SPACE_1))
}

pub(super) fn section(id: &'static str, title: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_none()
        .min_w_0()
        .gap(px(theme::SPACE_2))
        .pt(px(theme::SPACE_3))
        .border_t_1()
        .border_color(theme::HAIRLINE)
        .child(
            div()
                .text_size(px(theme::TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::TEXT)
                .child(title.into()),
        )
}

/// Bounded resource groups keep a large device catalog from hiding every
/// parameter and diagnostic below it. Each group retains its own scroll state.
pub(super) fn resource_list(id: &'static str, title: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_none()
        .min_w_0()
        .max_h(px(128.))
        .overflow_y_scroll()
        .gap(px(theme::SPACE_1))
        .child(label(title).text_size(px(theme::TEXT_XS)))
}

#[cfg(test)]
mod tests {
    use super::Section;

    #[::core::prelude::v1::test]
    fn secondary_controls_start_closed_but_parameters_and_routing_start_open() {
        assert!(!Section::Binding.default_open());
        assert!(!Section::Strip.default_open());
        assert!(!Section::Readings.default_open());
        assert!(Section::Advanced.default_open());
        assert!(Section::Routing.default_open());
    }
}
