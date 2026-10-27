//! Native Spectrum acceptance: real sine IO, engine FFT, active-only observations.
use super::{capture_group, dispatch, prepare_group, wait_for};
use crate::service::{
    AppRequest, GraphEdit, GraphSnapshot, InputSelection, RunLimit, ServiceGraph, SessionPhase,
    SessionSpec,
};
use crate::ui::{spectrum::SpectrumStatus, window::StudioShell};
use gpui_kit::{AnyWindowHandle, AsyncApp, Entity, px, size};
use moiren_core::graph::{NodeId, NodeKind};
use moiren_engine::spectrum::SpectrumSettings;
use std::{path::Path, time::Duration};

mod backend;
mod evidence;
mod native;
#[cfg(test)]
mod tests;

pub(super) use backend::ToneBackend;
pub(super) const TONE_HZ: f64 = 937.5;
const KIND: [NodeKind; 1] = [NodeKind::SpectrumAnalyzer];

pub(super) fn fixture() -> anyhow::Result<GraphSnapshot> {
    let snapshot = super::chain_fixture(
        &[NodeKind::Source, NodeKind::SpectrumAnalyzer, NodeKind::Sink],
        2,
    )?;
    let source = snapshot
        .node_of_kind(NodeKind::Source)
        .expect("source")
        .0
        .id();
    let mut graph = ServiceGraph::default();
    graph
        .replace(snapshot.graph, snapshot.nodes)
        .map_err(anyhow::Error::msg)?;
    graph
        .edit(GraphEdit::BindSource {
            node: source,
            source: InputSelection::Tone {
                frequency_hz: TONE_HZ,
            },
        })
        .map_err(anyhow::Error::msg)?;
    graph
        .preparation()
        .engine_project(256)
        .map_err(anyhow::Error::msg)?;
    Ok(graph.snapshot())
}

fn session() -> SessionSpec {
    SessionSpec {
        source: InputSelection::Tone {
            frequency_hz: TONE_HZ,
        },
        output_endpoint_id: super::OUTPUT.into(),
        limit: RunLimit::UntilStopped,
        gain: 0.25,
        pan: 0.,
        max_block_frames: 256,
    }
}

pub(super) async fn capture_live(
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
            .node_of_kind(NodeKind::SpectrumAnalyzer)
            .expect("analyzer")
            .0
            .id()
    });
    let path = directory.join("node-workstations-spectrum-live.json");
    let mut manifest = serde_json::json!({
        "filter": "spectrum-live", "backend": "capture sine source + FakeBackend transport; real engine Hann FFT",
        "tone_hz": TONE_HZ, "channels": 2, "preferences": "isolated defaults; persistence disabled",
        "captures": [], "settings_changes": [], "channel_changes": [], "complete": false,
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest)?)?;
    for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        if shell.update(cx, |shell, cx| {
            shell.state.read(cx).snapshot.phase == SessionPhase::Idle
        }) {
            dispatch(shell, AppRequest::Start(session()), cx)?;
            wait_for(
                shell,
                "restarted spectrum engine",
                |_, snapshot| {
                    snapshot.phase == SessionPhase::Running
                        && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                },
                cx,
            )
            .await?;
        }
        prepare_group(window, shell, &KIND, false, true, cx).await?;
        wait_live(shell, node, current_settings(shell, node, cx)?, cx).await?;
        // The second density starts from the same settings using real controls.
        for (field, value) in [("fft", 1024), ("hop", 512)] {
            if let Some(change) =
                native::change_setting(window, shell, node, "card", field, value, cx).await?
            {
                manifest["settings_changes"]
                    .as_array_mut()
                    .expect("changes")
                    .push(change);
            }
        }
        capture_pair(
            window,
            shell,
            directory,
            density,
            "live",
            node,
            true,
            &mut manifest,
            cx,
        )
        .await?;

        prepare_group(window, shell, &KIND, false, true, cx).await?;
        let fft = native::change_setting(window, shell, node, "card", "fft", 2048, cx).await?;
        anyhow::ensure!(
            fft.is_some(),
            "FFT acceptance must execute a real settings change"
        );
        manifest["settings_changes"]
            .as_array_mut()
            .expect("changes")
            .push(fft.expect("checked"));
        prepare_group(window, shell, &KIND, true, true, cx).await?;
        let hop = native::change_setting(window, shell, node, "inspector", "hop", 1024, cx).await?;
        anyhow::ensure!(
            hop.is_some(),
            "hop acceptance must execute a real settings change"
        );
        manifest["settings_changes"]
            .as_array_mut()
            .expect("changes")
            .push(hop.expect("checked"));
        capture_pair(
            window,
            shell,
            directory,
            density,
            "applied",
            node,
            true,
            &mut manifest,
            cx,
        )
        .await?;

        prepare_group(window, shell, &KIND, true, true, cx).await?;
        manifest["channel_changes"]
            .as_array_mut()
            .expect("channels")
            .push(native::select_channel(
                window,
                shell,
                node,
                "inspector",
                1,
                cx,
            )?);
        capture_pair(
            window,
            shell,
            directory,
            density,
            "channel-2",
            node,
            true,
            &mut manifest,
            cx,
        )
        .await?;
        prepare_group(window, shell, &KIND, false, true, cx).await?;
        manifest["channel_changes"]
            .as_array_mut()
            .expect("channels")
            .push(native::select_channel(window, shell, node, "card", 0, cx)?);

        let graph = shell.update(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
        dispatch(shell, AppRequest::Stop, cx)?;
        wait_for(
            shell,
            "stopped spectrum releases trace and active binding",
            |shell, snapshot| {
                snapshot.phase == SessionPhase::Idle
                    && !snapshot.signal_topics.node_spectra.contains_key(&node)
                    && shell.node_spectra.observation(node).is_none()
                    && shell.node_spectra.status(node, snapshot) == SpectrumStatus::Stopped
            },
            cx,
        )
        .await?;
        shell.update(cx, |shell, cx| {
            native::verify_graph_content(&graph, &shell.state.read(cx).snapshot.graph, node, None)
        })?;
        capture_pair(
            window,
            shell,
            directory,
            density,
            "stopped",
            node,
            false,
            &mut manifest,
            cx,
        )
        .await?;
        std::fs::write(&path, serde_json::to_vec_pretty(&manifest)?)?;
    }
    anyhow::ensure!(
        manifest["captures"]
            .as_array()
            .is_some_and(|frames| frames.len() == 16),
        "Spectrum acceptance requires 16 native frames"
    );
    manifest["complete"] = true.into();
    std::fs::write(path, serde_json::to_vec_pretty(&manifest)?)?;
    eprintln!(
        "Spectrum live: 16 native frames; active FFT/hop, stereo selection and stopped traces verified"
    );
    Ok(())
}

