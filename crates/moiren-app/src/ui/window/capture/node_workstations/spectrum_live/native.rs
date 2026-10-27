//! Real clicks and observed geometry; no disabled-state inference or UI injection.
use super::super::{contains_native_bounds, native_measurements, pixel_bounds, wait_for};
use super::{TONE_HZ, current_settings, evidence, wait_live};
use crate::service::{BuiltinState, GraphSnapshot, SessionPhase};
use crate::ui::{spectrum::SpectrumStatus, window::StudioShell};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Bounds, Entity, InputEvent, ScrollDelta, Window, point, px,
};
use moiren_core::graph::NodeId;
use moiren_engine::spectrum::SpectrumSettings;
use std::path::Path;

pub(super) fn verify_graph_content(
    before: &GraphSnapshot,
    after: &GraphSnapshot,
    node: NodeId,
    settings: Option<SpectrumSettings>,
) -> anyhow::Result<()> {
    let mut expected_nodes = before.nodes.clone();
    if let Some(settings) = settings {
        expected_nodes
            .iter_mut()
            .find(|row| row.id == node)
            .ok_or_else(|| anyhow::anyhow!("missing capture analyzer"))?
            .builtin = BuiltinState::SpectrumAnalyzer(settings);
    }
    anyhow::ensure!(
        before.graph == after.graph
            && expected_nodes == after.nodes
            && before.exports == after.exports
            && before.rack_epochs == after.rack_epochs
            && after.last_error.is_none()
            && after.faults.is_empty(),
        "Spectrum operation changed unrelated topology, routing, or node settings"
    );
    Ok(())
}

pub(super) async fn change_setting(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    surface: &str,
    field: &str,
    value: usize,
    cx: &mut AsyncApp,
) -> anyhow::Result<Option<serde_json::Value>> {
    let previous = current_settings(shell, node, cx)?;
    let next = match field {
        "fft" => SpectrumSettings {
            fft_size: value,
            hop_frames: previous.hop_frames.min(value),
        },
        "hop" => SpectrumSettings {
            hop_frames: value,
            ..previous
        },
        _ => anyhow::bail!("unknown capture Spectrum setting: {field}"),
    };
    anyhow::ensure!(next.valid(), "invalid requested Spectrum capture settings");
    if next == previous {
        return Ok(None);
    }
    wait_live(shell, node, previous, cx).await?;
    let (graph, old_binding, old_window, camera, positions) =
        shell.update(cx, |shell, cx| -> anyhow::Result<_> {
            let snapshot = &shell.state.read(cx).snapshot;
            Ok((
                snapshot.graph.clone(),
                snapshot
                    .signal_topics
                    .node_spectra
                    .get(&node)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("missing active spectrum binding"))?,
                shell
                    .node_spectra
                    .observation(node)
                    .ok_or_else(|| anyhow::anyhow!("missing old spectrum window"))?
                    .window,
                shell.camera,
                shell.node_positions.clone(),
            ))
        })?;
    let target = format!("{surface}-spectrum-{field}-{node:?}-{value}");
    window.update(cx, |_, window, cx| -> anyhow::Result<()> {
        window.render_frame(cx);
        if surface == "inspector" {
            reveal(window, &target, cx)?;
        }
        control_bounds(window, node, surface, &target)?;
        window.click(target.clone(), cx);
        window.render_frame(cx);
        anyhow::ensure!(
            shell.read(cx).camera == camera
                && shell.read(cx).node_positions == positions
                && shell.read(cx).node_drag.is_none(),
            "Spectrum settings click dragged a card or moved the camera"
        );
        Ok(())
    })??;
    wait_for(
        shell,
        "native FFT/hop edit confirmed active with replacement binding",
        |_, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.graph.candidate_revision > graph.candidate_revision
                && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                && snapshot
                    .signal_topics
                    .node_spectra
                    .get(&node)
                    .is_some_and(|binding| {
                        binding.settings == next && binding.topic.id() != old_binding.topic.id()
                    })
        },
        cx,
    )
    .await?;
    wait_live(shell, node, next, cx).await?;
    shell.update(cx, |shell, cx| -> anyhow::Result<_> {
        let state = shell.state.read(cx);
        let snapshot = &state.snapshot;
        verify_graph_content(&graph, &snapshot.graph, node, Some(next))?;
        anyhow::ensure!(shell.camera == camera && shell.node_positions == positions && shell.node_drag.is_none(), "Spectrum apply changed graph camera or positions");
        anyhow::ensure!(old_binding.topic.status().lifecycle == moiren_signal::TopicLifecycle::Closed, "old Spectrum topic was not retired after apply");
        let observation = shell.node_spectra.observation(node).ok_or_else(|| anyhow::anyhow!("missing applied spectrum"))?;
        anyhow::ensure!(observation.window != old_window, "settings apply retained the old spectrum window");
        let actual = &snapshot.signal_topics.node_spectra[&node];
        Ok(Some(serde_json::json!({
            "surface": surface, "field": field, "before": previous, "active": actual.settings,
            "native_click": true, "active_candidate_confirmed": true, "topic_replaced_and_old_closed": true,
            "graph_only_intended_setting_changed": true, "camera_and_positions_unchanged": true,
            "spectrum": evidence::verify_spectrum(observation.window, &observation.magnitudes, actual.settings, actual.channels, TONE_HZ)?,
        })))
    })
}

