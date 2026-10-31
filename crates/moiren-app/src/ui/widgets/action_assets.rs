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
    Mixer,
    Matrix,
    Devices,
    Minimize,
    Maximize,
    Close,
    Cursor,
    HandMove,
    ZoomOut,
    ZoomIn,
}

impl ActionIcon {
    /// Every variant, so the asset regression test can enumerate all request
    /// sites instead of chasing scattered call sites.
    #[cfg(test)]
    pub(crate) const ALL: [Self; 13] = [
        Self::Plus,
        Self::Settings,
        Self::Graph,
        Self::Mixer,
        Self::Matrix,
        Self::Devices,
        Self::Minimize,
        Self::Maximize,
        Self::Close,
        Self::Cursor,
        Self::HandMove,
        Self::ZoomOut,
        Self::ZoomIn,
    ];

    /// The asset path is a literal: every toolbar rebuild used to pay a
    /// format! allocation for a string that never varies per variant.
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Plus => "figma-actions/plus.svg",
            Self::Settings => "figma-actions/settings.svg",
            Self::Graph => "figma-actions/graph.svg",
            Self::Mixer => "icons/sliders.svg",
            Self::Matrix => "icons/route.svg",
            Self::Devices => "icons/devices.svg",
            Self::Minimize => "figma-actions/minimize.svg",
            Self::Maximize => "figma-actions/maximize.svg",
            Self::Close => "figma-actions/close.svg",
            Self::Cursor => "icons/cursor-arrow.svg",
            Self::HandMove => "icons/hand-move.svg",
            Self::ZoomOut => "icons/minus.svg",
            Self::ZoomIn => "icons/plus.svg",
        }
    }

    pub(crate) fn element(self) -> impl IntoElement {
        // Kit's Icon resolves inherited text color before painting Svg. A bare
        // Svg with no explicit color silently skips painting in this GPUI version.
        Icon::default()
            .path(self.path())
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
            "figma-nodes/input.svg" => {
                Some(include_bytes!("../../../assets/figma-nodes/input.svg"))
            }
            "figma-nodes/output.svg" => {
                Some(include_bytes!("../../../assets/figma-nodes/output.svg"))
            }
            "figma-nodes/fader.svg" => {
                Some(include_bytes!("../../../assets/figma-nodes/fader.svg"))
            }
            "figma-nodes/bus.svg" => Some(include_bytes!("../../../assets/figma-nodes/bus.svg")),
            "figma-nodes/more.svg" => Some(include_bytes!("../../../assets/figma-nodes/more.svg")),
            "figma-nodes/input-port.svg" => {
                Some(include_bytes!("../../../assets/figma-nodes/input-port.svg"))
            }
            "figma-nodes/output-port.svg" => Some(include_bytes!(
                "../../../assets/figma-nodes/output-port.svg"
            )),
            "brand/mark.svg" => Some(include_bytes!(
                "../../../../../assets/logos/moiren-2c-mark-light.svg"
            )),
            "icons/mic.svg" => Some(include_bytes!("../../../assets/icons/mic.svg")),
            "icons/activity.svg" => Some(include_bytes!("../../../assets/icons/activity.svg")),
            "icons/radio.svg" => Some(include_bytes!("../../../assets/icons/radio.svg")),
            "icons/sliders.svg" => Some(include_bytes!("../../../assets/icons/sliders.svg")),
            "icons/wave.svg" => Some(include_bytes!("../../../assets/icons/wave.svg")),
            "icons/app.svg" => Some(include_bytes!("../../../assets/icons/app.svg")),
            "icons/git-merge.svg" => Some(include_bytes!("../../../assets/icons/git-merge.svg")),
            "icons/clock.svg" => Some(include_bytes!("../../../assets/icons/clock.svg")),
            "icons/speaker.svg" => Some(include_bytes!("../../../assets/icons/speaker.svg")),
            "icons/route.svg" => Some(include_bytes!("../../../assets/icons/route.svg")),
            "icons/devices.svg" => Some(include_bytes!("../../../assets/icons/devices.svg")),
            "icons/chevron-left.svg" => {
                Some(include_bytes!("../../../assets/icons/chevron-left.svg"))
            }
            "icons/chevron-right.svg" => {
                Some(include_bytes!("../../../assets/icons/chevron-right.svg"))
            }
            "icons/zoom-scan.svg" => Some(include_bytes!("../../../assets/icons/zoom-scan.svg")),
            "icons/cursor-arrow.svg" => {
                Some(include_bytes!("../../../assets/icons/cursor-arrow.svg"))
            }
            "icons/hand-move.svg" => Some(include_bytes!("../../../assets/icons/hand-move.svg")),
            "icons/minus.svg" => Some(include_bytes!("../../../assets/icons/minus.svg")),
            "icons/plus.svg" => Some(include_bytes!("../../../assets/icons/plus.svg")),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon the node canvas and library can paint must resolve through
    /// this source; a name missed here falls through to the Kit bundle, which
    /// errors on every painted frame instead of skipping quietly.
    #[test]
    fn every_painted_node_icon_is_registered() {
        for template in crate::ui::pages::node_catalog::NodeTemplate::ALL {
            let path = format!("icons/{}.svg", template.icon());
            assert!(
                ActionAssets.load(&path).ok().flatten().is_some(),
                "unregistered node icon asset: {path}"
            );
        }
        for name in [
            "input",
            "output",
            "bus",
            "fader",
            "more",
            "input-port",
            "output-port",
        ] {
            let path = format!("figma-nodes/{name}.svg");
            assert!(
                ActionAssets.load(&path).ok().flatten().is_some(),
                "unregistered node icon asset: {path}"
            );
        }
    }

    /// The canvas editing toolbar paints icon-only tool and zoom buttons; a
    /// missing registration errors on every painted frame (see the node test
    /// above).
    #[test]
    fn toolbar_tool_icons_are_registered() {
        for name in ["cursor-arrow", "hand-move", "minus", "plus"] {
            let path = format!("icons/{name}.svg");
            assert!(
                ActionAssets.load(&path).ok().flatten().is_some(),
                "unregistered toolbar icon asset: {path}"
            );
        }
    }

    /// Reverse guard for the 2026-10 dead-asset prune (audit DC-3/DC-4): every
    /// svg kept in the embedded asset trees must be reachable from a real
    /// request site, and every enumerated request must resolve through this
    /// source. An asset nobody asks for is shipped binary weight with no
    /// consumer; re-add files together with the code path that paints them.
    #[test]
    fn every_icon_asset_has_a_request_path() {
        use std::collections::HashSet;

        use crate::ui::pages::node_catalog::NodeTemplate;

        // Icon names painted by the canvas node cards come from the `NodeKind`
        // match in ui/pages/graph_canvas.rs; keep this list in sync there.
        // Names with the `figma-` prefix resolve to figma-nodes/, the rest to
        // icons/ (same routing as graph_canvas_view/nodes.rs).
        const CANVAS_NODE_ICONS: &[&str] = &[
            "figma-input",
            "figma-output",
            "figma-bus",
            "figma-fader",
            "app",
            "wave",
            "radio",
            "activity",
            "clock",
            "route",
        ];
        // Assets the node painter requests straight from figma-nodes/ without
        // the name routing above: port anchors and the overflow marker.
        const FIGMA_NODE_ASSETS: &[&str] = &["input-port", "output-port", "more"];
        // Fixed paths outside the ActionIcon/canvas/catalog pipelines.
        const FIXED_REQUESTS: &[&str] = &[
            "icons/chevron-left.svg",  // sidebar collapse control
            "icons/chevron-right.svg", // sidebar collapse control
            "icons/zoom-scan.svg",     // inspector zoom-to-scan button
            "brand/mark.svg",          // toolbar brand mark
        ];

        let mut used: HashSet<String> = ActionIcon::ALL
            .iter()
            .map(|icon| icon.path().to_string())
            .collect();
        used.extend(FIXED_REQUESTS.iter().map(|path| (*path).to_string()));
        for name in CANVAS_NODE_ICONS {
            let path = match name.strip_prefix("figma-") {
                Some(rest) => format!("figma-nodes/{rest}.svg"),
                None => format!("icons/{name}.svg"),
            };
            used.insert(path);
        }
        for asset in FIGMA_NODE_ASSETS {
            used.insert(format!("figma-nodes/{asset}.svg"));
        }
        for template in NodeTemplate::ALL {
            used.insert(format!("icons/{}.svg", template.icon()));
        }

        for path in &used {
            assert!(
                ActionAssets.load(path).ok().flatten().is_some(),
                "icon request without a registered asset: {path}"
            );
        }

        // Only embedded trees are checked; assets/logos feeds build.rs' icon
        // pipeline and keeps its own provenance ledger.
        for dir in ["icons", "figma-nodes", "figma-actions"] {
            let entries = std::fs::read_dir(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets")
                    .join(dir),
            )
            .expect("embedded asset directory");
            for entry in entries {
                let name = entry.expect("asset entry").file_name();
                let name = name.to_string_lossy();
                if !name.ends_with(".svg") {
                    continue;
                }
                let path = format!("{dir}/{name}");
                assert!(
                    used.contains(&path),
                    "asset without a request path; delete it or register its consumer: {path}"
                );
            }
        }
    }
}
