//! Native Mapper acceptance: real 32-channel routing, bounded pages, and presets.
use super::{
    chain_fixture, contains_native_bounds, dispatch, meter_scroll_geometry, native_measurements,
    pixel_bounds, prepare_group, settle_measured_focus, verify_meter_scroll_end,
    verify_native_bounds, wait_for,
};
use crate::service::{
    AppRequest, BuiltinState, GraphEdit, GraphSnapshot, ServiceGraph, SessionPhase,
};
use crate::ui::{
    monitoring::MeterStatus,
    window::{Camera, StudioShell},
};
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Bounds, Entity, InputEvent, Role, ScrollDelta, Window, point,
    px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::processor::ChannelMapperSettings;
use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};

const CHANNELS: usize = 32;
const PAGE_WIDTH: usize = 4;
const LAST_PAGE: usize = CHANNELS - PAGE_WIDTH;
const FRAME_COUNT: usize = 16;

fn signed_settings() -> ChannelMapperSettings {
    let mut matrix = (0..CHANNELS)
        .map(|output| {
            (0..CHANNELS)
                .map(|input| f64::from(u8::from(input == output)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    matrix[0][0] = -2.5;
    matrix[1][1] = 3.25;
    matrix[CHANNELS - 1][CHANNELS - 1] = -1.75;
    matrix[CHANNELS - 1][CHANNELS - 2] = 2.25;
    // An off-page route catches accidental replacement with the visible tile.
    matrix[5][12] = 0.375;
    ChannelMapperSettings {
        mapping: Vec::new(),
        matrix,
    }
}

pub(super) fn fixture() -> anyhow::Result<GraphSnapshot> {
    let snapshot = chain_fixture(
        &[NodeKind::Source, NodeKind::ChannelMapper, NodeKind::Sink],
        CHANNELS,
    )?;
    let node = snapshot
        .node_of_kind(NodeKind::ChannelMapper)
        .expect("capture Mapper")
        .0
        .id();
    let mut graph = ServiceGraph::default();
    graph
        .replace(snapshot.graph, snapshot.nodes)
        .map_err(anyhow::Error::msg)?;
    graph
        .edit(GraphEdit::SetBuiltin {
            node,
            state: BuiltinState::ChannelMapper(signed_settings()),
        })
        .map_err(anyhow::Error::msg)?;
    graph
        .preparation()
        .engine_project(256)
        .map_err(anyhow::Error::msg)?;
    Ok(graph.snapshot())
}

fn preset_settings(swap: bool) -> ChannelMapperSettings {
    let mut settings = ChannelMapperSettings::identity(CHANNELS);
    if swap {
        settings.mapping.swap(0, 1);
    }
    settings
}

fn weight(settings: &ChannelMapperSettings, output: usize, input: usize) -> f64 {
    settings.matrix.get(output).map_or_else(
        || {
            f64::from(u8::from(
                settings.mapping.get(output).copied().unwrap_or(output) == input,
            ))
        },
        |row| row[input],
    )
}

fn root(surface: &str, node: NodeId) -> String {
    format!("mapper-{surface}-{node:?}")
}

fn cell(surface: &str, node: NodeId, output: usize, input: usize) -> String {
    format!("{}-weight-{output}-{input}", root(surface, node))
}

struct Fence {
    camera: Camera,
    positions: HashMap<NodeId, (f32, f32)>,
    selected: Option<NodeId>,
    graph: Arc<GraphSnapshot>,
}

impl Fence {
    fn new(shell: &StudioShell, cx: &App) -> Self {
        Self {
            camera: shell.camera,
            positions: shell.node_positions.clone(),
            selected: shell.selected_node,
            graph: shell.state.read(cx).snapshot.graph.clone(),
        }
    }

    fn motion_unchanged(&self, shell: &StudioShell) -> anyhow::Result<()> {
        anyhow::ensure!(
            shell.camera == self.camera
                && shell.node_positions == self.positions
                && shell.selected_node == self.selected
                && shell.node_drag.is_none(),
            "Mapper interaction changed camera, selection, or card positions"
        );
        Ok(())
    }

    fn graph_unchanged(&self, shell: &StudioShell, cx: &App) -> anyhow::Result<()> {
        self.motion_unchanged(shell)?;
        let state = shell.state.read(cx);
        anyhow::ensure!(
            !state.pending_request && state.snapshot.graph == self.graph,
            "Mapper presentation-only interaction dispatched or changed the graph"
        );
        Ok(())
    }
}

fn actual_settings(
    shell: &StudioShell,
    node: NodeId,
    cx: &App,
) -> anyhow::Result<ChannelMapperSettings> {
    let state = shell.state.read(cx);
    anyhow::ensure!(
        !state.pending_request
            && state.dispatch_error.is_none()
            && state.snapshot.phase == SessionPhase::Running
            && state.snapshot.graph.active_revision
                == Some(state.snapshot.graph.candidate_revision)
            && shell.node_meters.status(node) == MeterStatus::Live
            && shell
                .node_meters
                .channel_levels(node)
                .is_some_and(|levels| levels.len() == CHANNELS),
        "Mapper requires an active confirmed plan and all 32 real channel readings"
    );
    state
        .snapshot
        .graph
        .node(node)
        .and_then(|node| node.builtin.channel_mapper())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing real Mapper settings"))
}

fn page_origin(window: &Window, surface: &str, node: NodeId) -> anyhow::Result<(usize, usize)> {
    let mut origins = Vec::new();
    for output in (0..CHANNELS).step_by(PAGE_WIDTH) {
        for input in (0..CHANNELS).step_by(PAGE_WIDTH) {
            if window
                .try_find(cell(surface, node, output, input))
                .is_some()
            {
                origins.push((output, input));
            }
        }
    }
    anyhow::ensure!(
        origins.len() == 1,
        "{surface} Mapper must render exactly one 4x4 page: {origins:?}"
    );
    Ok(origins[0])
}

fn reveal(window: &mut Window, surface: &str, node: NodeId, cx: &mut App) -> anyhow::Result<()> {
    if surface != "inspector" {
        return Ok(());
    }
    for _ in 0..8 {
        window.render_frame(cx);
        let panel = window.find("graph-inspector").bounds();
        let editor = window
            .try_find(root(surface, node))
            .ok_or_else(|| anyhow::anyhow!("missing Mapper Inspector editor"))?
            .bounds();
        anyhow::ensure!(
            editor.size.height <= panel.size.height
                && editor.left() >= panel.left()
                && editor.right() <= panel.right(),
            "Mapper editor cannot fit Inspector: editor={editor:?}, panel={panel:?}"
        );
        let delta = if editor.top() < panel.top() {
            panel.top() - editor.top() + px(2.)
        } else if editor.bottom() > panel.bottom() {
            panel.bottom() - editor.bottom() - px(2.)
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
    anyhow::bail!("Mapper Inspector remains clipped after native scrolling")
}

fn visible_control(
    window: &Window,
    id: String,
    parent: Bounds<f32>,
) -> anyhow::Result<ElementSnapshot> {
    let control = window
        .try_find(id.clone())
        .ok_or_else(|| anyhow::anyhow!("missing Mapper control {id}"))?;
    anyhow::ensure!(
        control.visible() && contains_native_bounds(parent, pixel_bounds(control.bounds())),
        "Mapper control clipped: {id}, bounds={:?}, parent={parent:?}",
        control.bounds()
    );
    Ok(control)
}

fn click(
    window: &mut Window,
    surface: &str,
    node: NodeId,
    suffix: &str,
    cx: &mut App,
) -> anyhow::Result<()> {
    reveal(window, surface, node, cx)?;
    window.render_frame(cx);
    let editor = window.find(root(surface, node));
    let id = format!("{}-{suffix}", root(surface, node));
    visible_control(window, id.clone(), pixel_bounds(editor.bounds()))?;
    // base::Button exposes no disabled observation. Native behavior and the
    // ensuing real service receipt, not missing metadata, prove acceptance.
    window.click(id, cx);
    window.render_frame(cx);
    Ok(())
}

async fn page_to(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    surface: &str,
    target: usize,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    for (axis, coordinate) in [("output", 0), ("input", 1)] {
        for _ in 0..CHANNELS / PAGE_WIDTH {
            let current = window.update(cx, |_, window, cx| {
                window.render_frame(cx);
                page_origin(window, surface, node)
            })??;
            let at = if coordinate == 0 {
                current.0
            } else {
                current.1
            };
            if at == target {
                break;
            }
            let direction = if at < target { "next" } else { "prev" };
            let other = if surface == "card" {
                "inspector"
            } else {
                "card"
            };
            let (fence, other_page) =
                window.update(cx, |_, window, cx| -> anyhow::Result<_> {
                    let other_page = window
                        .try_find(root(other, node))
                        .map(|_| page_origin(window, other, node))
                        .transpose()?;
                    let fence = Fence::new(shell.read(cx), cx);
                    click(window, surface, node, &format!("{axis}-{direction}"), cx)?;
                    fence.graph_unchanged(shell.read(cx), cx)?;
                    Ok((fence, other_page))
                })??;
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
            window.update(cx, |_, window, cx| -> anyhow::Result<()> {
                window.render_frame(cx);
                fence.graph_unchanged(shell.read(cx), cx)?;
                let after = page_origin(window, surface, node)?;
                let expected = if direction == "next" {
                    at + PAGE_WIDTH
                } else {
                    at - PAGE_WIDTH
                };
                anyhow::ensure!(
                    if coordinate == 0 {
                        after == (expected, current.1)
                    } else {
                        after == (current.0, expected)
                    },
                    "Mapper {axis} page did not advance by four: {current:?} -> {after:?}"
                );
                if let Some(other_page) = other_page {
                    anyhow::ensure!(
                        page_origin(window, other, node)? == other_page,
                        "Mapper paging changed the other surface's independent page"
                    );
                }
                Ok(())
            })??;
        }
    }
    anyhow::ensure!(
        window.update(cx, |_, window, _| page_origin(window, surface, node))?? == (target, target),
        "Mapper did not reach requested page {target}"
    );
    Ok(())
}

async fn confirm_edit(
    shell: &Entity<StudioShell>,
    node: NodeId,
    before: &GraphSnapshot,
    expected: &ChannelMapperSettings,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    wait_for(
        shell,
        "Mapper native edit confirmed active",
        |_, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.graph.candidate_revision > before.candidate_revision
                && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                && snapshot
                    .graph
                    .node(node)
                    .and_then(|node| node.builtin.channel_mapper())
                    == Some(expected)
        },
        cx,
    )
    .await?;
    shell.update(cx, |shell, cx| -> anyhow::Result<()> {
        let graph = &shell.state.read(cx).snapshot.graph;
        let mut expected_nodes = before.nodes.clone();
        expected_nodes
            .iter_mut()
            .find(|entry| entry.id == node)
            .expect("Mapper node")
            .builtin = BuiltinState::ChannelMapper(expected.clone());
        anyhow::ensure!(
            graph.graph == before.graph
                && graph.nodes == expected_nodes
                && graph.exports == before.exports
                && graph.rack_epochs == before.rack_epochs,
            "Mapper edit changed unrelated topology, settings, bindings, or exports"
        );
        Ok(())
    })
}

async fn reset_signed(
    shell: &Entity<StudioShell>,
    node: NodeId,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let before = shell.update(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
    let expected = signed_settings();
    if before
        .node(node)
        .and_then(|node| node.builtin.channel_mapper())
        == Some(&expected)
    {
        return Ok(());
    }
    dispatch(
        shell,
        AppRequest::EditGraph(GraphEdit::SetBuiltin {
            node,
            state: BuiltinState::ChannelMapper(expected.clone()),
        }),
        cx,
    )?;
    confirm_edit(shell, node, &before, &expected, cx).await
}

fn only_input(window: &Window, wrapper: &ElementSnapshot) -> anyhow::Result<ElementSnapshot> {
    let mut inputs = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|entry| {
            entry.role() == Some(Role::TextInput) && entry.path().starts_with(wrapper.path())
        });
    let input = inputs
        .next()
        .ok_or_else(|| anyhow::anyhow!("Mapper cell has no native precision input"))?;
    anyhow::ensure!(
        inputs.next().is_none()
            && input.visible()
            && contains_native_bounds(pixel_bounds(wrapper.bounds()), pixel_bounds(input.bounds())),
        "Mapper cell must contain exactly one fully visible native input"
    );
    Ok(input)
}

async fn edit_last_cell(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    surface: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let (fence, expected) = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        reveal(window, surface, node, cx)?;
        let mut expected = actual_settings(shell.read(cx), node, cx)?;
        let fence = Fence::new(shell.read(cx), cx);
        let id = cell(surface, node, CHANNELS - 1, CHANNELS - 1);
        let editor = pixel_bounds(window.find(root(surface, node)).bounds());
        let wrapper = visible_control(window, id.clone(), editor)?;
        let input = only_input(window, &wrapper)?;
        window.click_at(id, input.bounds().center() - wrapper.bounds().origin, cx);
        window.press("ctrl-a", cx);
        window.input("-3.125", cx);
        window.press("enter", cx);
        fence.motion_unchanged(shell.read(cx))?;
        expected.matrix[CHANNELS - 1][CHANNELS - 1] = -3.125;
        Ok((fence, expected))
    })??;
    confirm_edit(shell, node, &fence.graph, &expected, cx).await?;
    shell.update(cx, |shell, _| fence.motion_unchanged(shell))?;
    Ok(())
}

async fn choose_preset(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    surface: &str,
    swap: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let fence = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        let fence = Fence::new(shell.read(cx), cx);
        click(
            window,
            surface,
            node,
            if swap { "swap" } else { "identity" },
            cx,
        )?;
        fence.motion_unchanged(shell.read(cx))?;
        Ok(fence)
    })??;
    confirm_edit(shell, node, &fence.graph, &preset_settings(swap), cx).await?;
    shell.update(cx, |shell, _| fence.motion_unchanged(shell))?;
    Ok(())
}

fn verify_value(text: &str, expected: f64) -> anyhow::Result<()> {
    let value = text.trim().parse::<f64>()?;
    anyhow::ensure!(
        value.is_finite() && (value - expected).abs() <= 1e-9,
        "Mapper displays {text:?}, expected signed linear weight {expected}"
    );
    Ok(())
}

fn verify_meter_bounds(
    body: Bounds<f32>,
    meter: Bounds<f32>,
    viewport: Bounds<f32>,
    content: Bounds<f32>,
) -> anyhow::Result<()> {
    let cap = crate::ui::widgets::meters::node_meter_width(4);
    anyhow::ensure!(
        contains_native_bounds(body, meter)
            && contains_native_bounds(meter, viewport)
            && (meter.top() - body.top()).abs() <= 1.
            && (meter.bottom() - body.bottom()).abs() <= 1.
            && (meter.size.width - cap).abs() <= 1.
            && (viewport.size.width - cap).abs() <= 1.
            && content.size.width >= CHANNELS as f32 * 38.
            && (content.top() - viewport.top()).abs() <= 1.
            && (content.size.height - viewport.size.height).abs() <= 1.,
        "Mapper meter must follow full body with four-channel viewport and all 32 natural channels: body={body:?}, meter={meter:?}, viewport={viewport:?}, content={content:?}"
    );
    Ok(())
}

fn editor_evidence(
    window: &Window,
    surface: &str,
    node: NodeId,
    settings: &ChannelMapperSettings,
    body: Bounds<f32>,
) -> anyhow::Result<serde_json::Value> {
    let viewport = if surface == "card" {
        body
    } else {
        pixel_bounds(window.find("graph-inspector").bounds())
    };
    let editor = visible_control(window, root(surface, node), viewport)?;
    let bounds = pixel_bounds(editor.bounds());
    let origin = page_origin(window, surface, node)?;
    let mut cells = Vec::new();
    for output in origin.0..origin.0 + PAGE_WIDTH {
        for input in origin.1..origin.1 + PAGE_WIDTH {
            let wrapper = visible_control(window, cell(surface, node, output, input), bounds)?;
            let field = only_input(window, &wrapper)?;
            let text = field
                .value()
                .ok_or_else(|| anyhow::anyhow!("Mapper field has no observed value"))?;
            verify_value(text, weight(settings, output, input))?;
            cells.push(serde_json::json!({"output": output + 1, "input": input + 1, "value": text, "bounds": rect(pixel_bounds(field.bounds()))}));
        }
    }
    let controls = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|entry| entry.path().starts_with(editor.path()) && entry.role().is_some())
        .collect::<Vec<_>>();
    anyhow::ensure!(
        controls
            .iter()
            .filter(|entry| entry.role() == Some(Role::TextInput))
            .count()
            == PAGE_WIDTH * PAGE_WIDTH,
        "Mapper must render exactly sixteen precision inputs, not the whole matrix"
    );
    for control in controls {
        anyhow::ensure!(
            control.visible() && contains_native_bounds(bounds, pixel_bounds(control.bounds())),
            "Mapper control overflows editor: path={:?}, control={:?}, editor={bounds:?}",
            control.path(),
            control.bounds()
        );
    }
    if let Some(overview) = window
        .try_find("graph-overview")
        .filter(|item| item.visible())
    {
        let overlay = pixel_bounds(overview.bounds());
        for target in [bounds, body] {
            anyhow::ensure!(
                !(overlay.left() < target.right()
                    && overlay.right() > target.left()
                    && overlay.top() < target.bottom()
                    && overlay.bottom() > target.top()),
                "graph-overview overlaps Mapper body/editor"
            );
        }
    }
    Ok(
        serde_json::json!({"surface": surface, "bounds": rect(bounds), "outputs": [origin.0 + 1, origin.0 + PAGE_WIDTH],
        "inputs": [origin.1 + 1, origin.1 + PAGE_WIDTH], "cells": cells, "control_bounds_verified": true}),
    )
}

