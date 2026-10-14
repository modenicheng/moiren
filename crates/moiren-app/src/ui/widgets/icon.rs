//! Tabler icon element (MIT, see assets/icons/LICENSE.tabler.txt).
//! SVGs are embedded via include_bytes! so the binary stays self-contained.

use gpui_kit::*;

/// Load a Tabler icon by file stem, e.g. "route", "wave", "grid".
pub fn icon(name: &str) -> Svg {
    let path: SharedString = format!("icons/{name}.svg").into();
    svg().path(path)
}

/// Icon with a fixed size and tint, centered in its box.
pub fn icon_tinted(name: &str, tint: Hsla, side: f32) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(name).size(px(side)).text_color(tint))
}
