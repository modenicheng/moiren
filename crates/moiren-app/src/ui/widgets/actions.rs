//! Figma Navigation & Actions translated into native, reusable gpui components.
//!
//! Base Button/Toggle own focus, Enter/Space activation and accessibility.
//! These components own the visual contract; callbacks request changes and
//! never infer that the audio engine has accepted or applied them.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::action_assets::ActionIcon;
use super::action_style::{Paint, SampleState, Skin, styled_control};
use crate::ui::theme::controls as t;
pub use t::{ControlSize, Density};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type ToggleHandler = Box<dyn Fn(bool, &ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default)]
pub enum ButtonVariant {
    Primary,
    Danger,
    #[default]
    Secondary,
    Ghost,
}

impl ButtonVariant {
    fn skin(self) -> Skin {
        let (normal, hover, pressed, disabled) = match self {
            Self::Primary => (
                Paint::new(t::PRIMARY, t::ON_ACCENT, t::BORDER),
                Paint::new(t::PRIMARY_HOVER, t::ON_ACCENT, t::BORDER),
                Paint::new(t::PRIMARY_PRESSED, t::ON_ACCENT, t::BORDER),
                Paint::new(t::PRIMARY_MUTED, t::SECONDARY, t::BORDER),
            ),
            Self::Danger => (
                Paint::new(t::DANGER, t::ON_ACCENT, t::DANGER),
                Paint::new(t::DANGER_HOVER, t::ON_ACCENT, t::DANGER),
                Paint::new(t::DANGER_PRESSED, t::ON_ACCENT, t::DANGER),
                Paint::new(t::DANGER_DISABLED, t::SECONDARY, t::DANGER),
            ),
            Self::Secondary => (
                Paint::new(t::RAISED, t::TEXT, t::BORDER),
                Paint::new(t::RAISED, t::TEXT, t::BORDER),
                Paint::new(t::CANVAS, t::TEXT, t::BORDER),
                Paint::new(t::RAISED, t::MUTED, t::BORDER),
            ),
            Self::Ghost => (
                Paint::new(t::TRANSPARENT, t::SECONDARY, t::TRANSPARENT),
                Paint::new(t::RAISED, t::SECONDARY, t::TRANSPARENT),
                Paint::new(t::CANVAS, t::SECONDARY, t::TRANSPARENT),
                Paint::new(t::TRANSPARENT, t::MUTED, t::TRANSPARENT),
            ),
        };
        Skin {
            normal,
            hover,
            pressed,
            disabled,
            focus: normal,
        }
    }
}

/// Filled (Primary/Danger) or Quiet (Secondary/Ghost) button, Figma 49:238/51:358.
#[derive(IntoElement)]
pub struct ActionButton {
    id: ElementId,
    label: SharedString,
    variant: ButtonVariant,
    size: ControlSize,
    density: Density,
    icon: Option<ActionIcon>,
    icon_only: bool,
    disabled: bool,
    on_click: Option<ClickHandler>,
    tooltip: Option<SharedString>,
    full_width: bool,
    truncate_label: bool,
    justify_start: bool,
    trailing: Option<SharedString>,
    square: bool,
    sample: Option<SampleState>,
}

impl ActionButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant: ButtonVariant::default(),
            size: ControlSize::default(),
            density: Density::default(),
            icon: None,
            icon_only: false,
            disabled: false,
            on_click: None,
            tooltip: None,
            full_width: false,
            truncate_label: false,
            justify_start: false,
            trailing: None,
            square: false,
            sample: None,
        }
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
    /// Ellipsize the label within a constrained button, preserving its full
    /// accessibility label. The caller owns any full-caption tooltip.
    pub fn truncate_label(mut self) -> Self {
        self.truncate_label = true;
        self
    }
    pub fn justify_start(mut self) -> Self {
        self.justify_start = true;
        self
    }
    pub fn trailing(mut self, text: impl Into<SharedString>) -> Self {
        self.trailing = Some(text.into());
        self
    }
    pub fn square(mut self) -> Self {
        self.square = true;
        self
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }
    pub fn density(mut self, density: Density) -> Self {
        self.density = density;
        self
    }
    pub fn icon(mut self, icon: ActionIcon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// Paint only the icon in a square hit target; the label stays available
    /// to accessibility and tooltips. Requires `icon`.
    pub fn icon_only(mut self) -> Self {
        self.icon_only = true;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
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
        self.disabled |= matches!(state, SampleState::Disabled);
        self.sample = Some(state);
        self
    }
}

