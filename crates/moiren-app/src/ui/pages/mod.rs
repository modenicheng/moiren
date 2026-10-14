//! Pages: workspace views rendered inside the studio shell.

pub mod graph_canvas;
pub mod graph_canvas_view;

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::service::AppSnapshot;
use crate::ui::state::UiState;
use crate::ui::theme;
use crate::ui::widgets::status_text;
use graph_canvas::CanvasWorld;

/// Which workspace page is active (toolbar tabs).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    #[default]
    Graph,
    Mixer,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Graph => "Graph",
            Page::Mixer => "Mixer",
        }
    }
}

/// Mixer page: gain/pan knobs plus a stereo meter, fed by the live snapshot.
pub fn mixer_page(state: &UiState) -> impl IntoElement {
    let (gain, pan) = state
        .snapshot
        .desired
        .as_ref()
        .map(|spec| (spec.gain as f32, spec.pan as f32))
        .unwrap_or((0.5, 0.0));
    div()
        .id("mixer-page")
        .size_full()
        .bg(theme::WINDOW_BG)
        .p(px(theme::SPACE_5))
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_4))
        .child(
            div()
                .text_size(px(theme::TEXT_MD))
                .text_color(theme::TEXT)
                .child("Mix · Level / Balance"),
        )
        .child(
            div()
                .flex()
                .gap(px(theme::SPACE_6))
                .items_start()
                .child(crate::ui::widgets::meters::knob(
                    "Gain",
                    (gain / 16.).clamp(0., 1.),
                    theme::PRIMARY,
                ))
                .child(crate::ui::widgets::meters::knob(
                    "Pan",
                    ((pan + 1.) / 2.).clamp(0., 1.),
                    theme::SIGNAL_DEVICE,
                ))
                .child(param_readout("Gain", format!("{gain:.2}")))
                .child(param_readout("Pan", format!("{pan:+.2}")))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(theme::SPACE_1))
                        .h(px(120.))
                        .child(
                            div()
                                .text_size(px(theme::TEXT_XS))
                                .text_color(theme::TEXT_FAINT)
                                .child("Monitor"),
                        )
                        .child(crate::ui::widgets::meters::stereo_level_meters(
                            0., 0., 0., 0.,
                        )),
                ),
        )
}

fn param_readout(label: &str, value: String) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::SPACE_1))
        .child(
            div()
                .text_size(px(theme::TEXT_XS))
                .text_color(theme::TEXT_FAINT)
                .child(label.to_string()),
        )
        .child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(px(theme::TEXT_LG))
                .text_color(theme::TEXT)
                .child(value),
        )
}

/// Project the snapshot into a canvas world.
pub fn world_from_snapshot(snapshot: &Arc<AppSnapshot>) -> Arc<CanvasWorld> {
    Arc::new(CanvasWorld::candidate(snapshot))
}

/// Engine status bar, ported from EngineStatusbar.
pub fn engine_statusbar(state: &UiState) -> impl IntoElement {
    let active = state.snapshot.phase == crate::service::SessionPhase::Running;
    let phase_color = if active {
        theme::PRIMARY
    } else {
        theme::TEXT_FAINT
    };
    let elapsed = state.elapsed_text();
    let phase = state.phase_text();
    div()
        .id("engine-statusbar")
        .flex()
        .flex_none()
        .h(px(28.))
        .px(px(theme::SPACE_4))
        .items_center()
        .justify_between()
        .bg(theme::SURFACE)
        .border_t_1()
        .border_color(theme::BORDER)
        .child(
            div()
                .flex()
                .gap(px(12.))
                .items_center()
                .child(div().size(px(6.)).rounded_full().bg(phase_color))
                .child(status_text(phase, phase_color))
                .child(status_text(format!("T {elapsed}"), theme::TEXT_MUTED))
                .child(status_text(
                    format!(
                        "IO {}in/{}out",
                        state.snapshot.catalog.inputs.len(),
                        state.snapshot.catalog.outputs.len()
                    ),
                    theme::TEXT_MUTED,
                )),
        )
        .child(status_text("v0.1.0", theme::PRIMARY))
}
