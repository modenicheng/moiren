//! Opt-in format acceptance: native controls, live service and real short WAVs.
use super::{
    capture_group, contains_native_bounds, dispatch, native_measurements, pixel_bounds,
    prepare_group, wait_for, wav_validation,
};
use crate::service::{
    AppRequest, RecordingFormat, RecordingPhase, RecordingSnapshot, SessionPhase,
};
use crate::ui::window::StudioShell;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, App, AsyncApp, Entity, Window, px, size};
use moiren_core::graph::{NodeId, NodeKind};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FORMATS: [(RecordingFormat, &str, wav_validation::Encoding); 3] = [
    (
        RecordingFormat::WavFloat32,
        "float32",
        wav_validation::Encoding::Float32,
    ),
    (
        RecordingFormat::WavPcm24,
        "pcm24",
        wav_validation::Encoding::Pcm24,
    ),
    (
        RecordingFormat::WavPcm16,
        "pcm16",
        wav_validation::Encoding::Pcm16,
    ),
];

pub(super) async fn capture_formats(
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
            .node_of_kind(NodeKind::Recorder)
            .expect("capture recorder")
            .0
            .id()
    });
    let directory = std::path::absolute(directory)?;
    let manifest_path = directory.join("node-workstations-recording-formats.json");
    let mut manifest = serde_json::json!({
        "filter": "recording-formats",
        "backend": "FakeBackend (real engine audio and managed recording worker)",
        "preferences": "isolated defaults; persistence disabled",
        "format_selection": "native format buttons; service-confirmed before recording",
        "captures": [], "recordings": [], "complete": false,
    });
    for (density, width, height) in [("comfortable", 1440., 900.), ("compact", 1024., 720.)] {
        window.update(cx, |_, window, _| {
            window.resize(size(px(width), px(height)))
        })?;
        cx.background_executor()
            .timer(Duration::from_millis(250))
            .await;
        for (index, (format, slug, encoding)) in FORMATS.into_iter().enumerate() {
            let path = new_destination(&directory, density, slug)?;
            dispatch(
                shell,
                AppRequest::SetRecordingDestination {
                    node,
                    path: path.clone(),
                },
                cx,
            )?;
            wait_for(
                shell,
                "idle format capture destination",
                |_, snapshot| {
                    snapshot.phase == SessionPhase::Running
                        && snapshot.recordings.iter().any(|r| {
                            r.node == node
                                && r.phase == RecordingPhase::Idle
                                && r.path.as_ref() == Some(&path)
                                && !r.estimate.preflight_pending
                        })
                },
                cx,
            )
            .await?;
            prepare_group(window, shell, &[NodeKind::Recorder], false, true, cx).await?;
            // Always prove a transition, including Float32 when it is the
            // default. Both selections go through UI, not SetRecordingFormat.
            for selected in [
                FORMATS[(index + 1) % FORMATS.len()],
                (format, slug, encoding),
            ] {
                click_format(window, shell, node, selected.0, selected.1, cx).await?;
            }
            let name = format!("nodes-{density}-card-recording-{slug}-armed");
            let frame = capture_group(
                window,
                shell,
                &directory,
                &name,
                &[NodeKind::Recorder],
                false,
                true,
                cx,
            )
            .await?;
            verify_frame(&frame, format, RecordingPhase::Idle)?;
            window.update(cx, |_, window, _| verify_format_controls(window, node))??;
            manifest["captures"]
                .as_array_mut()
                .expect("capture array")
                .push(frame);
            anyhow::ensure!(
                !path.try_exists()?,
                "arming created a WAV before Start recording"
            );
            click_transport(window, node, cx)?;
            let recording_check = async {
                wait_for(
                    shell,
                    "short real recording in selected format",
                    |_, snapshot| {
                        snapshot.phase == SessionPhase::Running
                            && snapshot.recordings.iter().any(|r| {
                                r.node == node
                                    && r.phase == RecordingPhase::Recording
                                    && r.format == format
                                    && r.sample_rate.is_some_and(|rate| {
                                        r.written_frames >= u64::from(rate) / 10
                                    })
                            })
                    },
                    cx,
                )
                .await?;
                verify_busy_format_clicks(window, shell, node, format, cx).await?;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            // Finalize even if busy-control verification fails. Do not strand
            // an audio file worker while unwinding a native acceptance error.
            if let Err(error) = recording_check {
                dispatch(shell, AppRequest::StopRecording { node }, cx)?;
                wait_finished(shell, node, format, cx).await?;
                return Err(error);
            }
            let stopped = click_transport(window, node, cx);
            if stopped.is_err() {
                dispatch(shell, AppRequest::StopRecording { node }, cx)?;
            }
            wait_finished(shell, node, format, cx).await?;
            stopped?;
            let saved = shell.update(cx, |shell, cx| {
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .recordings
                    .iter()
                    .find(|r| r.node == node)
                    .expect("saved recorder")
                    .clone()
            });
            let proof = verify_saved(&path, &saved, encoding)?;
            let name = format!("nodes-{density}-card-recording-{slug}-saved");
            let frame = capture_group(
                window,
                shell,
                &directory,
                &name,
                &[NodeKind::Recorder],
                false,
                true,
                cx,
            )
            .await?;
            verify_frame(&frame, format, RecordingPhase::Finished)?;
            window.update(cx, |_, window, _| verify_format_controls(window, node))??;
            manifest["captures"]
                .as_array_mut()
                .expect("capture array")
                .push(frame);
            let takes = manifest["recordings"]
                .as_array_mut()
                .expect("recording array");
            let take_index = takes.len();
            takes.push(serde_json::json!({
                "density": density, "format": format.label(), "format_slug": slug,
                "path": path, "wav": proof, "wav_header_verified": true,
                "sample_payload_verified": true, "busy_format_buttons_disabled": true,
                "busy_format_verification": "native clicks on all three controls; no pending request or service format/lifecycle change",
                "native_format_transition_verified": true, "capture_wav_removed": false,
            }));
            // Save evidence first; only this run's unique, verified artifact is
            // deleted. Failed verification leaves the WAV for diagnosis.
            write_manifest(&manifest_path, &manifest)?;
            std::fs::remove_file(&path)?;
            anyhow::ensure!(!path.try_exists()?, "verified capture WAV cleanup failed");
            manifest["recordings"][take_index]["capture_wav_removed"] = true.into();
            write_manifest(&manifest_path, &manifest)?;
        }
    }
    anyhow::ensure!(
        manifest["captures"]
            .as_array()
            .is_some_and(|frames| frames.len() == 12)
            && manifest["recordings"]
                .as_array()
                .is_some_and(|takes| takes.len() == 6),
        "recording formats requires 12 native frames and six verified WAVs"
    );
    dispatch(shell, AppRequest::Stop, cx)?;
    wait_for(
        shell,
        "stopped format capture audio",
        |_, snapshot| snapshot.phase == SessionPhase::Idle,
        cx,
    )
    .await?;
    manifest["complete"] = true.into();
    write_manifest(&manifest_path, &manifest)?;
    eprintln!("Recording formats: 12 frames, six verified WAVs removed");
    Ok(())
}

