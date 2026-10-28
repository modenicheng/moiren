//! Routing-mode summary; precision editing lives in the shared Mapper surface.
use super::*;

pub(super) fn summary(settings: &ChannelMapperSettings) -> AnyElement {
    let mode = if !settings.matrix.is_empty() {
        "Weighted matrix"
    } else if settings.mapping.is_empty() {
        "Identity routing"
    } else {
        "Explicit channel map"
    };
    div()
        .flex()
        .flex_col()
        .child(heading(t!("Channel mapper")))
        .child(detail(&t!("Mode"), t!(mode)))
        .into_any_element()
}
