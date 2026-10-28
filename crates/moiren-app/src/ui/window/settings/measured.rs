//! Session-local measured selection; persisted estimates are a separate draft.
use super::*;
use crate::service::{AppSnapshot, InputSelection, MeasuredProcessRequest, RequestId};
use moiren_core::graph::NodeId;

mod state;
pub(super) use state::MeasuredEditor;

pub(super) fn editor(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let measured = &shell.settings.measured;
    let blocked = conflict(shell, cx);
    let preflight = measured.draft.request(snapshot);
    let feedback = measured
        .error
        .clone()
        .or_else(|| measured.message.clone())
        .or_else(|| blocked.clone())
        .or_else(|| preflight.as_ref().err().cloned())
        .unwrap_or_else(|| {
            t!("Ready to validate the measured input budget and restart audio.").into()
        });
    let mut sources = div().flex().flex_wrap().gap(px(4.));
    for (index, node) in snapshot
        .graph
        .nodes
        .iter()
        .filter(|node| matches!(node.source, Some(InputSelection::Process { .. })))
        .enumerate()
    {
        let id = node.id;
        let this = cx.entity();
        sources = sources.child(
            button(
                SharedString::from(format!("measured-process-node-{index}")),
                node.label.clone(),
            )
            .variant(if measured.draft.selected_node(snapshot) == Some(id) {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Secondary
            })
            .disabled(shell.settings.busy())
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| {
                    if !shell.settings.busy() {
                        shell.settings.measured.draft.node = Some(id);
                        shell.settings.measured.changed();
                        cx.notify();
                    }
                })
            }),
        );
    }
    let this = cx.entity();
    let mut view = div().flex().flex_col().gap(px(10.))
        .child(heading(t!("Measured Process input")))
        .child(label(t!("Uses this running source's observed delivery and whole-wake demand. Restart replaces the owner; it does not qualify continuous output or physical latency.")))
        .child(field(&t!("Process source"), sources));
    for (kind, title, values, selected) in [
        (
            "capacity",
            "Ring capacity",
            vec![512, 1024, 2048, 4096, 8192, 16384, 32768],
            measured.draft.capacity_frames,
        ),
        (
            "target",
            "Normal input target",
            vec![64, 96, 128, 256, 512, 1024, 2048, 4096],
            measured.draft.target_frames,
        ),
        (
            "jitter",
            "Jitter reserve",
            vec![0, 32, 64, 96, 128, 256, 480],
            measured.draft.jitter_frames,
        ),
    ] {
        let mut row = div().flex().flex_wrap().gap(px(4.));
        let mut values = values;
        if !values.contains(&selected) {
            values.push(selected);
        }
        for (index, value) in values.into_iter().enumerate() {
            let this = cx.entity();
            row = row.child(
                button(
                    SharedString::from(format!("measured-{kind}-{index}")),
                    value.to_string(),
                )
                .variant(if value == selected {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .disabled(shell.settings.busy())
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        if shell.settings.busy() {
                            return;
                        }
                        let measured = &mut shell.settings.measured;
                        match kind {
                            "capacity" => measured.draft.capacity_frames = value,
                            "target" => measured.draft.target_frames = value,
                            _ => measured.draft.jitter_frames = value,
                        }
                        measured.changed();
                        cx.notify();
                    })
                }),
            );
        }
        view = view.child(field(&t!(title), row));
    }
    view.child(
        button(
            "measured-process-apply",
            t!("Apply measured budget and restart"),
        )
        .variant(ButtonVariant::Primary)
        .disabled(blocked.is_some() || preflight.is_err())
        .on_click(move |_, _, cx| this.update(cx, |shell, cx| shell.apply_measured_process(cx))),
    )
    .child(
        div()
            .id("measured-process-feedback")
            .aria_label(feedback.clone())
            .text_size(px(theme::TEXT_SM))
            .text_color(if measured.error.is_some() {
                theme::DANGER
            } else {
                theme::TEXT_MUTED
            })
            .child(feedback)
            .gpui_test_observe(),
    )
    .child(summary(
        "measured-process-requested",
        &t!("Requested process input"),
        process_input_summary(
            snapshot.transport_latency.process,
            snapshot.measured_process.as_ref(),
        ),
    ))
    .child(summary(
        "measured-process-active",
        &t!("Active process request"),
        if snapshot.phase == SessionPhase::Running {
            snapshot.active_transport_latency.map(|v| {
                process_input_summary(v.process, snapshot.active_measured_process.as_ref())
            })
        } else {
            None
        }
        .unwrap_or_else(|| t!("No active audio configuration").into()),
    ))
}

fn summary(id: &'static str, title: &str, value: String) -> AnyElement {
    div()
        .id(id)
        .aria_label(format!("{title}: {value}"))
        .text_size(px(theme::TEXT_SM))
        .child(format!("{title}: {value}"))
        .gpui_test_observe()
        .into_any_element()
}

fn conflict(shell: &StudioShell, cx: &Context<StudioShell>) -> Option<String> {
    if shell.settings.busy() || shell.state.read(cx).pending_request {
        Some(t!("Waiting for the current audio transaction.").into())
    } else if shell.settings.global.dirty() || shell.settings.device.dirty() {
        Some(t!("Save or discard other settings drafts before applying a measured budget.").into())
    } else {
        None
    }
}

impl StudioShell {
    pub(in crate::ui::window) fn apply_measured_process(&mut self, cx: &mut Context<Self>) {
        // Recheck at activation, not only render: a queued event may be stale.
        if let Some(error) = conflict(self, cx) {
            if !self.settings.measured.busy() {
                self.settings.measured.error = Some(error);
            }
            return;
        }
        let snapshot = self.state.read(cx).snapshot.clone();
        let request = match self.settings.measured.draft.request(&snapshot) {
            Ok(request) => request,
            Err(error) => {
                self.settings.measured.error = Some(error);
                cx.notify();
                return;
            }
        };
        match self.state.update(cx, |state, _| {
            state.try_dispatch(AppRequest::ApplyMeasuredProcessBridge(request))
        }) {
            Ok(id) => {
                self.settings.measured.begin(id, request, &snapshot);
                // A deadline must wake even if preparation stops publishing.
                // This is presentation-only expiry, never a Stop/cancellation.
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_secs(30))
                        .await;
                    let _ = this.update(cx, |shell, cx| {
                        shell.finish_measured_process(cx);
                        shell.settings.measured.expire(id);
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => self.settings.measured.error = Some(format!("{error:?}")),
        }
        cx.notify();
    }

    pub(in crate::ui::window) fn finish_measured_process(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.settings
            .measured
            .observe(&state.snapshot, state.dispatch_error.as_deref());
    }
}

#[cfg(all(test, windows))]
#[path = "measured_tests.rs"]
mod tests;
