//! Exact 16px icons exported from the approved Figma library.

use crate::ui::theme::controls;
use gpui_kit::component::{Icon, Sizable, Size};
use gpui_kit::{App, AssetSource, IntoElement, Result, SharedString, px};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug)]
pub enum ActionIcon {
    Plus,
    Settings,
    Graph,
    Minimize,
    Maximize,
    Close,
}

impl ActionIcon {
    fn name(self) -> &'static str {
        match self {
            Self::Plus => "plus",
            Self::Settings => "settings",
            Self::Graph => "graph",
            Self::Minimize => "minimize",
            Self::Maximize => "maximize",
            Self::Close => "close",
        }
    }
    pub(crate) fn element(self) -> impl IntoElement {
        // Kit's Icon resolves inherited text color before painting Svg. A bare
        // Svg with no explicit color silently skips painting in this GPUI version.
        Icon::default()
            .path(format!("figma-actions/{}.svg", self.name()))
            .with_size(Size::Size(px(controls::ICON)))
    }
}

/// Embeds approved assets while retaining Kit assets for built-in controls.
pub(crate) struct ActionAssets;

/// Register at application startup, so font availability does not change the design.
pub(crate) fn register_fonts(cx: &App) -> Result<()> {
    cx.text_system()
        .add_fonts(vec![Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/Inter.ttf"
        ))])
}

impl AssetSource for ActionAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "brand/mark.svg" => Some(include_bytes!(
                "../../../../../assets/logos/moiren-2c-mark-light.svg"
            )),
            "icons/mic.svg" => Some(include_bytes!("../../../assets/icons/mic.svg")),
            "icons/headphones.svg" => Some(include_bytes!("../../../assets/icons/headphones.svg")),
            "icons/sliders.svg" => Some(include_bytes!("../../../assets/icons/sliders.svg")),
            "icons/wave.svg" => Some(include_bytes!("../../../assets/icons/wave.svg")),
            "icons/app.svg" => Some(include_bytes!("../../../assets/icons/app.svg")),
            "icons/git-merge.svg" => Some(include_bytes!("../../../assets/icons/git-merge.svg")),
            "icons/clock.svg" => Some(include_bytes!("../../../assets/icons/clock.svg")),
            "icons/speaker.svg" => Some(include_bytes!("../../../assets/icons/speaker.svg")),
            "icons/monitor.svg" => Some(include_bytes!("../../../assets/icons/monitor.svg")),
            "figma-actions/plus.svg" => {
                Some(include_bytes!("../../../assets/figma-actions/plus.svg"))
            }
            "figma-actions/settings.svg" => {
                Some(include_bytes!("../../../assets/figma-actions/settings.svg"))
            }
            "figma-actions/graph.svg" => {
                Some(include_bytes!("../../../assets/figma-actions/graph.svg"))
            }
            "figma-actions/minimize.svg" => {
                Some(include_bytes!("../../../assets/figma-actions/minimize.svg"))
            }
            "figma-actions/maximize.svg" => {
                Some(include_bytes!("../../../assets/figma-actions/maximize.svg"))
            }
            "figma-actions/close.svg" => {
                Some(include_bytes!("../../../assets/figma-actions/close.svg"))
            }
            _ => None,
        };
        match bytes {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path == "figma-actions" {
            Ok(
                ["plus", "settings", "graph", "minimize", "maximize", "close"]
                    .into_iter()
                    .map(|name| format!("figma-actions/{name}.svg").into())
                    .collect(),
            )
        } else {
            gpui_kit::assets::Assets.list(path)
        }
    }
}
