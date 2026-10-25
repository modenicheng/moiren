//! Shared workspace and window actions, Figma 59:542 and 56:512.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::action_assets::ActionIcon;
use super::action_style::{Paint, SampleState, Skin, styled_control};
use crate::ui::theme::controls as t;
use rust_i18n::t;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug)]
pub enum WindowAction {
    Minimize,
    Maximize,
    Close,
}

impl WindowAction {
    fn icon(self) -> ActionIcon {
        match self {
            Self::Minimize => ActionIcon::Minimize,
            Self::Maximize => ActionIcon::Maximize,
            Self::Close => ActionIcon::Close,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Minimize => "Minimize window",
            Self::Maximize => "Maximize window",
            Self::Close => "Close window",
        }
    }
}

/// Caller-owned window action, including the application's close/tray policy.
#[derive(IntoElement)]
pub struct WindowControlButton {
    id: ElementId,
    action: WindowAction,
    on_click: Option<ClickHandler>,
    sample: Option<SampleState>,
}

impl WindowControlButton {
    pub fn new(id: impl Into<ElementId>, action: WindowAction) -> Self {
        Self {
            id: id.into(),
            action,
            on_click: None,
            sample: None,
        }
    }
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
    pub(crate) fn sample(mut self, state: SampleState) -> Self {
        self.sample = Some(state);
        self
    }
}

impl RenderOnce for WindowControlButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let normal = Paint::new(t::TRANSPARENT, t::MUTED, t::TRANSPARENT);
        let close = matches!(self.action, WindowAction::Close);
        let skin = Skin {
            normal,
            hover: Paint::new(
                if close { t::DANGER } else { t::RAISED },
                if close { t::ON_ACCENT } else { t::MUTED },
                t::TRANSPARENT,
            ),
            pressed: Paint::new(
                if close { t::CLOSE_PRESSED } else { t::CANVAS },
                if close { t::ON_ACCENT } else { t::MUTED },
                t::TRANSPARENT,
            ),
            disabled: normal,
            focus: normal,
        };
        styled_control(
            base::Button::new(self.id).accessibility_label(t!(self.action.label())),
            skin,
            false,
            self.sample,
        )
        .rounded_none()
        .w(px(t::WINDOW_WIDTH))
        .h(px(t::HIT_TARGET))
        .child(self.action.icon().element())
        .when_some(self.on_click, |s, handler| {
            s.on_click(move |e, w, cx| handler(e, w, cx))
        })
    }
}

/// Workspace tab; the owner controls selection and navigation.
#[derive(IntoElement)]
pub struct WorkspaceTab {
    id: ElementId,
    label: SharedString,
    icon: Option<ActionIcon>,
    active: bool,
    enabled: bool,
    on_click: Option<ClickHandler>,
    sample: Option<SampleState>,
}

impl WorkspaceTab {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            active: false,
            enabled: true,
            on_click: None,
            sample: None,
        }
    }
    pub fn icon(mut self, icon: ActionIcon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
    pub(crate) fn sample(mut self, state: SampleState) -> Self {
        self.enabled &= !matches!(state, SampleState::Disabled);
        self.sample = Some(state);
        self
    }
}

impl RenderOnce for WorkspaceTab {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let normal = Paint::new(
            t::TRANSPARENT,
            if self.active { t::TEXT } else { t::SECONDARY },
            t::TRANSPARENT,
        );
        let skin = Skin {
            normal,
            hover: Paint {
                bg: t::RAISED,
                ..normal
            },
            pressed: Paint {
                bg: t::CANVAS,
                ..normal
            },
            disabled: Paint {
                fg: t::TAB_DISABLED,
                ..normal
            },
            focus: normal,
        };
        styled_control(
            base::Button::new(self.id)
                .selected(self.active)
                .disabled(!self.enabled)
                .accessibility_label(self.label.clone()),
            skin,
            !self.enabled,
            self.sample,
        )
        .role(Role::Tab)
        .aria_selected(self.active)
        .w(px(t::TAB_WIDTH))
        .h(px(t::TOOLBAR))
        .px(px(12.))
        .gap(px(8.))
        .font_family(t::FONT)
        .text_size(px(12.))
        .line_height(px(16.))
        .font_weight(if self.active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .when_some(self.icon, |s, icon| s.child(icon.element()))
        .child(self.label)
        .when(self.active, |s| {
            s.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(2.))
                    .bg(t::FOCUS),
            )
        })
        .when_some(self.on_click, |s, handler| {
            s.on_click(move |e, w, cx| handler(e, w, cx))
        })
    }
}