fn new_destination(directory: &Path, density: &str, format: &str) -> anyhow::Result<PathBuf> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = directory.join(format!(
        "recording-{density}-{format}-{}-{unique}.wav",
        std::process::id()
    ));
    anyhow::ensure!(
        !path.try_exists()?,
        "capture WAV destination already exists"
    );
    Ok(path)
}

fn write_manifest(path: &Path, manifest: &serde_json::Value) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(manifest)?)?;
    Ok(())
}

async fn click_format(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    format: RecordingFormat,
    slug: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let camera = shell.update(cx, |shell, _| shell.camera);
    window.update(cx, |_, window, cx| -> anyhow::Result<()> {
        window.render_frame(cx);
        verify_format_controls(window, node)?;
        window.click(format!("record-format-{node:?}-{slug}"), cx);
        Ok(())
    })??;
    wait_for(
        shell,
        "native format selection and updated preflight",
        |_, snapshot| {
            snapshot.recordings.iter().any(|r| {
                r.node == node
                    && r.format == format
                    && r.phase == RecordingPhase::Idle
                    && !r.estimate.preflight_pending
            })
        },
        cx,
    )
    .await?;
    anyhow::ensure!(
        shell.update(cx, |shell, _| shell.camera) == camera,
        "recording format click moved the canvas"
    );
    Ok(())
}

fn verify_format_controls(window: &Window, node: NodeId) -> anyhow::Result<()> {
    let canvas = window.find("graph-canvas-input").bounds();
    let native = native_measurements(window, &[node])?;
    let body = native.bodies[&node];
    for (_, slug, _) in FORMATS {
        let control = window
            .try_find(format!("record-format-{node:?}-{slug}"))
            .ok_or_else(|| anyhow::anyhow!("missing native recording format control: {slug}"))?;
        let mut bounds = pixel_bounds(control.bounds());
        bounds.origin.x -= f32::from(canvas.left());
        bounds.origin.y -= f32::from(canvas.top());
        anyhow::ensure!(
            control.visible() && contains_native_bounds(body, bounds),
            "recording format control {slug} is clipped"
        );
    }
    Ok(())
}

