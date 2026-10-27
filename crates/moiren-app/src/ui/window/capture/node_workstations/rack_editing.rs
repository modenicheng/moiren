//! Native Rack acceptance uses service-owned settings and presentation-only selection.
use super::{
    capture_group, contains_native_bounds, dispatch, native_measurements, pixel_bounds,
    prepare_group, wait_for,
};
use crate::service::{AppRequest, BuiltinState, GraphEdit, GraphSnapshot, SessionPhase};
use crate::ui::window::StudioShell;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Bounds, Entity, InputEvent, Role, ScrollDelta, Window, point,
    px, size,
};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::rack::{RackProcessor, RackSettings, RackSlot};
use std::{path::Path, time::Duration};

const STAGES: [(u32, &str); 8] = [
    (1, "gain"),
    (2, "pan"),
    (3, "eq"),
    (4, "compressor"),
    (5, "gate"),
    (6, "limiter"),
    (7, "delay"),
    (8, "mapper"),
];

fn editing_settings() -> RackSettings {
    RackSettings {
        slots: [
            RackProcessor::Gain { initial: 0.75 },
            RackProcessor::Pan {
                initial: 0.,
                mono: false,
                swap: false,
            },
            RackProcessor::Eq(Default::default()),
            RackProcessor::Compressor(Default::default()),
            RackProcessor::Gate(Default::default()),
            RackProcessor::Limiter(Default::default()),
            RackProcessor::Delay(Default::default()),
            RackProcessor::ChannelMapper(Default::default()),
        ]
        .into_iter()
        .zip(STAGES)
        .map(|(processor, (id, _))| RackSlot {
            id,
            bypass: false,
            processor,
        })
        .collect(),
        routes: Vec::new(),
    }
}