fn current_settings(
    shell: &Entity<StudioShell>,
    node: NodeId,
    cx: &mut AsyncApp,
) -> anyhow::Result<SpectrumSettings> {
    shell.update(cx, |shell, cx| {
        shell
            .state
            .read(cx)
            .snapshot
            .graph
            .node(node)
            .and_then(|row| row.builtin.spectrum())
            .copied()
            .ok_or_else(|| anyhow::anyhow!("capture analyzer settings missing"))
    })
}

async fn wait_live(
    shell: &Entity<StudioShell>,
    node: NodeId,
    settings: SpectrumSettings,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    wait_for(
        shell,
        "fresh real tone FFT matching active settings",
        |shell, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.graph.active_revision == Some(snapshot.graph.candidate_revision)
                && shell.node_spectra.status(node, snapshot) == SpectrumStatus::Live
                && snapshot
                    .signal_topics
                    .node_spectra
                    .get(&node)
                    .is_some_and(|binding| {
                        binding.settings == settings
                            && shell
                                .node_spectra
                                .observation(node)
                                .is_some_and(|observation| {
                                    evidence::verify_spectrum(
                                        observation.window,
                                        &observation.magnitudes,
                                        binding.settings,
                                        binding.channels,
                                        TONE_HZ,
                                    )
                                    .is_ok()
                                })
                    })
        },
        cx,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn capture_pair(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    directory: &Path,
    density: &str,
    state: &str,
    node: NodeId,
    running: bool,
    manifest: &mut serde_json::Value,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    for surface in ["card", "inspector"] {
        let inspector = surface == "inspector";
        let name = format!("nodes-{density}-{surface}-spectrum-{state}");
        let graph = shell.update(cx, |shell, cx| shell.state.read(cx).snapshot.graph.clone());
        let mut frame = capture_group(
            window, shell, directory, &name, &KIND, inspector, running, cx,
        )
        .await?;
        if running {
            wait_live(shell, node, current_settings(shell, node, cx)?, cx).await?;
        }
        frame["spectrum"] = native::frame_evidence(
            window,
            shell,
            node,
            surface,
            running,
            &directory.join(format!("{name}.png")),
            cx,
        )?;
        let expected_channel = if state == "channel-2" { 2 } else { 1 };
        anyhow::ensure!(
            frame["spectrum"]["selected_channel"].as_u64() == Some(expected_channel),
            "Spectrum selection changed while switching capture surfaces"
        );
        shell.update(cx, |shell, cx| -> anyhow::Result<()> {
            anyhow::ensure!(
                shell.state.read(cx).snapshot.graph == graph,
                "Spectrum capture changed the graph"
            );
            Ok(())
        })?;
        manifest["captures"]
            .as_array_mut()
            .expect("captures")
            .push(frame);
        std::fs::write(
            directory.join("node-workstations-spectrum-live.json"),
            serde_json::to_vec_pretty(manifest)?,
        )?;
    }
    Ok(())
}
