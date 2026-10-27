//! Stopped Tone geometry acceptance; never implies live source observations.
use super::{capture_group, contains_native_bounds, native_measurements, pixel_bounds};
use crate::service::{InputSelection, SessionPhase};
use crate::ui::{monitoring::MeterStatus, window::StudioShell};
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Bounds, Entity, InputEvent, Role, ScrollDelta, Window, point,
    px,
};
use moiren_core::graph::{NodeId, NodeKind};
use std::path::Path;

#[derive(Debug)]
struct DialBounds {
    field: Bounds<f32>,
    caption: Bounds<f32>,
    knob: Bounds<f32>,
    input: Bounds<f32>,
    unit: Bounds<f32>,
}

fn verify_dial_bounds(bounds: &DialBounds) -> anyhow::Result<()> {
    let DialBounds {
        field,
        caption,
        knob,
        input,
        unit,
    } = *bounds;
    for child in [caption, knob, input, unit] {
        anyhow::ensure!(
            child.size.width > 0. && child.size.height > 0. && contains_native_bounds(field, child),
            "Tone dial child escapes its field: {bounds:?}"
        );
    }
    anyhow::ensure!(
        caption.bottom() <= knob.top() + 1.
            && knob.bottom() <= input.top() + 1.
            && (caption.center().x - knob.center().x).abs() <= 2.
            && (input.center().x - knob.center().x).abs() <= 2.
            && input.right() <= unit.left() + 1.
            && (input.center().y - unit.center().y).abs() <= 2.,
        "Tone dial caption/numeric/unit alignment or overlap failed: {bounds:?}"
    );
    Ok(())
}

fn verify_precision_value(text: &str, frequency: f64) -> anyhow::Result<()> {
    let actual = text.trim().parse::<f64>()?;
    anyhow::ensure!(
        actual.is_finite()
            && actual > 0.
            && frequency.is_finite()
            && (actual - frequency).abs() <= 1e-9,
        "Tone precision displays {text:?}, expected {frequency} Hz"
    );
    Ok(())
}

fn stopped_frequency(shell: &StudioShell, node: NodeId, cx: &App) -> anyhow::Result<f64> {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    anyhow::ensure!(
        snapshot.phase == SessionPhase::Idle
            && snapshot.graph.active_revision.is_none()
            && !state.pending_request
            && state.dispatch_error.is_none()
            && shell.node_meters.status(node) != MeterStatus::Live
            && shell.node_meters.channel_levels(node).is_none(),
        "Tone precision capture requires stopped service and unknown readings"
    );
    match snapshot
        .graph
        .node(node)
        .and_then(|entry| entry.source.as_ref())
    {
        Some(InputSelection::Tone { frequency_hz }) => Ok(*frequency_hz),
        _ => anyhow::bail!("Tone capture source is not bound to Tone"),
    }
}

fn visible(window: &Window, id: &str, viewport: Bounds<f32>) -> anyhow::Result<ElementSnapshot> {
    let element = window
        .try_find(id.to_owned())
        .ok_or_else(|| anyhow::anyhow!("missing Tone capture element {id}"))?;
    let bounds = pixel_bounds(element.bounds());
    anyhow::ensure!(
        element.visible()
            && bounds.size.width > 0.
            && bounds.size.height > 0.
            && contains_native_bounds(viewport, bounds),
        "Tone control clipped: id={id}, visible={}, bounds={bounds:?}, viewport={viewport:?}",
        element.visible()
    );
    if let Some(overview) = window
        .try_find("graph-overview")
        .filter(|item| item.visible())
    {
        let outer = pixel_bounds(overview.bounds());
        anyhow::ensure!(
            !(outer.left() < bounds.right()
                && outer.right() > bounds.left()
                && outer.top() < bounds.bottom()
                && outer.bottom() > bounds.top()),
            "graph-overview covers Tone control {id}"
        );
    }
    Ok(element)
}

fn only_input(window: &Window, root: &ElementSnapshot) -> anyhow::Result<ElementSnapshot> {
    let mut inputs = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|input| {
            input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
        });
    let input = inputs
        .next()
        .ok_or_else(|| anyhow::anyhow!("Tone field has no native TextInput"))?;
    anyhow::ensure!(
        inputs.next().is_none()
            && input.visible()
            && contains_native_bounds(pixel_bounds(root.bounds()), pixel_bounds(input.bounds())),
        "Tone field must contain one fully visible native TextInput"
    );
    Ok(input)
}

