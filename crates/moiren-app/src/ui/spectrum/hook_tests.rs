use super::{Camera, Page, SessionPhase};
use crate::service::{AppRequest, GraphEdit};
use crate::ui::spectrum::SpectrumStatus;
use crate::ui::{finish_service, settle_panels, test_shell, wait_until};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, px, size};
use moiren_core::graph::NodeKind;
use std::time::Duration;

#[gpui_kit::test]
fn spectrum_stopped_card_hook_preserves_visible_demand_without_transport(cx: &mut TestAppContext) {
    let (view, cx, runtime) = test_shell(cx);
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::AddNode {
                kind: NodeKind::SpectrumAnalyzer,
                channels: 2,
            }));
        });
    });
    wait_until(Duration::from_secs(2), || {
        view.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            shell
                .state
                .read(cx)
                .snapshot
                .graph
                .node_of_kind(NodeKind::SpectrumAnalyzer)
                .is_some()
        })
    });
    cx.simulate_resize(size(px(1440.), px(900.)));
    let id = view.update(cx, |shell, cx| {
        let graph = &shell.state.read(cx).snapshot.graph;
        let id = graph
            .node_of_kind(NodeKind::SpectrumAnalyzer)
            .unwrap()
            .0
            .id();
        for node in &graph.nodes {
            shell.node_positions.insert(node.id, (4000., 4000.));
        }
        shell.node_positions.insert(id, (20., 20.));
        shell.presentation.show();
        shell.presentation.page = Page::Graph;
        shell.settings_open = false;
        shell.sidebar_open = false;
        shell.inspector_open = false;
        shell.selected_node = Some(id);
        shell.camera = Camera {
            pan_x: 20.,
            pan_y: 65.,
            zoom: 1.,
            adjusted: true,
        };
        cx.notify();
        id
    });
    settle_panels(cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.find(format!("spectrum-plot-card-{id:?}")).visible());
    });
    view.update(cx, |shell, cx| {
        assert_eq!(shell.state.read(cx).snapshot.phase, SessionPhase::Idle);
        assert!(shell.node_meter_points.borrow().contains(&id));
        assert!(!shell.meters_frame_active(cx));
        shell.pump_meters(cx);
        assert_eq!(
            shell
                .node_spectra
                .status(id, &shell.state.read(cx).snapshot),
            SpectrumStatus::Stopped,
            "a visible card must not become Hidden just because audio stopped"
        );
        assert!(shell.node_spectra.observation(id).is_none());

        shell.presentation.hide_to_tray();
        shell.pump_meters(cx);
        assert_eq!(
            shell
                .node_spectra
                .status(id, &shell.state.read(cx).snapshot),
            SpectrumStatus::Hidden
        );

        shell.presentation.show();
        shell.set_page(Page::Mixer, cx);
        // Even a registered Mixer observation point cannot own an FFT plot.
        shell.node_meter_points.borrow_mut().insert(id);
        shell.pump_meters(cx);
        assert_eq!(
            shell
                .node_spectra
                .status(id, &shell.state.read(cx).snapshot),
            SpectrumStatus::Hidden
        );
        shell.inspector_open = true;
        shell.pump_meters(cx);
        assert_eq!(
            shell
                .node_spectra
                .status(id, &shell.state.read(cx).snapshot),
            SpectrumStatus::Stopped
        );
        shell.settings_open = true;
        shell.pump_meters(cx);
        assert_eq!(
            shell
                .node_spectra
                .status(id, &shell.state.read(cx).snapshot),
            SpectrumStatus::Hidden
        );
    });
    finish_service(runtime);
}
