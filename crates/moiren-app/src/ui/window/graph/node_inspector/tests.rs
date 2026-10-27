use super::has_advanced_parameters;
use moiren_core::graph::NodeKind;

#[::core::prelude::v1::test]
fn only_processors_have_an_advanced_parameter_section() {
    for kind in [NodeKind::Source, NodeKind::Sink, NodeKind::Bus] {
        assert!(!has_advanced_parameters(kind));
    }
    for kind in [
        NodeKind::Gain,
        NodeKind::Pan,
        NodeKind::Compressor,
        NodeKind::Eq,
        NodeKind::Gate,
        NodeKind::Limiter,
        NodeKind::Delay,
        NodeKind::ChannelMapper,
        NodeKind::Recorder,
        NodeKind::Meter,
        NodeKind::SpectrumAnalyzer,
        NodeKind::Rack,
    ] {
        assert!(has_advanced_parameters(kind));
    }
}

#[cfg(feature = "gpui-test")]
mod ui {
    use crate::service::{AppRequest, CompressorParameter, GraphEdit};
    use crate::ui::window::StudioShell;
    use crate::ui::window::test_harness::{
        finish_service, settle_panels, test_shell, wait_for, wait_until,
    };
    use gpui_kit::component::input::InputEvent;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Entity, VisualTestContext, px, size};
    use moiren_core::graph::{NodeId, NodeKind};
    use std::time::Duration;

    fn show_inspector(
        view: &Entity<StudioShell>,
        cx: &mut VisualTestContext,
        id: NodeId,
        width: f32,
        height: f32,
    ) {
        cx.update(|window, cx| {
            window.resize(size(px(width), px(height)));
            // Let the shell apply its responsive defaults before explicitly
            // reopening Inspector in the compact layout.
            window.render_frame(cx);
        });
        view.update(cx, |shell, cx| {
            shell.selected_node = Some(id);
            shell.selected_nodes.clear();
            shell.selected_route = None;
            shell.sidebar_open = false;
            shell.inspector_open = true;
            cx.notify();
        });
        cx.update(|window, cx| window.render_frame(cx));
        settle_panels(cx);
    }

    #[gpui_kit::test]
    fn compact_source_inspector_prioritizes_routing_and_status_over_secondary_controls(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (view, cx, runtime) = test_shell(cx);
        let (id, original_graph) = view.update(cx, |shell, cx| {
            let graph = &shell.state.read(cx).snapshot.graph;
            (
                graph.node_of_kind(NodeKind::Source).unwrap().0.id(),
                graph.clone(),
            )
        });
        show_inspector(&view, cx, id, 1024., 720.);
        let mute_id = format!("strip-mute-{id:?}");
        cx.update(|window, cx| {
            window.render_frame(cx);
            for key in [
                "inspector-binding-toggle",
                "inspector-strip-toggle",
                "inspector-routing-toggle",
                "inspector-route-false-0-0",
                "node-observation-status",
                "node-observation-unknown",
                "remove-node",
            ] {
                assert!(window.find(key).visible(), "missing {key}");
                assert!(
                    window.find(key).bounds().bottom()
                        <= window.find("graph-inspector").bounds().bottom(),
                    "{key} must be above the fold"
                );
            }
            assert!(window.try_find("bind-node-tone").is_none());
            assert!(window.try_find("unbind-node-source").is_none());
            assert!(window.try_find("refresh-node-resources").is_none());
            assert!(window.try_find(mute_id.clone()).is_none());
            window.click("inspector-binding-toggle", cx);
            window.render_frame(cx);
            assert!(window.find("bind-node-tone").visible());
            assert!(window.find("unbind-node-source").visible());
            assert!(window.try_find("refresh-node-resources").is_some());
            window.click("inspector-binding-toggle", cx);
            window.render_frame(cx);
            assert!(window.try_find("bind-node-tone").is_none());
            window.click("inspector-strip-toggle", cx);
            window.render_frame(cx);
            assert!(window.find(mute_id.clone()).visible());
            assert!(window.find(format!("strip-mode-{id:?}")).visible());
            window.click("inspector-strip-toggle", cx);
            window.render_frame(cx);
            assert!(window.try_find(mute_id).is_none());
            window.click("inspector-routing-toggle", cx);
            window.render_frame(cx);
            assert!(window.try_find("inspector-route-false-0-0").is_none());
            window.click("inspector-routing-toggle", cx);
            window.render_frame(cx);
            assert!(window.find("inspector-route-false-0-0").visible());
        });
        view.update(cx, |shell, cx| {
            assert_eq!(shell.selected_node, Some(id));
            assert_eq!(shell.state.read(cx).snapshot.graph, original_graph);
            assert!(!shell.state.read(cx).pending_request);
        });
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn compressor_precision_fields_remain_complete_and_editable_after_disclosure_round_trip(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (view, cx, runtime) = test_shell(cx);
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                    kind: NodeKind::Compressor,
                    channels: 2,
                }));
            });
        });
        let id = wait_for(Duration::from_secs(3), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node_of_kind(NodeKind::Compressor)
                    .map(|(node, _)| node.id())
            })
        });
        show_inspector(&view, cx, id, 1440., 900.);
        cx.update(|window, cx| {
            window.render_frame(cx);
            for parameter in CompressorParameter::ALL {
                assert!(
                    window
                        .find(format!("compressor-input-{}", parameter.id().0))
                        .visible()
                );
            }
            let first = window
                .find(format!(
                    "compressor-input-{}",
                    CompressorParameter::InputGain.id().0
                ))
                .bounds();
            let second = window
                .find(format!(
                    "compressor-input-{}",
                    CompressorParameter::Threshold.id().0
                ))
                .bounds();
            assert_eq!(
                first.top(),
                second.top(),
                "precision inputs should share a row"
            );
            assert!(first.right() < second.left());
            window.click("inspector-parameters-toggle", cx);
            window.render_frame(cx);
            for parameter in CompressorParameter::ALL {
                assert!(
                    window
                        .try_find(format!("compressor-input-{}", parameter.id().0))
                        .is_none()
                );
            }
            window.click("inspector-parameters-toggle", cx);
            window.render_frame(cx);
            assert!(window.find("compressor-input-0").visible());
            view.update(cx, |shell, cx| {
                shell.compressor.inputs[&(id, CompressorParameter::Mix)].update(cx, |input, cx| {
                    input.set_value("0.25", window, cx);
                    cx.emit(InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    });
                });
            });
        });
        wait_until(Duration::from_secs(3), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node(id)
                    .unwrap()
                    .compressor
                    .mix
                    == 0.25
            })
        });
        view.update(cx, |shell, cx| {
            assert!(
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .nodes
                    .iter()
                    .filter(|node| node.id != id)
                    .all(|node| node.compressor == Default::default())
            );
        });
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn inspector_keeps_identity_edits_and_explicit_unknown_observation(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (view, cx, runtime) = test_shell(cx);
        cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
        view.update(cx, |shell, cx| {
            let id = shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node_of_kind(NodeKind::Gain)
                .unwrap()
                .0
                .id();
            shell.selected_node = Some(id);
            shell.inspector_open = true;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.find("inspector-identity").visible());
            assert!(window.find("node-name-display").visible());
            assert!(window.find("node-gain-up").visible());
            assert!(window.find("node-observation-status").visible());
            assert!(window.find("node-observation-unknown").visible());
            assert!(window.try_find("node-observation-levels").is_none());
            assert!(window.try_find("node-observation-candidate").is_none());
        });
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn clearing_node_selection_removes_parameter_controls_and_restores_empty_state(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (view, cx, runtime) = test_shell(cx);
        cx.update(|window, _| window.resize(size(px(1600.), px(1000.))));
        view.update(cx, |shell, cx| {
            shell.selected_node = None;
            shell.selected_nodes.clear();
            shell.selected_route = None;
            shell.inspector_open = true;
            cx.notify();
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.find("inspector-empty").visible());
            assert!(window.find("inspector-empty-art").visible());
            assert!(window.try_find("node-gain-up").is_none());
            assert!(window.try_find("node-name-display").is_none());
        });
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn recorder_inspector_selects_the_same_service_format_as_the_card(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use crate::service::RecordingFormat;
        let (view, cx, runtime) = test_shell(cx);
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                    kind: NodeKind::Recorder,
                    channels: 2,
                }));
            });
        });
        let id = wait_for(Duration::from_secs(3), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node_of_kind(NodeKind::Recorder)
                    .map(|(node, _)| node.id())
            })
        });
        show_inspector(&view, cx, id, 1024., 720.);
        cx.update(|window, cx| {
            window.render_frame(cx);
            for key in ["float32", "pcm24", "pcm16"] {
                assert!(
                    window
                        .find(format!("inspector-record-format-{id:?}-{key}"))
                        .visible()
                );
            }
            window.click(format!("inspector-record-format-{id:?}-pcm24"), cx);
        });
        wait_until(Duration::from_secs(3), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .recordings
                    .iter()
                    .any(|recording| {
                        recording.node == id && recording.format == RecordingFormat::WavPcm24
                    })
            })
        });
        view.read_with(cx, |shell, cx| {
            let recording = shell
                .state
                .read(cx)
                .snapshot
                .recordings
                .iter()
                .find(|recording| recording.node == id)
                .unwrap();
            assert!(
                recording.path.is_none(),
                "format selection must not open or invent a destination"
            );
            assert_eq!(recording.written_frames, 0);
        });
        finish_service(runtime);
    }
}