fn rect(bounds: Bounds<f32>) -> [f32; 4] {
    [
        bounds.left(),
        bounds.top(),
        bounds.size.width,
        bounds.size.height,
    ]
}

fn card_evidence(
    window: &Window,
    node: NodeId,
    frequency: f64,
) -> anyhow::Result<serde_json::Value> {
    let body = pixel_bounds(window.find(format!("node-controls-{node:?}")).bounds());
    let id = format!("card-param-{node:?}-Frequency");
    let field = visible(window, &id, body)?;
    let field_bounds = pixel_bounds(field.bounds());
    let caption = visible(window, &format!("{id}-label"), field_bounds)?;
    let unit = visible(window, &format!("{id}-unit"), field_bounds)?;
    let knob = visible(
        window,
        &format!("card-knob-{node:?}-Frequency"),
        field_bounds,
    )?;
    let input = only_input(window, &field)?;
    let value = input
        .value()
        .ok_or_else(|| anyhow::anyhow!("Tone dial has no observed numeric value"))?;
    verify_precision_value(value, frequency)?;
    let bounds = DialBounds {
        field: field_bounds,
        caption: pixel_bounds(caption.bounds()),
        knob: pixel_bounds(knob.bounds()),
        input: pixel_bounds(input.bounds()),
        unit: pixel_bounds(unit.bounds()),
    };
    verify_dial_bounds(&bounds)?;
    Ok(
        serde_json::json!({"field": rect(bounds.field), "caption": rect(bounds.caption), "knob": rect(bounds.knob),
        "input": rect(bounds.input), "unit": rect(bounds.unit), "value": value, "units": "Hz", "alignment_verified": true}),
    )
}

pub(super) fn closed_evidence(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        let frequency = stopped_frequency(shell.read(cx), node, cx)?;
        anyhow::ensure!(window.try_find(format!("tone-precision-{node:?}")).is_none(), "baseline Tone binding disclosure must stay closed");
        Ok(serde_json::json!({"phase": "Idle", "live_observation_claimed": false, "binding_open": false,
            "frequency_hz": frequency, "card": card_evidence(window, node, frequency)?}))
    })?
}