pub(super) async fn capture_rack(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let (node, revision) = shell.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        (
            graph
                .node_of_kind(NodeKind::Rack)
                .expect("capture rack")
                .0
                .id(),
            graph.candidate_revision,
        )
    });
    let settings = editing_settings();
    dispatch(
        shell,
        AppRequest::EditGraph(GraphEdit::SetBuiltin {
            node,
            state: BuiltinState::Rack(settings.clone()),
        }),
        cx,
    )?;
    wait_for(
        shell,
        "all eight Rack stages confirmed active",
        |_, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.graph.candidate_revision > revision
                && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                && snapshot
                    .graph
                    .node(node)
                    .and_then(|node| node.builtin.rack())
                    == Some(&settings)
        },
        cx,
    )
    .await?;
    let confirmed = shell.update(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
    let path = directory.join("node-workstations-rack-editing.json");
    let mut manifest = serde_json::json!({
        "filter": "rack-editing", "backend": "FakeBackend (real service and engine)",
        "preferences": "isolated defaults; persistence disabled", "settings": settings,
        "selection": "native selectors; shared Card/Inspector presentation state",
        "input_editing_verified": false, "captures": [], "complete": false,
    });
    for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        for (surface, stages) in [
            ("Card", STAGES.to_vec()),
            ("Inspector", vec![STAGES[3], STAGES[7]]),
        ] {
            let inspector = surface == "Inspector";
            prepare_group(window, shell, &[NodeKind::Rack], inspector, true, cx).await?;
            if inspector {
                // Opening the second surface must preserve the Card selection.
                window.update(cx, |_, window, _| verify_shared_selection(window, node))??;
            }
            for (slot, slug) in stages {
                select_stage(window, shell, node, (surface, slot), &confirmed, cx)?;
                let name = format!(
                    "nodes-{density}-{}-rack-editing-{slug}",
                    surface.to_lowercase()
                );
                // The editor has an intrinsic height: settle/recenter after
                // every selection, never reuse the preceding stage's bounds.
                let mut frame = capture_group(
                    window,
                    shell,
                    directory,
                    &name,
                    &[NodeKind::Rack],
                    inspector,
                    true,
                    cx,
                )
                .await?;
                frame["rack"] = window.update(cx, |_, window, cx| -> anyhow::Result<_> {
                    if inspector {
                        let camera = shell.read(cx).camera;
                        let measured = native_measurements(window, &[node])?;
                        reveal(
                            window,
                            "graph-inspector",
                            &format!("rack-Inspector-{node:?}-editor-{slot}"),
                            cx,
                        )?;
                        window.dispatch_event(
                            gpui_kit::MouseMoveEvent {
                                position: point(px(2.), px(2.)),
                                pressed_button: None,
                                modifiers: Default::default(),
                            }
                            .to_platform_input(),
                            cx,
                        );
                        window.render_frame(cx);
                        anyhow::ensure!(
                            shell.read(cx).camera == camera
                                && native_measurements(window, &[node])? == measured,
                            "Inspector scroll changed the measured canvas"
                        );
                    }
                    verify_confirmed(shell.read(cx), node, &confirmed, cx)?;
                    let evidence = editor_evidence(window, node, slot, inspector)?;
                    if inspector {
                        // Preserve capture_group's verified canvas and replace
                        // only the frame with its revealed Inspector editor.
                        window
                            .render_to_image()?
                            .save(directory.join(format!("{name}.png")))?;
                    }
                    Ok(evidence)
                })??;
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
            .is_some_and(|frames| frames.len() == 20),
        "Rack editing requires 20 native frames"
    );
    dispatch(shell, AppRequest::Stop, cx)?;
    wait_for(
        shell,
        "Rack capture audio stopped",
        |_, snapshot| snapshot.phase == SessionPhase::Idle,
        cx,
    )
    .await?;
    manifest["complete"] = true.into();
    std::fs::write(path, serde_json::to_vec_pretty(&manifest)?)?;
    eprintln!("Rack editing: 16 selected-stage cards and four shared-selection Inspectors");
    Ok(())
}

fn verify_confirmed(
    shell: &StudioShell,
    node: NodeId,
    confirmed: &GraphSnapshot,
    cx: &App,
) -> anyhow::Result<()> {
    let state = shell.state.read(cx);
    anyhow::ensure!(
        !state.pending_request && state.dispatch_error.is_none(),
        "stage selection dispatched a request"
    );
    anyhow::ensure!(
        state.snapshot.phase == SessionPhase::Running
            && state.snapshot.error.is_none()
            && state.snapshot.graph.as_ref() == confirmed
            && state.snapshot.graph.active_revision
                == Some(state.snapshot.graph.candidate_revision)
            && state
                .snapshot
                .graph
                .node(node)
                .and_then(|node| node.builtin.rack())
                == Some(&editing_settings()),
        "stage selection changed the confirmed Rack graph"
    );
    Ok(())
}

fn select_stage(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    target: (&str, u32),
    confirmed: &GraphSnapshot,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let (surface, slot) = target;
    window.update(cx, |_, window, cx| -> anyhow::Result<()> {
        window.render_frame(cx);
        let camera = shell.read(cx).camera;
        let positions = shell.read(cx).node_positions.clone();
        verify_confirmed(shell.read(cx), node, confirmed, cx)?;
        let list = format!("rack-{surface}-{node:?}-stages");
        let selector = format!("rack-{surface}-{node:?}-select-{slot}");
        if surface == "Inspector" {
            reveal(window, "graph-inspector", &list, cx)?;
        }
        reveal(window, &list, &selector, cx)?;
        window.click(selector, cx);
        window.render_frame(cx);
        let shell = shell.read(cx);
        anyhow::ensure!(
            shell.camera == camera
                && shell.node_positions == positions
                && shell.node_drag.is_none(),
            "Rack selector dragged a card or changed the camera"
        );
        verify_confirmed(shell, node, confirmed, cx)?;
        selected_editor(window, node, surface, slot)?;
        if surface == "Inspector" {
            verify_shared_selection(window, node)?;
        }
        Ok(())
    })??;
    Ok(())
}

fn scroll_delta(viewport: Bounds<f32>, target: Bounds<f32>) -> anyhow::Result<f32> {
    anyhow::ensure!(
        target.size.height > 0.
            && target.size.height <= viewport.size.height
            && target.size.width > 0.
            && target.left() >= viewport.left()
            && target.right() <= viewport.right(),
        "Rack control cannot fit inside its native scroll viewport: target={target:?}, viewport={viewport:?}"
    );
    Ok(if target.top() < viewport.top() {
        viewport.top() - target.top() + 2.
    } else if target.bottom() > viewport.bottom() {
        viewport.bottom() - target.bottom() - 2.
    } else {
        0.
    })
}

fn reveal(window: &mut Window, viewport: &str, target: &str, cx: &mut App) -> anyhow::Result<()> {
    for _ in 0..8 {
        window.render_frame(cx);
        let outer = window
            .try_find(viewport.to_owned())
            .ok_or_else(|| anyhow::anyhow!("missing Rack scroll viewport {viewport}"))?;
        let inner = window
            .try_find(target.to_owned())
            .ok_or_else(|| anyhow::anyhow!("missing Rack control {target}"))?;
        let delta = scroll_delta(pixel_bounds(outer.bounds()), pixel_bounds(inner.bounds()))?;
        if delta == 0. {
            anyhow::ensure!(
                outer.visible() && inner.visible(),
                "Rack target is not visible: {target}"
            );
            return Ok(());
        }
        let delta = ScrollDelta::Pixels(point(px(0.), px(delta)));
        if viewport == "graph-inspector" {
            // Aim at panel padding, not its center: a nested stage list at the
            // center would consume the wheel and leave the editor clipped.
            window.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position: point(outer.bounds().right() - px(2.), outer.bounds().center().y),
                    delta,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        } else {
            window.scroll(viewport.to_owned(), delta, cx);
        }
    }
    anyhow::bail!("Rack target remained clipped after native scrolling: {target}")
}

fn selected_editor(
    window: &Window,
    node: NodeId,
    surface: &str,
    slot: u32,
) -> anyhow::Result<gpui_kit::test::ElementSnapshot> {
    let editor = window
        .try_find(format!("rack-{surface}-{node:?}-editor-{slot}"))
        .ok_or_else(|| {
            anyhow::anyhow!("missing selected {surface} Rack editor for stage {slot}")
        })?;
    for (other, _) in STAGES {
        anyhow::ensure!(
            other == slot
                || window
                    .try_find(format!("rack-{surface}-{node:?}-editor-{other}"))
                    .is_none(),
            "Rack renders more than the selected stage"
        );
    }
    Ok(editor)
}

fn verify_shared_selection(window: &Window, node: NodeId) -> anyhow::Result<()> {
    let selected = STAGES
        .into_iter()
        .filter(|(slot, _)| {
            window
                .try_find(format!("rack-Card-{node:?}-editor-{slot}"))
                .is_some()
        })
        .map(|(slot, _)| slot)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        selected.len() == 1,
        "Card requires exactly one selected Rack stage"
    );
    selected_editor(window, node, "Inspector", selected[0])?;
    Ok(())
}

