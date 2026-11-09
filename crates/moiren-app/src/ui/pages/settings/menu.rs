use rust_i18n::t;

use super::*;
use crate::ui::widgets::anchored_popover::clamped_left;
use crate::ui::widgets::occluding_page::{claim_presses, occluding_page};
use moiren_engine::audio_config::{BUFFER_SIZES, SAMPLE_RATES, SampleFormat};

/// Which value a dropdown edits. Stream fields resolve through the draft via
/// [`StreamTarget`]; the processing block edits the draft's processing
/// section directly.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueField {
    ProcessingBlock,
    StreamRate,
    StreamFormat,
    StreamBuffer,
}

/// An open dropdown: anchored where the trigger was clicked (clamped into
/// the viewport on render), living at the shell root so it can cover the
/// settings page. The state lives outside `SettingsKey` on purpose — the
/// trigger shows no open state, so opening and closing the menu must not
/// rebuild the cached page.
#[derive(Clone)]
pub(crate) struct ValueMenu {
    pub editor: Editor,
    pub target: StreamTarget,
    pub field: ValueField,
    pub at: Point<Pixels>,
}

const MENU_WIDTH: f32 = 240.;
/// Gap between item rows; row height itself comes from the button metrics
/// so the menu box math always matches what the items actually paint.
const ITEM_GAP: f32 = 2.;
const MENU_PADDING: f32 = 6.;

struct MenuItem {
    title: String,
    selected: bool,
    apply: std::rc::Rc<dyn Fn(&mut Draft)>,
}

fn menu_items(shell: &StudioShell, menu: &ValueMenu) -> Vec<MenuItem> {
    let draft = shell.settings.editor.draft(menu.editor);
    let mut items = Vec::new();
    match menu.field {
        ValueField::ProcessingBlock => {
            let rate = draft.audio.processing.sample_rate;
            for frames in BUFFER_SIZES {
                items.push(MenuItem {
                    title: fields::buffer_title(frames, rate),
                    selected: draft.audio.processing.block_frames == frames,
                    apply: std::rc::Rc::new(move |draft| {
                        draft.audio.processing.block_frames = frames
                    }),
                });
            }
        }
        ValueField::StreamRate => {
            let stream = menu.target.stream(&draft.audio);
            let target = menu.target.clone();
            items.push(MenuItem {
                title: t!("Follow processing").into_owned(),
                selected: stream.sample_rate.is_none(),
                apply: std::rc::Rc::new(move |draft| target.stream_mut(draft).sample_rate = None),
            });
            for rate in SAMPLE_RATES {
                let target = menu.target.clone();
                items.push(MenuItem {
                    title: fields::rate_title(rate),
                    selected: stream.sample_rate == Some(rate),
                    apply: std::rc::Rc::new(move |draft| {
                        target.stream_mut(draft).sample_rate = Some(rate)
                    }),
                });
            }
        }
        ValueField::StreamFormat => {
            let stream = menu.target.stream(&draft.audio);
            for format in SampleFormat::ALL {
                let target = menu.target.clone();
                items.push(MenuItem {
                    title: t!(format.title()).into_owned(),
                    selected: stream.format == format,
                    apply: std::rc::Rc::new(move |draft| target.stream_mut(draft).format = format),
                });
            }
        }
        ValueField::StreamBuffer => {
            let stream = menu.target.stream(&draft.audio);
            let effective = stream
                .sample_rate
                .unwrap_or(draft.audio.processing.sample_rate);
            let target = menu.target.clone();
            items.push(MenuItem {
                title: t!("Driver default").into_owned(),
                selected: stream.buffer_frames.is_none(),
                apply: std::rc::Rc::new(move |draft| target.stream_mut(draft).buffer_frames = None),
            });
            for frames in BUFFER_SIZES {
                let target = menu.target.clone();
                items.push(MenuItem {
                    title: fields::buffer_title(frames, effective),
                    selected: stream.buffer_frames == Some(frames),
                    apply: std::rc::Rc::new(move |draft| {
                        target.stream_mut(draft).buffer_frames = Some(frames)
                    }),
                });
            }
        }
    }
    items
}

/// The shell-root overlay behind an open [`ValueMenu`]: an occluding page
/// that closes the menu on any press outside the menu subtree (the menu
/// claims its own presses) and swallows the rest so the cached settings
/// page and the workspace underneath never see the click. Escape is not
/// wired here — the settings page peels the value menu first, then itself.
pub(crate) fn menu_overlay(
    shell: &StudioShell,
    window: &Window,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let Some(menu) = shell.settings.menu.clone() else {
        return div().into_any_element();
    };
    let items = menu_items(shell, &menu);
    let this = cx.entity();
    let viewport = window.viewport_size();
    let item = ControlSize::Small.metrics(shell.chrome.density).height;
    let height = (items.len() as f32 * (item + ITEM_GAP) - ITEM_GAP + MENU_PADDING * 2.)
        .min(f32::from(viewport.height) - 48.)
        .max(item * 3. + MENU_PADDING * 2.);
    let width = MENU_WIDTH.min(f32::from(viewport.width) - 16.);
    let left = clamped_left(f32::from(menu.at.x), width, viewport);
    let top = f32::from(menu.at.y)
        .min(f32::from(viewport.height) - height - 8.)
        .max(8.);
    let dismiss = this.clone();
    let mut list = claim_presses(
        div()
            .id("settings-menu")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .max_h(px(height))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(ITEM_GAP))
            .p(px(MENU_PADDING))
            .rounded(px(theme::RADIUS_MD))
            .border_1()
            .border_color(theme::BORDER_STRONG)
            .bg(theme::SURFACE)
            .gpui_test_observe(),
    );
    for (index, item) in items.into_iter().enumerate() {
        let this = this.clone();
        let apply = item.apply;
        list = list.child(
            button(
                SharedString::from(format!("settings-menu-item-{index}")),
                item.title,
            )
            .full_width()
            .justify_start()
            .variant(if item.selected {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Ghost
            })
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    let draft = shell.settings.editor.draft_mut(menu.editor);
                    if draft.pending.is_none() {
                        apply(draft);
                        draft.changed();
                    }
                    shell.settings.menu = None;
                    cx.notify();
                });
            }),
        );
    }
    occluding_page(
        "settings-menu-page",
        false,
        move |cx| {
            dismiss.update(cx, |shell, cx| {
                shell.settings.menu = None;
                cx.notify();
            })
        },
        list,
    )
}