impl RenderOnce for ActionButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let m = self.size.metrics(self.density);
        let icon_only = self.icon_only && self.icon.is_some();
        styled_control(
            base::Button::new(self.id)
                .disabled(self.disabled)
                .accessibility_label(self.label.clone()),
            self.variant.skin(),
            self.disabled,
            self.sample,
        )
        .h(px(m.height))
        .px(px(m.padding))
        .gap(px(m.gap))
        .font_family(t::FONT)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(m.font))
        .line_height(px(m.line_height))
        .when(self.full_width, |s| s.w_full().justify_center())
        .when(self.truncate_label, |s| s.min_w_0())
        .when(self.justify_start, |s| s.justify_start())
        .when(self.square, |s| s.rounded_none())
        .when(icon_only, |s| s.w(px(m.height)).justify_center().px(px(6.)))
        .when_some(self.icon, |s, icon| s.child(icon.element()))
        .when(!icon_only, |s| {
            if self.truncate_label {
                s.child(div().flex_1().min_w_0().truncate().child(self.label))
            } else {
                s.child(self.label)
            }
        })
        .when_some(self.trailing, |s, text| {
            if self.truncate_label {
                s.child(div().flex_none().child(text))
            } else {
                s.child(div().flex_1()).child(text)
            }
        })
        .when_some(self.tooltip, |s, text| {
            s.tooltip(move |window, cx| {
                component::tooltip::Tooltip::new(text.clone()).build(window, cx)
            })
        })
        .when_some(self.on_click, |s, handler| {
            s.on_click(move |e, w, cx| handler(e, w, cx))
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum IconIntent {
    #[default]
    Neutral,
    Accent,
    Danger,
}

impl IconIntent {
    fn skin(self) -> Skin {
        let normal = match self {
            Self::Neutral => Paint::new(t::TRANSPARENT, t::SECONDARY, t::BORDER),
            Self::Accent => Paint::new(t::PRIMARY_MUTED, t::PRIMARY, t::BORDER),
            Self::Danger => Paint::new(t::TRANSPARENT, t::DANGER, t::BORDER),
        };
        let (hover, pressed) = match self {
            Self::Neutral => (
                Paint::new(t::RAISED, t::SECONDARY, t::BORDER),
                Paint::new(t::CANVAS, t::SECONDARY, t::BORDER),
            ),
            Self::Accent => (
                Paint::new(t::PRIMARY_HOVER, t::ON_ACCENT, t::BORDER),
                Paint::new(t::PRIMARY_PRESSED, t::ON_ACCENT, t::BORDER),
            ),
            Self::Danger => (
                Paint::new(t::DANGER, t::ON_ACCENT, t::BORDER),
                Paint::new(t::DANGER, t::ON_ACCENT, t::BORDER),
            ),
        };
        Skin {
            normal,
            hover,
            pressed,
            disabled: Paint::new(
                if matches!(self, Self::Accent) {
                    t::PRIMARY_MUTED
                } else {
                    t::CONTROL_DISABLED
                },
                t::DISABLED,
                t::BORDER,
            ),
            focus: if matches!(self, Self::Accent) {
                Paint::new(t::PRIMARY, t::ON_ACCENT, t::BORDER)
            } else {
                normal
            },
        }
    }
}

/// Standard icon button, Figma 52:448. An accessible action name is required.
#[derive(IntoElement)]
pub struct IconButton {
    button: ActionButton,
    icon: ActionIcon,
    intent: IconIntent,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, icon: ActionIcon) -> Self {
        Self {
            button: ActionButton::new(id, label),
            icon,
            intent: IconIntent::Neutral,
        }
    }
    pub fn intent(mut self, intent: IconIntent) -> Self {
        self.intent = intent;
        self
    }
    pub fn size(mut self, size: ControlSize) -> Self {
        self.button.size = size;
        self
    }
    pub fn density(mut self, density: Density) -> Self {
        self.button.density = density;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.button.disabled = disabled;
        self
    }
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.button.on_click = Some(Box::new(handler));
        self
    }
    pub(crate) fn sample(mut self, state: SampleState) -> Self {
        self.button = self.button.sample(state);
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let b = self.button;
        styled_control(
            base::Button::new(b.id)
                .disabled(b.disabled)
                .accessibility_label(b.label),
            self.intent.skin(),
            b.disabled,
            b.sample,
        )
        .size(px(b.size.metrics(b.density).height))
        .child(self.icon.element())
        .when_some(b.on_click, |s, handler| {
            s.on_click(move |e, w, cx| handler(e, w, cx))
        })
    }
}