fn precision_input_count(slot: u32, inspector: bool) -> Option<usize> {
    Some(match slot {
        1 | 2 => 1,
        3 | 6 | 7 => 3,
        4 if inspector => 10,
        4 => 6,
        5 => 5,
        8 => 4, // The actual fixture is a two-input, two-output matrix.
        _ => return None,
    })
}

fn verify_overview_clear(
    overview: Option<Bounds<f32>>,
    target: Bounds<f32>,
    label: &str,
) -> anyhow::Result<()> {
    if let Some(overview) = overview {
        anyhow::ensure!(
            !(overview.left() < target.right()
                && overview.right() > target.left()
                && overview.top() < target.bottom()
                && overview.bottom() > target.top()),
            "graph-overview overlaps Rack {label}: overview={overview:?}, target={target:?}"
        );
    }
    Ok(())
}

fn editor_evidence(
    window: &Window,
    node: NodeId,
    slot: u32,
    inspector: bool,
) -> anyhow::Result<serde_json::Value> {
    let canvas = pixel_bounds(window.find("graph-canvas-input").bounds());
    let mut body = native_measurements(window, &[node])?.bodies[&node];
    body.origin.x += canvas.origin.x;
    body.origin.y += canvas.origin.y;
    // All comparisons use observed window coordinates, not canvas-local bounds.
    let overview = window
        .try_find("graph-overview")
        .filter(|overlay| overlay.visible())
        .map(|overlay| pixel_bounds(overlay.bounds()));
    verify_overview_clear(overview, body, "Card body")?;
    if let Some(meter) = window.try_find(format!("card-meter-{node:?}")) {
        verify_overview_clear(overview, pixel_bounds(meter.bounds()), "Card meter")?;
    }
    let surfaces = if inspector {
        vec!["Card", "Inspector"]
    } else {
        vec!["Card"]
    };
    let mut editors = Vec::new();
    for surface in surfaces {
        let editor = selected_editor(window, node, surface, slot)?;
        let bounds = pixel_bounds(editor.bounds());
        verify_overview_clear(
            overview,
            bounds,
            &format!("{surface} editor (stage {slot})"),
        )?;
        let viewport = if surface == "Card" {
            body
        } else {
            pixel_bounds(window.find("graph-inspector").bounds())
        };
        anyhow::ensure!(
            editor.visible() && contains_native_bounds(viewport, bounds),
            "{surface} Rack stage {slot} selected editor is clipped: visible={}, editor={bounds:?}, viewport={viewport:?}",
            editor.visible()
        );
        // Inspect the controls actually painted below this editor, not guessed
        // parameter IDs or accessibility disabled facts absent on base::Button.
        let controls = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|control| {
                control.path().len() > editor.path().len()
                    && control.path().starts_with(editor.path())
                    && control.role().is_some()
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !controls.is_empty(),
            "{surface} Rack editor has no observed controls"
        );
        for control in &controls {
            let control_bounds = pixel_bounds(control.bounds());
            anyhow::ensure!(
                control.visible() && contains_native_bounds(bounds, control_bounds),
                "{surface} Rack stage {slot} control overflows selected editor: path={:?}, role={:?}, visible={}, control={control_bounds:?}, editor={bounds:?}, viewport={viewport:?}, overflow_ltrb={:?}",
                control.path(),
                control.role(),
                control.visible(),
                [
                    (bounds.left() - control_bounds.left()).max(0.),
                    (bounds.top() - control_bounds.top()).max(0.),
                    (control_bounds.right() - bounds.right()).max(0.),
                    (control_bounds.bottom() - bounds.bottom()).max(0.)
                ]
            );
        }
        let mut inputs = controls
            .iter()
            .filter(|control| control.role() == Some(Role::TextInput))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            Some(inputs.len()) == precision_input_count(slot, surface == "Inspector"),
            "{surface} Rack stage {slot} has {} precision fields; expected {:?}",
            inputs.len(),
            precision_input_count(slot, surface == "Inspector")
        );
        inputs.sort_by(|left, right| {
            let left = pixel_bounds(left.bounds());
            let right = pixel_bounds(right.bounds());
            left.top()
                .total_cmp(&right.top())
                .then_with(|| left.left().total_cmp(&right.left()))
        });
        let values = inputs
            .iter()
            .map(|input| {
                let value = input
                    .value()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("{surface} Rack stage {slot} has an empty precision value")
                    })?;
                Ok(value.to_owned())
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        editors.push(serde_json::json!({"surface": surface, "bounds": [bounds.left(), bounds.top(), bounds.size.width, bounds.size.height],
            "observed_controls": controls.len(), "control_bounds_verified": true,
            "precision_input_count": inputs.len(), "precision_values_top_to_bottom": values}));
    }
    if inspector {
        verify_shared_selection(window, node)?;
    }
    Ok(
        serde_json::json!({"stage": STAGES.iter().find(|(id, _)| *id == slot).expect("stage").1,
        "editors": editors, "shared_selection_verified": inspector, "active_candidate_confirmed": true,
        "overview_visible": overview.is_some(),
        "overview_bounds": overview.map(|bounds| [bounds.left(), bounds.top(), bounds.size.width, bounds.size.height]),
        "overview_overlap_verified": true,
        "selection_did_not_dispatch_or_drag": true}),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        STAGES, editing_settings, precision_input_count, scroll_delta, verify_overview_clear,
    };
    use crate::service::{BuiltinState, GraphEdit, ServiceGraph};
    use gpui_kit::{Bounds, point, size};
    use moiren_core::graph::NodeKind;
    use moiren_engine::rack::RackProcessor;

    #[::core::prelude::v1::test]
    fn overview_must_not_cover_editor_or_right_meter_but_may_touch_an_edge() {
        let target = Bounds {
            origin: point(100., 100.),
            size: size(40., 200.),
        };
        let touching = Bounds {
            origin: point(140., 220.),
            size: size(180., 100.),
        };
        assert!(verify_overview_clear(None, target, "meter").is_ok());
        assert!(verify_overview_clear(Some(touching), target, "meter").is_ok());
        let overlap = Bounds {
            origin: point(139., 220.),
            ..touching
        };
        let error = verify_overview_clear(Some(overlap), target, "Card meter").unwrap_err();
        assert!(error.to_string().contains("Card meter"));
        assert!(verify_overview_clear(Some(target), target, "Inspector editor").is_err());
        let below = Bounds {
            origin: point(120., 300.),
            ..touching
        };
        assert!(verify_overview_clear(Some(below), target, "editor").is_ok());
    }

    #[::core::prelude::v1::test]
    fn precision_input_counts_cover_every_stage_and_inspector_compressor_extras() {
        assert_eq!(
            STAGES.map(|(slot, _)| precision_input_count(slot, false)),
            [
                Some(1),
                Some(1),
                Some(3),
                Some(6),
                Some(5),
                Some(3),
                Some(3),
                Some(4)
            ]
        );
        assert_eq!(precision_input_count(4, true), Some(10));
        assert_eq!(precision_input_count(8, true), Some(4));
        assert_eq!(precision_input_count(0, false), None);
        assert_eq!(precision_input_count(9, true), None);
    }

    #[::core::prelude::v1::test]
    fn eight_leaf_fixture_has_stable_ids_and_one_default_stereo_route() {
        let settings = editing_settings();
        assert_eq!(settings.slots.len(), 8);
        assert!(settings.routes.is_empty());
        assert!(settings.slots.iter().all(|slot| !slot.bypass));
        assert_eq!(
            settings
                .slots
                .iter()
                .map(|slot| slot.id)
                .collect::<Vec<_>>(),
            STAGES.map(|(id, _)| id)
        );
        assert!(matches!(
            settings.slots[0].processor,
            RackProcessor::Gain { .. }
        ));
        assert!(matches!(
            settings.slots[1].processor,
            RackProcessor::Pan { .. }
        ));
        assert!(matches!(settings.slots[2].processor, RackProcessor::Eq(_)));
        assert!(matches!(
            settings.slots[3].processor,
            RackProcessor::Compressor(_)
        ));
        assert!(matches!(
            settings.slots[4].processor,
            RackProcessor::Gate(_)
        ));
        assert!(matches!(
            settings.slots[5].processor,
            RackProcessor::Limiter(_)
        ));
        assert!(matches!(
            settings.slots[6].processor,
            RackProcessor::Delay(_)
        ));
        assert!(matches!(
            settings.slots[7].processor,
            RackProcessor::ChannelMapper(_)
        ));
        let layout = settings.resolve_layout(&[2], &[2]).unwrap();
        assert_eq!(layout.routes.len(), 1);
        assert_eq!(layout.routes[0].slots, (1..=8).collect::<Vec<_>>());
        assert!(
            layout
                .slots
                .values()
                .all(|slot| slot.input_channels == 2 && slot.output_channels == 2)
        );
    }

    #[::core::prelude::v1::test]
    fn real_rack_fixture_edit_preserves_connections_and_prepares_all_eight_stages() {
        let initial = super::super::fixture().unwrap();
        let mut graph = ServiceGraph::default();
        graph
            .replace(initial.graph.clone(), initial.nodes.clone())
            .unwrap();
        let node = graph
            .snapshot()
            .node_of_kind(NodeKind::Rack)
            .unwrap()
            .0
            .id();
        graph
            .edit(GraphEdit::SetBuiltin {
                node,
                state: BuiltinState::Rack(editing_settings()),
            })
            .unwrap();
        let edited = graph.snapshot();
        assert_eq!(edited.graph, initial.graph);
        assert_eq!(
            edited.node(node).unwrap().builtin.rack(),
            Some(&editing_settings())
        );
        graph.preparation().engine_project(256).unwrap();
    }

    #[::core::prelude::v1::test]
    fn scroll_reveals_last_selector_instead_of_clicking_a_clipped_center() {
        let viewport = Bounds {
            origin: point(20., 40.),
            size: size(320., 96.),
        };
        let row = Bounds {
            origin: point(20., 124.),
            size: size(150., 28.),
        };
        assert!(scroll_delta(viewport, row).unwrap() < 0.);
        let visible = Bounds {
            origin: point(20., 108.),
            ..row
        };
        assert_eq!(scroll_delta(viewport, visible).unwrap(), 0.);
        let above = Bounds {
            origin: point(20., 28.),
            ..row
        };
        assert!(scroll_delta(viewport, above).unwrap() > 0.);
        let too_tall = Bounds {
            size: size(150., 100.),
            ..row
        };
        assert!(scroll_delta(viewport, too_tall).is_err());
    }
}