pub(super) fn select_channel(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    surface: &str,
    channel: usize,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        window.render_frame(cx);
        let before = shell.read(cx);
        let graph = before.state.read(cx).snapshot.graph.clone();
        let camera = before.camera;
        let positions = before.node_positions.clone();
        let (selected, channels) = selected_channel(window, node, surface)?;
        anyhow::ensure!(channels == 2, "capture channel readout is not stereo");
        anyhow::ensure!(selected != channel && channel < 2, "channel acceptance requires a real stereo selection change");
        let direction = if channel > selected { "next" } else { "prev" };
        let target = format!("spectrum-channel-{direction}-{surface}-{node:?}");
        if surface == "inspector" { reveal(window, &target, cx)?; }
        control_bounds(window, node, surface, &target)?;
        window.click(target, cx);
        window.render_frame(cx);
        let after = shell.read(cx);
        let state = after.state.read(cx);
        anyhow::ensure!(selected_channel(window, node, surface)? == (channel, 2), "native Spectrum channel selector did not update its readout");
        anyhow::ensure!(selected_channel(window, node, "card")? == (channel, 2), "Inspector and Card Spectrum selections disagree");
        anyhow::ensure!(!state.pending_request && state.dispatch_error.is_none() && state.snapshot.graph == graph, "channel selection dispatched a graph edit");
        anyhow::ensure!(after.camera == camera && after.node_positions == positions && after.node_drag.is_none(), "channel selection dragged a card or moved the camera");
        Ok(serde_json::json!({"surface": surface, "before_channel": selected + 1, "selected_channel": channel + 1,
            "native_click": true, "shared_selection_verified": true, "graph_unchanged": true, "camera_and_positions_unchanged": true}))
    })?
}

pub(super) fn parse_channel_readout(text: &str) -> anyhow::Result<(usize, usize)> {
    let (selected, channels) = text
        .strip_prefix("Ch ")
        .and_then(|text| text.split_once(" / "))
        .ok_or_else(|| anyhow::anyhow!("invalid Spectrum channel readout {text:?}"))?;
    let selected = selected.parse::<usize>()?;
    let channels = channels.parse::<usize>()?;
    anyhow::ensure!(
        (1..=256).contains(&channels) && (1..=channels).contains(&selected),
        "invalid Spectrum channel counts {text:?}"
    );
    Ok((selected - 1, channels))
}