fn verify_busy_format_state(
    shell: &StudioShell,
    node: NodeId,
    format: RecordingFormat,
    cx: &App,
) -> anyhow::Result<()> {
    let state = shell.state.read(cx);
    anyhow::ensure!(
        !state.pending_request && state.dispatch_error.is_none(),
        "busy recording format control dispatched a request"
    );
    anyhow::ensure!(
        state.snapshot.phase == SessionPhase::Running
            && state.snapshot.error.is_none()
            && state.snapshot.recordings.iter().any(|r| {
                r.node == node && r.format == format && r.phase.is_busy() && r.error.is_none()
            }),
        "busy recording format click changed the service format or lifecycle"
    );
    Ok(())
}

async fn verify_busy_format_clicks(
    window: AnyWindowHandle,
    shell: &Entity<StudioShell>,
    node: NodeId,
    format: RecordingFormat,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let camera = shell.update(cx, |shell, _| shell.camera);
    for (_, slug, _) in FORMATS {
        window.update(cx, |_, window, cx| -> anyhow::Result<()> {
            window.render_frame(cx);
            verify_format_controls(window, node)?;
            verify_busy_format_state(shell.read(cx), node, format, cx)?;
            window.click(format!("record-format-{node:?}-{slug}"), cx);
            // base::Button does not publish a disabled observation. Check the
            // actual click before polling can acknowledge an erroneous request.
            verify_busy_format_state(shell.read(cx), node, format, cx)
        })??;
        cx.background_executor()
            .timer(Duration::from_millis(75))
            .await;
        window.update(cx, |_, window, cx| -> anyhow::Result<()> {
            shell.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
            });
            window.render_frame(cx);
            verify_format_controls(window, node)?;
            let shell = shell.read(cx);
            verify_busy_format_state(shell, node, format, cx)?;
            anyhow::ensure!(shell.camera == camera, "busy format click moved the canvas");
            Ok(())
        })??;
    }
    Ok(())
}

fn click_transport(window: AnyWindowHandle, node: NodeId, cx: &mut AsyncApp) -> anyhow::Result<()> {
    window.update(cx, |_, window, cx| -> anyhow::Result<()> {
        window.render_frame(cx);
        let key = format!("record-transport-{node:?}");
        let button = window
            .try_find(key.clone())
            .ok_or_else(|| anyhow::anyhow!("missing recorder transport"))?;
        let canvas = window.find("graph-canvas-input").bounds();
        let native = native_measurements(window, &[node])?;
        let mut bounds = pixel_bounds(button.bounds());
        bounds.origin.x -= f32::from(canvas.left());
        bounds.origin.y -= f32::from(canvas.top());
        anyhow::ensure!(
            button.visible() && contains_native_bounds(native.bodies[&node], bounds),
            "recorder transport is not visible within the card"
        );
        window.click(key, cx);
        Ok(())
    })??;
    Ok(())
}

async fn wait_finished(
    shell: &Entity<StudioShell>,
    node: NodeId,
    format: RecordingFormat,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    wait_for(
        shell,
        "saved format recording with audio still running",
        |_, snapshot| {
            snapshot.phase == SessionPhase::Running
                && snapshot.recordings.iter().any(|r| {
                    r.node == node
                        && r.phase == RecordingPhase::Finished
                        && r.format == format
                        && r.written_frames > 0
                })
        },
        cx,
    )
    .await
}

fn verify_frame(
    frame: &serde_json::Value,
    format: RecordingFormat,
    phase: RecordingPhase,
) -> anyhow::Result<()> {
    let recording = &frame["recordings"][0];
    anyhow::ensure!(
        recording["format"].as_str() == Some(format.label())
            && recording["phase"].as_str() == Some(format!("{phase:?}").as_str())
            && recording["path"].is_string()
            && recording["error"].is_null()
            && if phase == RecordingPhase::Idle {
                recording["written_frames"].as_u64() == Some(0)
            } else {
                recording["written_frames"]
                    .as_u64()
                    .is_some_and(|frames| frames > 0)
            },
        "native recording frame does not match selected format/lifecycle"
    );
    Ok(())
}

fn verify_saved(
    path: &Path,
    saved: &RecordingSnapshot,
    encoding: wav_validation::Encoding,
) -> anyhow::Result<wav_validation::WavEvidence> {
    anyhow::ensure!(
        saved.phase == RecordingPhase::Finished
            && saved.error.is_none()
            && saved.path.as_deref() == Some(path),
        "recording did not finalize this capture artifact"
    );
    // These are deliberately short takes; reject runaway recordings rather
    // than allocating a huge WAV just to collect acceptance evidence.
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 64 * 1024 * 1024,
        "format capture exceeded its short-take size budget"
    );
    wav_validation::verify_bytes(
        &std::fs::read(path)?,
        encoding,
        saved.written_frames,
        saved
            .sample_rate
            .ok_or_else(|| anyhow::anyhow!("missing confirmed rate"))?,
        saved
            .channels
            .ok_or_else(|| anyhow::anyhow!("missing confirmed channels"))?,
    )
}