/// Controlled icon toggle, Figma 54:488. The owner must confirm selection.
#[derive(IntoElement)]
pub struct ToggleIconButton {
    id: ElementId,
    label: SharedString,
    icon: ActionIcon,
    selected: bool,
    disabled: bool,
    on_change: Option<ToggleHandler>,
    sample: Option<SampleState>,
}

impl ToggleIconButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, icon: ActionIcon) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
            selected: false,
            disabled: false,
            on_change: None,
            sample: None,
        }
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn on_change(
        mut self,
        handler: impl Fn(bool, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Box::new(handler));
        self
    }
    pub(crate) fn sample(mut self, state: SampleState) -> Self {
        self.disabled |= matches!(state, SampleState::Disabled);
        self.sample = Some(state);
        self
    }
}

impl RenderOnce for ToggleIconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let normal = if self.selected {
            Paint::new(t::PRIMARY_MUTED, t::ON_ACCENT, t::PRIMARY)
        } else {
            Paint::new(t::TRANSPARENT, t::SECONDARY, t::BORDER)
        };
        let skin = Skin {
            normal,
            hover: if self.selected {
                Paint::new(t::PRIMARY_HOVER, t::ON_ACCENT, t::PRIMARY)
            } else {
                Paint::new(t::RAISED, t::SECONDARY, t::BORDER)
            },
            pressed: if self.selected {
                Paint::new(t::PRIMARY_PRESSED, t::ON_ACCENT, t::PRIMARY)
            } else {
                Paint::new(t::CANVAS, t::SECONDARY, t::BORDER)
            },
            disabled: Paint::new(normal.bg, t::DISABLED, normal.border),
            focus: normal,
        };
        styled_control(
            base::Toggle::new(self.id)
                .pressed(self.selected)
                .disabled(self.disabled)
                .accessibility_label(self.label),
            skin,
            self.disabled,
            self.sample,
        )
        .size(px(t::HIT_TARGET))
        .child(self.icon.element())
        .when(self.selected, |s| {
            s.child(
                div()
                    .absolute()
                    .top(px(t::INDICATOR_INSET))
                    .right(px(t::INDICATOR_INSET))
                    .size(px(t::INDICATOR_SIZE))
                    .rounded_full()
                    .bg(if self.disabled {
                        t::MUTED
                    } else {
                        t::INDICATOR
                    }),
            )
        })
        .when_some(self.on_change, |s, handler| {
            s.on_change(move |value, e, w, cx| handler(value, e, w, cx))
        })
    }
}

#[cfg(test)]
mod label_tests {
    use super::ActionButton;

    #[::core::prelude::v1::test]
    fn label_truncation_is_opt_in_and_preserves_the_full_caption() {
        let caption = "A long device name with an important endpoint suffix";
        let legacy = ActionButton::new("legacy", caption);
        assert!(!legacy.truncate_label);
        let truncated = ActionButton::new("truncated", caption)
            .full_width()
            .truncate_label()
            .trailing("v");
        assert!(truncated.truncate_label);
        assert_eq!(truncated.label.as_ref(), caption);
        assert_eq!(truncated.trailing.as_deref(), Some("v"));
        assert!(truncated.tooltip.is_none());
    }
}
