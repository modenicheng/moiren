//! Additional native acceptance states, after timing collection has finished.
use super::*;
use crate::service::{GraphEdit, MixerState, ServiceGraph};
use moiren_core::graph::NodeKind;

async fn settle(
    shell: &Entity<StudioShell>,
    confirmed: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        cx.background_executor()
            .timer(Duration::from_millis(10))
            .await;
        let ready = shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| state.poll());
            let state = shell.state.read(cx);
            !state.pending_request
                && (!confirmed
                    || state.snapshot.graph.active_revision
                        == Some(state.snapshot.graph.candidate_revision))
        });
        if ready {
            break;
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "capture state did not settle"
        );
    }
    cx.background_executor()
        .timer(Duration::from_millis(100))
        .await;
    Ok(())
}

pub(super) async fn capture(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &std::path::Path,
    save: &impl Fn(&mut AsyncApp, std::path::PathBuf),
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    window.update(cx, |_, window, _| window.resize(size(px(1440.), px(900.))))?;
    cx.background_executor()
        .timer(Duration::from_millis(200))
        .await;
    shell.update(cx, |shell, cx| {
        shell.inspector_open = false;
        let gain = shell
            .state
            .read(cx)
            .snapshot
            .graph
            .nodes
            .iter()
            .find(|entry| {
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .graph
                    .get_node(entry.id)
                    .is_ok_and(|node| node.kind() == NodeKind::Gain)
            })
            .unwrap()
            .id;
        shell.selected_node = Some(gain);
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::EditGraph(GraphEdit::SetMixer {
                node: gain,
                state: MixerState {
                    muted: true,
                    solo: true,
                    dimmed: true,
                    channel_mode: 4,
                    balance: -0.25,
                    gain: 0.5,
                },
            }));
        });
        cx.notify();
    });
    settle(shell, false, cx).await?;
    save(cx, directory.join("mixer-muted-controls.png"));
    shell.update(cx, |shell, cx| {
        shell
            .state
            .update(cx, |state, _| state.dispatch(AppRequest::Stop))
    });
    settle(shell, false, cx).await?;
    save(cx, directory.join("mixer-unknown.png"));

    let mut graph = ServiceGraph::default();
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::Bus,
            channels: 2,
        })
        .map_err(anyhow::Error::msg)?;
    let bus = graph.snapshot().nodes.last().unwrap().id;
    for _ in 0..12 {
        graph
            .edit(GraphEdit::AddBusInput(bus))
            .map_err(anyhow::Error::msg)?;
    }
    graph
        .edit(GraphEdit::SetLabel {
            node: bus,
            label: "Routing bus".into(),
        })
        .map_err(anyhow::Error::msg)?;
    let snapshot = graph.snapshot();
    let source = snapshot
        .graph
        .nodes()
        .iter()
        .find(|node| node.kind() == NodeKind::Source)
        .unwrap()
        .outputs()[0]
        .id();
    for port in snapshot.graph.get_node(bus)?.inputs() {
        graph
            .edit(GraphEdit::Connect {
                source,
                destination: port.id(),
            })
            .map_err(anyhow::Error::msg)?;
    }
    let snapshot = graph.snapshot();
    shell.update(cx, |shell, cx| {
        shell.selected_node = None;
        shell.mixer.dock_width = 320.;
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::ReplaceGraph {
                graph: snapshot.graph,
                nodes: snapshot.nodes,
            })
        });
        cx.notify();
    });
    settle(shell, false, cx).await?;
    save(cx, directory.join("mixer-routing-overflow.png"));
    Ok(())
}