fn rect(bounds: Bounds<f32>) -> [f32; 4] {
    [
        bounds.left(),
        bounds.top(),
        bounds.size.width,
        bounds.size.height,
    ]
}

async fn meter_position(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    end: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        let fence = Fence::new(shell.read(cx), cx);
        window.within(format!("card-meter-{node:?}")).scroll("node-meter-viewport",
            ScrollDelta::Pixels(point(px(100_000.), px(0.))), cx);
        window.render_frame(cx);
        let (viewport, before) = meter_scroll_geometry(window, node)?;
        anyhow::ensure!((before.left() - viewport.left()).abs() <= 1., "Mapper meter did not reset to channel 1");
        let last = if end {
            window.within(format!("card-meter-{node:?}")).scroll("node-meter-viewport",
                ScrollDelta::Pixels(point(px(-100_000.), px(0.))), cx);
            window.render_frame(cx);
            let (after_viewport, after) = meter_scroll_geometry(window, node)?;
            anyhow::ensure!(after_viewport == viewport, "Mapper scrolling resized meter viewport");
            Some(verify_meter_scroll_end(viewport, before, after)?)
        } else { None };
        fence.graph_unchanged(shell.read(cx), cx)?;
        Ok(serde_json::json!({"last_channel_fully_visible": end, "last_channel_cell": last.map(rect), "camera_unchanged": true}))
    })?
}

