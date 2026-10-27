//! File transport is owned by the service, so closing a panel cannot stop it.
use super::*;
use crate::service::{RecordingPhase, RecordingSnapshot};

pub(super) fn content(shell: &StudioShell, node: NodeId, cx: &Context<StudioShell>) -> AnyElement {
    let snapshot = &shell.state.read(cx).snapshot;
    let recording = snapshot.recordings.iter().find(|r| r.node == node);
    let busy = recording.is_some_and(|r| r.phase.is_busy());
    let path = recording.and_then(|r| r.path.as_ref());
    let directory = path
        .and_then(|p| p.parent())
        .map(|p| p.display().to_string());
    let filename = path
        .and_then(|p| p.file_name())
        .map(|p| p.to_string_lossy().to_string());
    let this = cx.entity();
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(7.))
        .child(note("Destination"))
        .child(
            controls::button(
                format!("record-path-{node:?}"),
                directory
                    .clone()
                    .map(|directory| compact_path(&directory, 42))
                    .unwrap_or_else(|| t!("Choose output file").to_string()),
            )
            .tooltip(directory.unwrap_or_else(|| {
                t!("Choose a new .wav file; existing files are never overwritten").to_string()
            }))
            .full_width()
            .justify_start()
            .truncate_label()
            .disabled(busy)
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| shell.choose_recording_path(node, cx))
            }),
        )
        .child(
            div()
                .text_size(px(11.))
                .font_family(theme::FONT_MONO)
                .text_ellipsis()
                .child(filename.unwrap_or_else(|| t!("No file selected").to_string())),
        )
        .child(detail_line("Format", "WAV · 32-bit float"))
        .child(note("Format fixed · graph sample rate and channels"));
    if let Some(path) = path {
        let target = path.clone();
        view = view.child(
            controls::button(format!("record-reveal-{node:?}"), t!("Open output folder"))
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, _, cx| cx.reveal_path(&target)),
        );
    }
    if let Some(r) = recording {
        let rate = r.sample_rate.map_or_else(
            || t!("Graph rate").to_string(),
            |v| format!("{:.1} kHz", f64::from(v) / 1000.),
        );
        let channels = r
            .channels
            .map_or_else(|| t!("Graph channels").to_string(), |v| format!("{v} ch"));
        view = view.child(detail_line("Audio", &format!("{rate} · {channels}")));
        if let (Some(rate), Some(channels)) = (
            r.estimate.proposed_sample_rate,
            r.estimate.proposed_channels,
        ) {
            view = view.child(detail_line(
                "Proposed format",
                &format!("{:.1} kHz · {channels} ch", f64::from(rate) / 1000.),
            ));
        }
        if r.estimate.preflight_pending {
            view = view.child(note("Checking disk space"));
        }
    }
    let phase = recording.map_or(RecordingPhase::Idle, |r| r.phase);
    let status = match phase {
        RecordingPhase::Idle if path.is_none() => "Choose output file",
        RecordingPhase::Idle if snapshot.phase != SessionPhase::Running => {
            "Start audio before recording"
        }
        RecordingPhase::Idle => "Ready",
        RecordingPhase::Starting => "Starting",
        RecordingPhase::Recording => "Recording",
        RecordingPhase::Stopping => "Finalizing file",
        RecordingPhase::Canceled => "Canceled before opening file",
        RecordingPhase::Finished => "Saved",
        RecordingPhase::Failed => "Recording failed",
    };
    let color = match phase {
        RecordingPhase::Recording => theme::DANGER,
        RecordingPhase::Failed => theme::DANGER,
        RecordingPhase::Starting | RecordingPhase::Stopping => theme::WARNING,
        _ => theme::TEXT_MUTED,
    };
    let elapsed = recording
        .and_then(|r| {
            r.sample_rate
                .filter(|rate| *rate > 0)
                .map(|rate| r.written_frames as f64 / f64::from(rate))
        })
        .unwrap_or(0.);
    view = view
        .child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(controls::badge(t!(status), color))
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(20.))
                        .child(duration(elapsed)),
                ),
        )
        .child(detail_line("Remaining", &remaining(recording)));
    if let Some(recording) = recording {
        view = view.child(note(recording.estimate.basis.label()));
    }
    if let Some(r) = recording {
        if let Some(bytes) = r.estimate.free_disk_bytes {
            view = view.child(detail_line(
                "Free disk",
                &format!("{:.2} GiB", bytes as f64 / 1_073_741_824.),
            ));
        }
        if let Some(stats) = r.stats
            && stats.dropped_frames > 0
        {
            view = view.child(
                div().text_size(px(11.)).text_color(theme::DANGER).child(
                    t!(
                        "%{count} dropped frames (endpoint total)",
                        count = stats.dropped_frames
                    )
                    .to_string(),
                ),
            );
        }
        if let Some(error) = &r.error {
            view = view.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::DANGER)
                    .child(error.clone()),
            );
        }
    }
    let this = cx.entity();
    let action = if busy {
        AppRequest::StopRecording { node }
    } else {
        AppRequest::StartRecording { node }
    };
    view.child(
        controls::button(
            format!("record-transport-{node:?}"),
            t!(if busy {
                "Stop recording"
            } else {
                "Start recording"
            }),
        )
        .variant(if busy {
            ButtonVariant::Danger
        } else {
            ButtonVariant::Primary
        })
        .disabled(
            matches!(phase, RecordingPhase::Stopping)
                || (!busy && (path.is_none() || snapshot.phase != SessionPhase::Running)),
        )
        .tooltip(if snapshot.phase != SessionPhase::Running {
            t!("Start audio before recording")
        } else {
            t!("Recording transport does not stop the audio graph")
        })
        .full_width()
        .on_click(move |_, _, cx| {
            this.update(cx, |s, cx| {
                s.state.update(cx, |s, _| s.dispatch(action.clone()));
                cx.notify();
            })
        }),
    )
    .into_any_element()
}