fn selected_channel(
    window: &Window,
    node: NodeId,
    surface: &str,
) -> anyhow::Result<(usize, usize)> {
    let id = format!("spectrum-channel-readout-{surface}-{node:?}");
    let readout = window
        .try_find(id.clone())
        .ok_or_else(|| anyhow::anyhow!("missing Spectrum channel readout {id}"))?;
    let text = readout.label().or_else(|| readout.value()).ok_or_else(|| {
        anyhow::anyhow!(
            "{id} needs aria_label or observed value; child text alone is not test-observable"
        )
    })?;
    parse_channel_readout(text)
}

fn surface_bounds(window: &Window, node: NodeId, surface: &str) -> anyhow::Result<Bounds<f32>> {
    let id = if surface == "card" {
        format!("node-controls-{node:?}")
    } else {
        "graph-inspector".into()
    };
    let observed = window
        .try_find(id.clone())
        .ok_or_else(|| anyhow::anyhow!("missing Spectrum {surface} viewport {id}"))?;
    anyhow::ensure!(
        observed.visible(),
        "Spectrum {surface} viewport is not visible"
    );
    Ok(pixel_bounds(observed.bounds()))
}

fn control_bounds(
    window: &Window,
    node: NodeId,
    surface: &str,
    target: &str,
) -> anyhow::Result<Bounds<f32>> {
    let viewport = surface_bounds(window, node, surface)?;
    let control = window
        .try_find(target.to_owned())
        .ok_or_else(|| anyhow::anyhow!("missing Spectrum control {target}"))?;
    let bounds = pixel_bounds(control.bounds());
    anyhow::ensure!(
        control.visible()
            && bounds.size.width > 0.
            && bounds.size.height > 0.
            && contains_native_bounds(viewport, bounds),
        "Spectrum control is clipped: id={target}, visible={}, control={bounds:?}, viewport={viewport:?}",
        control.visible()
    );
    no_overlay(window, bounds, target)?;
    Ok(bounds)
}

fn no_overlay(window: &Window, target: Bounds<f32>, label: &str) -> anyhow::Result<()> {
    if let Some(overlay) = window
        .try_find("graph-overview")
        .filter(|overlay| overlay.visible())
    {
        let bounds = pixel_bounds(overlay.bounds());
        anyhow::ensure!(
            !(bounds.left() < target.right()
                && bounds.right() > target.left()
                && bounds.top() < target.bottom()
                && bounds.bottom() > target.top()),
            "graph-overview covers Spectrum {label}: overview={bounds:?}, target={target:?}"
        );
    }
    Ok(())
}

