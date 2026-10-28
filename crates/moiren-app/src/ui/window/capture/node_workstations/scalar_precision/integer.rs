//! One actual frame-count field, plus bounded native Inspector reveal.
use super::{fields, inputs, pixel_bounds, rect, visible};
use crate::service::AppSnapshot;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, InputEvent, ScrollDelta, Window, point, px};
use moiren_core::graph::NodeId;

pub(super) fn verify_integer(text: &str, value: usize, maximum: usize) -> anyhow::Result<()> {
    let text = text.trim();
    anyhow::ensure!(
        !text.is_empty()
            && text.bytes().all(|b| b.is_ascii_digit())
            && text.parse::<usize>()? == value
            && (1..=maximum).contains(&value),
        "integer capture must show actual whole frames within engine limits: {text:?}, value={value}, maximum={maximum}"
    );
    Ok(())
}

pub(super) fn evidence(
    window: &Window,
    snapshot: &AppSnapshot,
    node: NodeId,
    field: fields::IntegerField,
) -> anyhow::Result<serde_json::Value> {
    let panel = pixel_bounds(window.find("graph-inspector").bounds());
    let root = visible(
        window,
        format!("integer-precision-{node:?}-{}", field.name),
        panel,
    )?;
    let native = inputs(window, &root);
    let [input] = native.as_slice() else {
        anyhow::bail!("integer field requires exactly one native Input");
    };
    let exact = visible(
        window,
        format!("integer-exact-{node:?}-{}", field.name),
        pixel_bounds(root.bounds()),
    )?;
    anyhow::ensure!(
        input.visible()
            && input.path().starts_with(exact.path())
            && super::contains_native_bounds(
                pixel_bounds(root.bounds()),
                pixel_bounds(input.bounds())
            ),
        "integer native Input escapes its precision field"
    );
    let text = input
        .value()
        .ok_or_else(|| anyhow::anyhow!("integer Input has no observed value"))?;
    verify_integer(text, field.value, field.maximum)?;
    let phase = snapshot
        .recordings
        .iter()
        .find(|r| r.node == node)
        .map(|r| format!("{:?}", r.phase));
    Ok(
        serde_json::json!({"field": field.name, "surface": "inspector", "native_input_count": 1,
        "units": "frames", "value": text, "service_value": field.value, "maximum_frames": field.maximum,
        "bounds": rect(pixel_bounds(root.bounds())), "input": rect(pixel_bounds(input.bounds())), "bounds_verified": true,
        "recording_phase": phase, "service_busy": field.busy, "observed_disabled": input.disabled(),
        "native_typing_verified": false, "busy_interaction_verified": false,
        "verification": "observed integer layout and actual service value; no input or busy-state interaction claimed"}),
    )
}

pub(in super::super) fn reveal(
    window: &mut Window,
    snapshot: &AppSnapshot,
    node: NodeId,
    cx: &mut App,
) -> anyhow::Result<()> {
    let root = if let Some(field) = fields::integer(snapshot, node) {
        format!("integer-precision-{node:?}-{}", field.name)
    } else {
        let state = snapshot
            .graph
            .node(node)
            .ok_or_else(|| anyhow::anyhow!("missing scalar node"))?;
        let (slug, _) = fields::parameters(&state.builtin)
            .ok_or_else(|| anyhow::anyhow!("missing scalar settings"))?;
        format!("{slug}-precision-{node:?}")
    };
    for _ in 0..8 {
        window.render_frame(cx);
        let panel = window.find("graph-inspector").bounds();
        let target = window
            .try_find(root.clone())
            .ok_or_else(|| anyhow::anyhow!("missing precision root {root}"))?
            .bounds();
        anyhow::ensure!(
            target.size.height <= panel.size.height
                && target.left() >= panel.left()
                && target.right() <= panel.right(),
            "precision editor cannot fit Inspector: target={target:?}, panel={panel:?}"
        );
        let delta = if target.top() < panel.top() {
            panel.top() - target.top() + px(2.)
        } else if target.bottom() > panel.bottom() {
            panel.bottom() - target.bottom() - px(2.)
        } else {
            return Ok(());
        };
        window.dispatch_event(
            gpui_kit::ScrollWheelEvent {
                position: point(panel.right() - px(2.), panel.center().y),
                delta: ScrollDelta::Pixels(point(px(0.), delta)),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    }
    anyhow::bail!("precision editor remains clipped after native Inspector scroll")
}