fn reveal(window: &mut Window, target: &str, cx: &mut App) -> anyhow::Result<()> {
    for _ in 0..8 {
        window.render_frame(cx);
        let viewport = window.find("graph-inspector").bounds();
        let bounds = window
            .try_find(target.to_owned())
            .ok_or_else(|| anyhow::anyhow!("missing Tone Inspector scroll target {target}"))?
            .bounds();
        anyhow::ensure!(
            bounds.size.height <= viewport.size.height
                && bounds.left() >= viewport.left()
                && bounds.right() <= viewport.right(),
            "Tone precision cannot fit Inspector: bounds={bounds:?}, viewport={viewport:?}"
        );
        let delta = if bounds.top() < viewport.top() {
            viewport.top() - bounds.top() + px(2.)
        } else if bounds.bottom() > viewport.bottom() {
            viewport.bottom() - bounds.bottom() - px(2.)
        } else {
            return Ok(());
        };
        window.dispatch_event(
            gpui_kit::ScrollWheelEvent {
                position: point(viewport.right() - px(2.), viewport.center().y),
                delta: ScrollDelta::Pixels(point(px(0.), delta)),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    }
    anyhow::bail!("Tone Inspector control remains clipped after scrolling: {target}")
}

pub(super) async fn capture_binding(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    directory: &Path,
    density: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    let name = format!("nodes-{density}-inspector-tone-binding-stopped");
    let mut frame = capture_group(
        window,
        shell,
        directory,
        &name,
        &[NodeKind::Source],
        true,
        false,
        cx,
    )
    .await?;
    frame["tone"] = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        let frequency = stopped_frequency(shell.read(cx), node, cx)?;
        let graph = shell.read(cx).state.read(cx).snapshot.graph.clone();
        let camera = shell.read(cx).camera;
        let positions = shell.read(cx).node_positions.clone();
        let measured = native_measurements(window, &[node])?;
        let precision = format!("tone-precision-{node:?}");
        anyhow::ensure!(window.try_find(precision.clone()).is_none(), "Tone disclosure already open before native click");
        reveal(window, "inspector-binding-toggle", cx)?;
        window.click("inspector-binding-toggle", cx);
        window.render_frame(cx);
        reveal(window, &precision, cx)?;
        window.dispatch_event(gpui_kit::MouseMoveEvent { position: point(px(2.), px(2.)), pressed_button: None, modifiers: Default::default() }.to_platform_input(), cx);
        window.render_frame(cx);
        let panel = pixel_bounds(window.find("graph-inspector").bounds());
        let field = visible(window, &precision, panel)?;
        let input = only_input(window, &field)?;
        let value = input.value().ok_or_else(|| anyhow::anyhow!("Inspector Tone frequency has no observed value"))?;
        verify_precision_value(value, frequency)?;
        let evidence = serde_json::json!({"phase": "Idle", "live_observation_claimed": false, "binding_open": true, "native_disclosure_click": true,
            "frequency_hz": frequency, "card": card_evidence(window, node, frequency)?,
            "inspector": {"id": precision, "field": rect(pixel_bounds(field.bounds())), "input": rect(pixel_bounds(input.bounds())), "value": value, "bounds_verified": true},
            "graph_camera_unchanged": true});
        anyhow::ensure!(shell.read(cx).camera == camera && shell.read(cx).node_positions == positions
            && shell.read(cx).state.read(cx).snapshot.graph == graph && native_measurements(window, &[node])? == measured,
            "opening Tone binding changed graph or measured canvas");
        stopped_frequency(shell.read(cx), node, cx)?;
        window.render_to_image()?.save(directory.join(format!("{name}.png")))?;
        // Restore the baseline disclosure for the next density and other filters.
        reveal(window, "inspector-binding-toggle", cx)?;
        window.click("inspector-binding-toggle", cx);
        window.render_frame(cx);
        anyhow::ensure!(window.try_find(format!("tone-precision-{node:?}")).is_none()
            && shell.read(cx).camera == camera && shell.read(cx).node_positions == positions
            && shell.read(cx).state.read(cx).snapshot.graph == graph,
            "Tone binding close failed or mutated graph/camera");
        stopped_frequency(shell.read(cx), node, cx)?;
        Ok(evidence)
    })??;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::{DialBounds, verify_dial_bounds, verify_precision_value};
    use gpui_kit::{Bounds, point, size};

    fn dial() -> DialBounds {
        DialBounds {
            field: Bounds {
                origin: point(100., 100.),
                size: size(88., 112.),
            },
            caption: Bounds {
                origin: point(100., 100.),
                size: size(88., 28.),
            },
            knob: Bounds {
                origin: point(120., 130.),
                size: size(48., 48.),
            },
            input: Bounds {
                origin: point(112., 182.),
                size: size(64., 22.),
            },
            unit: Bounds {
                origin: point(176., 182.),
                size: size(12., 22.),
            },
        }
    }

    #[::core::prelude::v1::test]
    fn tone_capture_dial_requires_aligned_caption_numeric_and_unit_bounds() {
        assert!(verify_dial_bounds(&dial()).is_ok());
        let mut bad = dial();
        bad.input.origin.x -= 5.;
        assert!(verify_dial_bounds(&bad).is_err());
        let mut bad = dial();
        bad.unit.origin.y += 8.;
        assert!(verify_dial_bounds(&bad).is_err());
        let mut bad = dial();
        bad.unit.origin.x -= 6.;
        assert!(verify_dial_bounds(&bad).is_err());
        let mut bad = dial();
        bad.caption.size.width += 20.;
        assert!(verify_dial_bounds(&bad).is_err());
    }

    #[::core::prelude::v1::test]
    fn tone_capture_precision_must_show_the_actual_stopped_frequency() {
        assert!(verify_precision_value("440.00", 440.).is_ok());
        assert!(verify_precision_value("440", 440.).is_ok());
        for text in ["", "NaN", "inf", "0", "441", "440 Hz"] {
            assert!(verify_precision_value(text, 440.).is_err(), "{text}");
        }
    }
}