fn reveal(window: &mut Window, target: &str, cx: &mut App) -> anyhow::Result<()> {
    for _ in 0..8 {
        window.render_frame(cx);
        let viewport = window.find("graph-inspector").bounds();
        let control = window
            .try_find(target.to_owned())
            .ok_or_else(|| anyhow::anyhow!("missing Spectrum scroll target {target}"))?;
        let bounds = control.bounds();
        anyhow::ensure!(
            bounds.size.height <= viewport.size.height
                && bounds.left() >= viewport.left()
                && bounds.right() <= viewport.right(),
            "Spectrum control cannot fit the Inspector: target={bounds:?}, viewport={viewport:?}"
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
    anyhow::bail!("Spectrum target remained clipped after Inspector scrolling: {target}")
}

pub(super) fn control_ids(node: NodeId, surface: &str, settings: SpectrumSettings) -> Vec<String> {
    let mut ids = [
        format!("spectrum-channel-prev-{surface}-{node:?}"),
        format!("spectrum-channel-next-{surface}-{node:?}"),
        format!("spectrum-channel-readout-{surface}-{node:?}"),
    ]
    .into_iter()
    .chain([60, 90, 120].map(|range| format!("spectrum-range-{surface}-{node:?}-{range}")))
    .chain(
        [512, 1024, 2048, 4096, 8192].map(|fft| format!("{surface}-spectrum-fft-{node:?}-{fft}")),
    )
    .chain(
        [
            settings.fft_size / 4,
            settings.fft_size / 2,
            settings.fft_size,
        ]
        .map(|hop| format!("{surface}-spectrum-hop-{node:?}-{hop}")),
    )
    .collect::<Vec<_>>();
    if surface == "inspector" {
        ids.extend(["Fft", "Hop"].map(|field| format!("spectrum-exact-{node:?}-{field}")));
    }
    ids
}

pub(super) fn frame_evidence(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    surface: &str,
    running: bool,
    path: &Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        window.render_frame(cx);
        let camera = shell.read(cx).camera;
        let positions = shell.read(cx).node_positions.clone();
        let measured = native_measurements(window, &[node])?;
        let settings = shell.read(cx).state.read(cx).snapshot.graph.node(node).and_then(|row| row.builtin.spectrum()).copied().ok_or_else(|| anyhow::anyhow!("missing Spectrum settings"))?;
        if surface == "inspector" {
            // Precision inputs follow the presets. Reveal the final field, then
            // require the plot and every control to fit in the same native frame.
            let ids = control_ids(node, surface, settings);
            reveal(window, ids.last().expect("Spectrum Inspector controls"), cx)?;
        }
        window.dispatch_event(gpui_kit::MouseMoveEvent { position: point(px(2.), px(2.)), pressed_button: None, modifiers: Default::default() }.to_platform_input(), cx);
        window.render_frame(cx);
        let view = shell.read(cx);
        anyhow::ensure!(view.camera == camera && view.node_positions == positions && native_measurements(window, &[node])? == measured, "Spectrum Inspector reveal changed measured graph geometry");
        let surfaces = if surface == "inspector" { vec!["card", "inspector"] } else { vec!["card"] };
        let mut geometry = Vec::new();
        for surface in surfaces {
            let plot = control_bounds(window, node, surface, &format!("spectrum-plot-{surface}-{node:?}"))?;
            let mut controls = Vec::new();
            for id in control_ids(node, surface, settings) {
                let bounds = control_bounds(window, node, surface, &id)?;
                controls.push(serde_json::json!({"id": id, "bounds": [bounds.left(), bounds.top(), bounds.size.width, bounds.size.height]}));
            }
            no_overlay(window, surface_bounds(window, node, surface)?, surface)?;
            geometry.push(serde_json::json!({"surface": surface, "plot": [plot.left(), plot.top(), plot.size.width, plot.size.height], "controls": controls}));
        }
        let snapshot = &view.state.read(cx).snapshot;
        let status = view.node_spectra.status(node, snapshot);
        let spectrum = if running {
            anyhow::ensure!(status == SpectrumStatus::Live, "Spectrum lost live status before capture");
            let observation = view.node_spectra.observation(node).ok_or_else(|| anyhow::anyhow!("live Spectrum has no observation"))?;
            let binding = snapshot.signal_topics.node_spectra.get(&node).ok_or_else(|| anyhow::anyhow!("live Spectrum has no active binding"))?;
            anyhow::ensure!(binding.settings == settings && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision), "Spectrum settings are not active before capture");
            serde_json::to_value(evidence::verify_spectrum(observation.window, &observation.magnitudes, binding.settings, binding.channels, TONE_HZ)?)?
        } else {
            anyhow::ensure!(status == SpectrumStatus::Stopped && view.node_spectra.observation(node).is_none() && !snapshot.signal_topics.node_spectra.contains_key(&node), "stopped Spectrum retained a trace or active binding");
            serde_json::Value::Null
        };
        let (channel, channels) = selected_channel(window, node, surface)?;
        anyhow::ensure!(channels == 2 && selected_channel(window, node, "card")? == (channel, channels), "Spectrum channel readouts disagree with real stereo source");
        let result = serde_json::json!({"status": format!("{status:?}"), "observation": spectrum,
            "selected_channel": channel + 1, "geometry": geometry,
            "plot_and_controls_inside_viewports": true, "overview_clear": true,
            "camera_and_positions_unchanged_by_scroll": true});
        // Replace capture_group's frame only after final observation/scroll checks.
        window.render_to_image()?.save(path)?;
        Ok(result)
    })?
}
