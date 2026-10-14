//! Native visual review of approved action components; no audio service starts.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::theme::controls as t;
use super::widgets::action_assets::{ActionAssets, register_fonts};
use super::widgets::action_style::SampleState;
use super::{
    ActionButton, ActionIcon, ButtonVariant, ControlSize, Density, IconButton, IconIntent,
    ToggleIconButton, WindowAction, WindowControlButton, WorkspaceTab,
};

const STATES: [SampleState; 5] = [
    SampleState::Default,
    SampleState::Hover,
    SampleState::Pressed,
    SampleState::Focus,
    SampleState::Disabled,
];

/// Launch the native component review without opening audio devices.
pub fn run_component_preview() {
    let small = std::env::args().any(|arg| arg == "--small");
    let compact = std::env::args().any(|arg| arg == "--compact");
    let group = if std::env::args().any(|arg| arg == "--group=icons") {
        Group::Icons
    } else if std::env::args().any(|arg| arg == "--group=navigation") {
        Group::Navigation
    } else {
        Group::Buttons
    };
    gpui_kit::application()
        .with_assets(ActionAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_fonts(cx).expect("register embedded Inter font");
            let dimensions = if small {
                size(px(1024.), px(720.))
            } else {
                size(px(1440.), px(900.))
            };
            let (window, _) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(dimensions, cx)),
                    window_min_size: Some(size(px(960.), px(600.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Moiren · Components".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|_| ComponentGallery {
                        density: if compact {
                            Density::Compact
                        } else {
                            Density::Comfortable
                        },
                        group,
                        ..Default::default()
                    })
                },
            )
            .expect("open component preview");
            window
                .update(cx, |_, window, cx| {
                    window.on_window_should_close(cx, |_, cx| {
                        cx.quit();
                        true
                    });
                })
                .expect("register preview close");
            cx.activate(true);
        });
}

#[derive(Default)]
pub(crate) struct ComponentGallery {
    density: Density,
    group: Group,
    clicks: usize,
    selected: bool,
    workspace: usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Group {
    #[default]
    Buttons,
    Icons,
    Navigation,
}

fn section(title: &'static str) -> Div {
    div().flex().flex_col().gap(px(8.)).child(
        div()
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(15.))
            .child(title),
    )
}

fn row(label: impl Into<SharedString>, samples: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(44.))
        .child(
            div()
                .w(px(110.))
                .flex_none()
                .text_size(px(11.))
                .text_color(t::SECONDARY)
                .child(label.into()),
        )
        .children(samples.into_iter().map(|sample| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(104.))
                .flex_none()
                .child(sample)
        }))
}

fn headings() -> Div {
    row(
        "",
        ["Default", "Hover", "Pressed", "Focus", "Disabled"]
            .into_iter()
            .map(|label| {
                div()
                    .text_size(px(10.))
                    .text_color(t::MUTED)
                    .child(label)
                    .into_any_element()
            })
            .collect(),
    )
}