#[allow(clippy::too_many_arguments)]
async fn save_frame(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    directory: &Path,
    density: &str,
    surface: &str,
    stage: &str,
    scroll: serde_json::Value,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    let fence = shell.update(cx, |shell, cx| Fence::new(shell, cx));
    settle_measured_focus(window, shell, &[NodeKind::ChannelMapper], &[node], cx).await?;
    let name = format!("nodes-{density}-{surface}-mapper-editing-{stage}");
    let frame = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
        reveal(window, surface, node, cx)?;
        window.dispatch_event(gpui_kit::MouseMoveEvent {
            position: point(px(2.), px(2.)), pressed_button: None, modifiers: Default::default(),
        }.to_platform_input(), cx);
        window.render_frame(cx);
        fence.graph_unchanged(shell.read(cx), cx)?;
        let view = shell.read(cx);
        let settings = actual_settings(view, node, cx)?;
        let measured = native_measurements(window, &[node])?;
        let canvas = pixel_bounds(window.find("graph-canvas-input").bounds());
        let body = Bounds { origin: measured.bodies[&node].origin + canvas.origin, size: measured.bodies[&node].size };
        let editor = editor_evidence(window, surface, node, &settings, body)?;
        let meter = pixel_bounds(window.find(format!("card-meter-{node:?}")).bounds());
        let (viewport, content) = meter_scroll_geometry(window, node)?;
        verify_meter_bounds(body, meter, viewport, content)?;
        if stage == "signed-last-edited" {
            let last = Bounds { origin: point(content.right() - 38., content.top()), size: size(38., content.size.height) };
            anyhow::ensure!((content.right() - viewport.right()).abs() <= 1. && contains_native_bounds(viewport, last),
                "Mapper channel 32 was scrolled away before saving its native frame");
        }
        let mut geometry = verify_native_bounds(&view.state.read(cx).snapshot, &view.node_positions,
            view.camera, &[node], &measured)?;
        // Narrowly record the stricter Mapper-only viewport verification.
        geometry[0]["vertical_meter_verified"] = true.into();
        window.render_to_image()?.save(directory.join(format!("{name}.png")))?;
        Ok(serde_json::json!({"file": format!("{name}.png"), "surface": surface, "stage": stage,
            "viewport": [f32::from(window.viewport_size().width), f32::from(window.viewport_size().height)],
            "zoom": view.camera.zoom, "phase": "Running", "channels": CHANNELS, "live_channels": CHANNELS,
            "editor": editor, "measured_geometry": geometry, "bounds_verified": true,
            "vertical_meter_verified": true, "meter_viewport": rect(viewport), "meter_content": rect(content), "meter_scroll": scroll,
            "camera_unchanged": true, "active_candidate_confirmed": true,
            "native_typing_verified": stage == "signed-last-edited", "settings": settings}))
    })??;
    eprintln!(
        "Mapper capture {}: saved",
        directory.join(format!("{name}.png")).display()
    );
    Ok(frame)
}

