//! Layout-only scalar evidence; exact-entry behavior belongs to native UI tests.
use super::{contains_native_bounds, pixel_bounds};
use crate::service::AppSnapshot;
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{Bounds, Role, Window};
use moiren_core::graph::NodeId;
mod fields;
mod integer;
#[cfg(test)]
mod tests;
use fields::verify_rows;
pub(super) use integer::reveal;

pub(super) fn checks_frame(name: &str) -> bool {
    let Some((density, rest)) = name.strip_prefix("nodes-").and_then(|s| s.split_once('-')) else {
        return false;
    };
    let Some((surface, slug)) = rest.split_once('-') else {
        return false;
    };
    matches!(density, "comfortable" | "compact")
        && matches!(surface, "card" | "inspector")
        && (matches!(slug, "eq" | "gate" | "limiter" | "delay")
            || (surface == "inspector" && matches!(slug, "meter" | "recorder")))
}

fn visible(window: &Window, id: String, parent: Bounds<f32>) -> anyhow::Result<ElementSnapshot> {
    let element = window
        .try_find(id.clone())
        .ok_or_else(|| anyhow::anyhow!("missing scalar control {id}"))?;
    anyhow::ensure!(
        element.visible() && contains_native_bounds(parent, pixel_bounds(element.bounds())),
        "scalar control clipped: id={id}, bounds={:?}, parent={parent:?}",
        element.bounds()
    );
    Ok(element)
}

fn inputs(window: &Window, root: &ElementSnapshot) -> Vec<ElementSnapshot> {
    gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|input| {
            input.role() == Some(Role::TextInput) && input.path().starts_with(root.path())
        })
        .collect()
}

fn verify_dial(
    caption: Bounds<f32>,
    knob: Bounds<f32>,
    input: Bounds<f32>,
    unit: Bounds<f32>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        caption.bottom() <= knob.top() + 1.
            && knob.bottom() <= input.top() + 1.
            && (caption.center().x - knob.center().x).abs() <= 2.
            && (input.center().x - knob.center().x).abs() <= 2.
            && input.right() <= unit.left() + 1.
            && (input.center().y - unit.center().y).abs() <= 2.,
        "scalar dial caption/knob/numeric/unit alignment failed: {caption:?}, {knob:?}, {input:?}, {unit:?}"
    );
    Ok(())
}

fn rect(bounds: Bounds<f32>) -> [f32; 4] {
    let size = bounds.size;
    [bounds.left(), bounds.top(), size.width, size.height]
}

pub(super) fn evidence(
    window: &Window,
    snapshot: &AppSnapshot,
    node: NodeId,
    inspector: bool,
) -> anyhow::Result<serde_json::Value> {
    if let Some(field) = fields::integer(snapshot, node) {
        anyhow::ensure!(inspector, "integer evidence requires Inspector");
        return integer::evidence(window, snapshot, node, field);
    }
    let state = snapshot
        .graph
        .node(node)
        .ok_or_else(|| anyhow::anyhow!("missing precision node"))?;
    let (slug, parameters) = fields::parameters(&state.builtin)
        .ok_or_else(|| anyhow::anyhow!("capture requires actual scalar service settings"))?;
    let mut surfaces = Vec::new();
    for surface in if inspector {
        &["card", "inspector"][..]
    } else {
        &["card"][..]
    } {
        let is_card = *surface == "card";
        let container = window.find(if is_card {
            format!("node-controls-{node:?}")
        } else {
            "graph-inspector".into()
        });
        let root = if is_card {
            container
        } else {
            let id = format!("{slug}-precision-{node:?}");
            visible(window, id, pixel_bounds(container.bounds()))?
        };
        anyhow::ensure!(
            inputs(window, &root).len() == parameters.len(),
            "{surface} {slug} requires exactly {} native precision inputs",
            parameters.len()
        );
        let root_bounds = pixel_bounds(root.bounds());
        let mut fields = Vec::new();
        let mut dials = Vec::new();
        for &(parameter, unit, actual) in &parameters {
            let key = if is_card {
                format!("card-param-{node:?}-{parameter}")
            } else {
                format!("inspector-precision-{node:?}-{parameter}")
            };
            let field = visible(window, key.clone(), root_bounds)?;
            let native = inputs(window, &field);
            let [input] = native.as_slice() else {
                anyhow::bail!("scalar field {key} requires one native Input");
            };
            let bounds = pixel_bounds(input.bounds());
            anyhow::ensure!(
                input.visible() && contains_native_bounds(pixel_bounds(field.bounds()), bounds),
                "scalar Input escapes field {key}: {bounds:?}, field={:?}",
                field.bounds()
            );
            let value = input
                .value()
                .ok_or_else(|| anyhow::anyhow!("scalar Input has no observed value: {key}"))?;
            anyhow::ensure!(
                value.trim().parse::<f64>().is_ok_and(f64::is_finite),
                "scalar Input is not finite: {key}={value:?}"
            );
            let mut evidence = serde_json::json!({"parameter": parameter, "units": unit, "value": value, "service_value": actual,
                "field": rect(pixel_bounds(field.bounds())), "input": rect(bounds)});
            if is_card {
                let field_bounds = pixel_bounds(field.bounds());
                let caption =
                    pixel_bounds(visible(window, format!("{key}-label"), field_bounds)?.bounds());
                let unit =
                    pixel_bounds(visible(window, format!("{key}-unit"), field_bounds)?.bounds());
                let knob_id = format!("card-knob-{node:?}-{parameter}");
                let knob = pixel_bounds(visible(window, knob_id, field_bounds)?.bounds());
                verify_dial(caption, knob, bounds, unit)?;
                dials.push((field_bounds, bounds));
                evidence["dial"] = serde_json::json!({"caption": rect(caption), "knob": rect(knob), "unit": rect(unit), "alignment_verified": true});
            } else {
                let exact = visible(
                    window,
                    format!("inspector-precision-input-{node:?}-{parameter}"),
                    pixel_bounds(field.bounds()),
                )?;
                anyhow::ensure!(
                    input.path().starts_with(exact.path())
                        && value.trim().parse::<f64>()? == actual,
                    "Inspector field must expose the exact service value through its native Input: {key}"
                );
            }
            fields.push(evidence);
        }
        let dial_rows = verify_rows(&dials)?;
        surfaces.push(serde_json::json!({"surface": surface, "native_input_count": parameters.len(), "bounds_verified": true, "dial_rows": dial_rows, "fields": fields}));
    }
    Ok(
        serde_json::json!({"node": slug, "surfaces": surfaces, "native_typing_verified": false, "verification": "observed native layout and service readouts only"}),
    )
}