impl Render for ComponentGallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = window.viewport_size().width < px(1280.);
        let mut filled = section("Button / Filled").child(headings());
        let mut quiet = section("Button / Quiet").child(headings());
        for (variant, name) in [
            (ButtonVariant::Primary, "Primary"),
            (ButtonVariant::Danger, "Danger"),
            (ButtonVariant::Secondary, "Secondary"),
            (ButtonVariant::Ghost, "Ghost"),
        ] {
            for size in [ControlSize::Small, ControlSize::Medium, ControlSize::Large] {
                let samples = STATES
                    .into_iter()
                    .enumerate()
                    .map(|(i, state)| {
                        ActionButton::new(
                            format!("{name}-{size:?}-{i}"),
                            if matches!(variant, ButtonVariant::Primary | ButtonVariant::Danger) {
                                "Apply"
                            } else {
                                "Action"
                            },
                        )
                        .variant(variant)
                        .size(size)
                        .density(self.density)
                        .icon(ActionIcon::Plus)
                        .sample(state)
                        .into_any_element()
                    })
                    .collect();
                let r = row(format!("{name} / {size:?}"), samples);
                if matches!(variant, ButtonVariant::Primary | ButtonVariant::Danger) {
                    filled = filled.child(r);
                } else {
                    quiet = quiet.child(r);
                }
            }
        }
        let mut icons = section("Icon Button / Standard").child(headings());
        for intent in [IconIntent::Neutral, IconIntent::Accent, IconIntent::Danger] {
            for size in [ControlSize::Small, ControlSize::Medium] {
                icons = icons.child(row(
                    format!("{intent:?} / {size:?}"),
                    STATES
                        .into_iter()
                        .enumerate()
                        .map(|(i, state)| {
                            IconButton::new(
                                format!("icon-{intent:?}-{size:?}-{i}"),
                                "Settings",
                                ActionIcon::Settings,
                            )
                            .intent(intent)
                            .size(size)
                            .density(self.density)
                            .sample(state)
                            .into_any_element()
                        })
                        .collect(),
                ));
            }
        }
        let mut toggles = section("Icon Button / Toggle");
        for selected in [false, true] {
            toggles = toggles.child(row(
                if selected { "Selected" } else { "Unselected" },
                STATES
                    .into_iter()
                    .enumerate()
                    .map(|(i, state)| {
                        ToggleIconButton::new(
                            format!("toggle-{selected}-{i}"),
                            "Settings",
                            ActionIcon::Settings,
                        )
                        .selected(selected)
                        .sample(state)
                        .into_any_element()
                    })
                    .collect(),
            ));
        }
        let mut windows = section("Window Button");
        for action in [
            WindowAction::Minimize,
            WindowAction::Maximize,
            WindowAction::Close,
        ] {
            windows = windows.child(row(
                format!("{action:?}"),
                STATES[..4]
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(i, state)| {
                        WindowControlButton::new(format!("window-{action:?}-{i}"), action)
                            .sample(state)
                            .into_any_element()
                    })
                    .collect(),
            ));
        }
        let tabs = section("Workspace Tab").child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    WorkspaceTab::new("tab-default", "Graph")
                        .icon(ActionIcon::Graph)
                        .sample(SampleState::Default),
                )
                .child(
                    WorkspaceTab::new("tab-hover", "Graph")
                        .icon(ActionIcon::Graph)
                        .sample(SampleState::Hover),
                )
                .child(
                    WorkspaceTab::new("tab-selected", "Graph")
                        .icon(ActionIcon::Graph)
                        .active(true)
                        .sample(SampleState::Default),
                )
                .child(
                    WorkspaceTab::new("tab-focus", "Graph")
                        .icon(ActionIcon::Graph)
                        .sample(SampleState::Focus),
                )
                .child(
                    WorkspaceTab::new("tab-disabled", "Graph")
                        .icon(ActionIcon::Graph)
                        .enabled(false),
                ),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t::CANVAS)
            .font_family(t::FONT)
            .text_color(t::TEXT)
            .text_size(px(13.))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .p(px(16.))
                    .bg(t::PANEL)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Moiren / Components"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t::SECONDARY)
                                    .child("Navigation & Actions · Figma v0.1.2"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .children(
                                [
                                    (Group::Buttons, "Buttons"),
                                    (Group::Icons, "Icons"),
                                    (Group::Navigation, "Navigation"),
                                ]
                                .into_iter()
                                .map(|(group, label)| {
                                    ActionButton::new(label, label)
                                        .variant(if self.group == group {
                                            ButtonVariant::Primary
                                        } else {
                                            ButtonVariant::Ghost
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.group = group;
                                            cx.notify();
                                        }))
                                }),
                            )
                            .child(
                                ActionButton::new("density", format!("{:?}", self.density))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.density = if this.density == Density::Comfortable {
                                            Density::Compact
                                        } else {
                                            Density::Comfortable
                                        };
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("component-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .p(px(16.))
                    .child(
                        (match self.group {
                            Group::Buttons => div().flex().gap(px(24.)).child(filled).child(quiet),
                            Group::Icons => div().flex().gap(px(24.)).child(icons).child(toggles),
                            Group::Navigation => {
                                div().flex().gap(px(24.)).child(windows).child(tabs)
                            }
                        })
                        .when(narrow, |s| s.flex_col().items_start()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(16.))
                    .p(px(12.))
                    .bg(t::PANEL)
                    .child(
                        ActionButton::new("live-action", "Apply")
                            .variant(ButtonVariant::Primary)
                            .icon(ActionIcon::Plus)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clicks += 1;
                                cx.notify();
                            })),
                    )
                    .child(
                        ToggleIconButton::new(
                            "live-toggle",
                            "Enable monitoring",
                            ActionIcon::Settings,
                        )
                        .selected(self.selected)
                        .on_change({
                            let this = cx.entity().downgrade();
                            move |selected, _, _, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.selected = selected;
                                    cx.notify();
                                });
                            }
                        }),
                    )
                    .children(
                        ["Graph", "Mixer"]
                            .into_iter()
                            .enumerate()
                            .map(|(index, label)| {
                                WorkspaceTab::new(format!("live-tab-{index}"), label)
                                    .active(self.workspace == index)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.workspace = index;
                                        cx.notify();
                                    }))
                            }),
                    )
                    .child(div().text_color(t::SECONDARY).child(format!(
                        "{} activations · selected {}",
                        self.clicks, self.selected
                    ))),
            )
    }
}