pub(super) async fn capture_mapper(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let node = shell.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node_of_kind(NodeKind::ChannelMapper)
            .expect("capture Mapper")
            .0
            .id()
    });
    let path = directory.join("node-workstations-mapper-editing.json");
    let mut manifest = serde_json::json!({"filter": "mapper-editing", "backend": "FakeBackend (real service, engine and subscriptions)",
        "preferences": "isolated defaults; persistence disabled", "channels": CHANNELS, "tile": [PAGE_WIDTH, PAGE_WIDTH],
        "expected_frames": FRAME_COUNT, "captures": [], "complete": false,
        "typing": "Native focused Input with ctrl-a, text dispatch, Enter; entire service matrix and active receipt checked"});
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest)?)?;
    for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        for surface in ["card", "inspector"] {
            reset_signed(shell, node, cx).await?;
            prepare_group(
                window,
                shell,
                &[NodeKind::ChannelMapper],
                surface == "inspector",
                true,
                cx,
            )
            .await?;
            page_to(window, shell, node, surface, 0, cx).await?;
            for stage in ["signed-first", "signed-last-edited", "swap", "identity"] {
                match stage {
                    "signed-last-edited" => {
                        page_to(window, shell, node, surface, LAST_PAGE, cx).await?;
                        edit_last_cell(window, shell, node, surface, cx).await?;
                    }
                    "swap" | "identity" => {
                        page_to(window, shell, node, surface, 0, cx).await?;
                        choose_preset(window, shell, node, surface, stage == "swap", cx).await?;
                    }
                    _ => {}
                }
                let scroll =
                    meter_position(window, shell, node, stage == "signed-last-edited", cx).await?;
                let frame = save_frame(
                    window, shell, node, directory, density, surface, stage, scroll, cx,
                )
                .await?;
                manifest["captures"]
                    .as_array_mut()
                    .expect("captures")
                    .push(frame);
                std::fs::write(&path, serde_json::to_vec_pretty(&manifest)?)?;
            }
        }
    }
    anyhow::ensure!(
        manifest["captures"]
            .as_array()
            .is_some_and(|frames| frames.len() == FRAME_COUNT),
        "Mapper editing requires {FRAME_COUNT} native frames"
    );
    dispatch(shell, AppRequest::Stop, cx)?;
    wait_for(
        shell,
        "Mapper capture audio stopped",
        |_, snapshot| snapshot.phase == SessionPhase::Idle,
        cx,
    )
    .await?;
    manifest["complete"] = true.into();
    std::fs::write(path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CHANNELS, FRAME_COUNT, LAST_PAGE, PAGE_WIDTH, fixture, preset_settings, signed_settings,
        verify_meter_bounds, verify_value, weight,
    };
    use crate::service::{GraphEdit, ServiceGraph};
    use gpui_kit::{Bounds, point, size};
    use moiren_core::graph::NodeKind;

    #[::core::prelude::v1::test]
    fn mapper_capture_fixture_is_real_32_channel_routing_with_signed_headroom() {
        let snapshot = fixture().unwrap();
        assert_eq!(snapshot.graph.nodes().len(), 3);
        assert_eq!(snapshot.graph.edges().len(), 2);
        assert!(
            snapshot
                .graph
                .nodes()
                .iter()
                .all(|node| node.channels() == CHANNELS)
        );
        let (_, node) = snapshot.node_of_kind(NodeKind::ChannelMapper).unwrap();
        let settings = node.builtin.channel_mapper().unwrap();
        assert!(settings.valid_layout(CHANNELS, CHANNELS));
        assert_eq!(settings, &signed_settings());
        assert_eq!(weight(settings, 0, 0), -2.5);
        assert_eq!(weight(settings, 1, 1), 3.25);
        assert_eq!(weight(settings, 31, 31), -1.75);
        assert_eq!(weight(settings, 31, 30), 2.25);
        assert_eq!(weight(settings, 5, 12), 0.375);
        assert_eq!(weight(settings, 14, 14), 1.);
    }

    #[::core::prelude::v1::test]
    fn mapper_capture_presets_use_service_and_preserve_all_higher_channels() {
        let snapshot = fixture().unwrap();
        let node = snapshot
            .node_of_kind(NodeKind::ChannelMapper)
            .unwrap()
            .0
            .id();
        let topology = snapshot.graph.clone();
        let mut graph = ServiceGraph::default();
        graph.replace(snapshot.graph, snapshot.nodes).unwrap();
        for swap in [true, false] {
            graph.edit(GraphEdit::MapperPreset { node, swap }).unwrap();
            let snapshot = graph.snapshot();
            assert_eq!(snapshot.graph, topology);
            let settings = snapshot
                .node(node)
                .unwrap()
                .builtin
                .channel_mapper()
                .unwrap();
            assert_eq!(settings, &preset_settings(swap));
            for channel in 2..CHANNELS {
                assert_eq!(weight(settings, channel, channel), 1.);
            }
            graph.preparation().engine_project(256).unwrap();
        }
    }

    #[::core::prelude::v1::test]
    fn mapper_capture_pages_cover_last_channel_without_expanding_the_tile() {
        assert_eq!(LAST_PAGE, 28);
        assert_eq!(LAST_PAGE + PAGE_WIDTH, CHANNELS);
        assert_eq!(FRAME_COUNT, 2 * 2 * 4);
        assert_eq!(
            super::super::capture_filter(Some("mapper-editing")).unwrap(),
            Some("mapper-editing")
        );
        assert_eq!(super::super::single_node_frame_count(Some("mapper")), 4);
    }

    #[::core::prelude::v1::test]
    fn mapper_capture_precision_rejects_clamped_rounded_or_nonfinite_values() {
        for (text, expected) in [("-3.125", -3.125), ("+3.25", 3.25), ("0", 0.)] {
            assert!(verify_value(text, expected).is_ok());
        }
        for text in ["-1", "-3.13", "NaN", "inf", "", "3.125"] {
            assert!(verify_value(text, -3.125).is_err(), "{text}");
        }
    }

    #[::core::prelude::v1::test]
    fn mapper_capture_meter_requires_full_height_and_uncompressed_natural_channels() {
        let body = Bounds {
            origin: point(20., 100.),
            size: size(516., 300.),
        };
        let meter = Bounds {
            origin: point(360., 100.),
            size: size(176., 300.),
        };
        let content = Bounds {
            origin: meter.origin,
            size: size(1240., 300.),
        };
        assert!(verify_meter_bounds(body, meter, meter, content).is_ok());
        let mut short = meter;
        short.size.height -= 20.;
        assert!(verify_meter_bounds(body, short, short, content).is_err());
        let mut compressed = content;
        compressed.size.width = 176.;
        assert!(verify_meter_bounds(body, meter, meter, compressed).is_err());
        let mut wide = meter;
        wide.size.width = 328.;
        assert!(verify_meter_bounds(body, wide, wide, content).is_err());
    }
}