impl StudioShell {
    fn choose_recording_path(&mut self, node: NodeId, cx: &mut Context<Self>) {
        let revision = self.state.read(cx).snapshot.graph.candidate_revision;
        let directory = self
            .state
            .read(cx)
            .snapshot
            .recordings
            .iter()
            .find(|r| r.node == node)
            .and_then(|r| r.path.as_ref())
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let prompt = cx.prompt_for_new_path(&directory, Some("Take.wav"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = prompt.await {
                let _ = this.update(cx, |shell, cx| {
                    let graph = &shell.state.read(cx).snapshot.graph;
                    // A native picker may outlive a loaded project. Reject its
                    // stale result even if the new document reuses this NodeId.
                    if graph.candidate_revision == revision
                        && graph.kind_of(node) == Some(NodeKind::Recorder)
                        && shell.pending_project.is_none()
                    {
                        shell.state.update(cx, |s, _| {
                            s.dispatch(AppRequest::SetRecordingDestination { node, path })
                        });
                    } else {
                        shell.project_message = Some(
                            t!("The graph changed. Choose the recording destination again.")
                                .to_string(),
                        );
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

fn remaining(recording: Option<&RecordingSnapshot>) -> String {
    let Some(estimate) = recording.map(|r| &r.estimate) else {
        return t!("Unknown until prepared").to_string();
    };
    match (estimate.remaining_seconds, estimate.free_disk_bytes) {
        (Some(seconds), Some(_)) => format!("~{}", duration(seconds)),
        (Some(seconds), None) => t!(
            "%{time} RIFF limit · disk unknown",
            time = duration(seconds)
        )
        .to_string(),
        _ => t!("Unknown until prepared").to_string(),
    }
}

fn duration(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.) as u64
    } else {
        0
    };
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Keep the volume and destination folder visible without painting over the
/// neighboring meter. The tooltip retains the full, unmodified path.
fn compact_path(path: &str, limit: usize) -> String {
    let chars: Vec<_> = path.chars().collect();
    if chars.len() <= limit {
        return path.to_owned();
    }
    let prefix: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - (limit - 11)..].iter().collect();
    format!("{prefix}...{tail}")
}

#[cfg(test)]
mod tests {
    use super::{compact_path, duration, remaining};
    #[::core::prelude::v1::test]
    fn recording_path_is_bounded_and_keeps_destination_suffix() {
        let path = "C:\\Sessions\\a-very-long-project-name\\stems\\recordings";
        let compact = compact_path(path, 42);
        assert_eq!(compact.chars().count(), 42);
        assert!(compact.starts_with("C:\\"));
        assert!(compact.ends_with("\\recordings"));
        assert_eq!(compact_path("C:\\Takes", 42), "C:\\Takes");
    }
    #[::core::prelude::v1::test]
    fn recording_times_are_file_based_and_unknown_capacity_stays_unknown() {
        assert_eq!(duration(3661.9), "01:01:01");
        assert_eq!(remaining(None), "Unknown until prepared");
    }
}
