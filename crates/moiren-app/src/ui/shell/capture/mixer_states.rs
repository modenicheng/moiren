//! Additional native acceptance states, after timing collection has finished.
use super::*;
use crate::service::{GraphEdit, MixerState, ServiceGraph};
use moiren_core::graph::NodeKind;

pub(super) async fn capture(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &std::path::Path,
    save: &impl Fn(&mut AsyncApp, std::path::PathBuf),
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let viewport = size(px(1440.), px(900.));
    window.update(cx, |_, window, _| window.resize(viewport))?;
    shell.update(cx, |shell, cx| {
        // The states capture wants the roomy layout explicitly: the compact
        // meter run before it collapsed the panels, and growing back above
        // the breakpoint no longer force-restores them (design 21 §2).
        shell.chrome.sidebar_open = true;
        shell.chrome.inspector_open = false;
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
        shell.selection.node = Some(gain);
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
    settle(window, shell, Some(viewport), cx).await?;
    save(cx, directory.join("mixer-muted-controls.png"));

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
    let source = snapshot.node_of_kind(NodeKind::Source).unwrap().0.outputs()[0].id();
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
        shell.selection.node = None;
        shell.state.update(cx, |state, _| {
            state.dispatch(AppRequest::ReplaceGraph {
                graph: snapshot.graph,
                nodes: snapshot.nodes,
            })
        });
        cx.notify();
    });
    settle(window, shell, None, cx).await?;
    save(cx, directory.join("mixer-routing-overflow.png"));
    Ok(())
}
