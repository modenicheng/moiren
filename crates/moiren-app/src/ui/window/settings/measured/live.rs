//! Explicit live-window selection, separate from per-wake and stopped replay.
use super::*;
use crate::service::{MeasuredProcessSummary, ProcessWindowPolicy};

#[derive(Default)]
pub(super) struct PolicyDraft {
    // None is intentionally not an admission default, including the two
    // startup allowances where zero is a meaningful explicit choice.
    pub values: [Option<u64>; 5],
}

impl PolicyDraft {
    pub(super) fn policy(&self) -> Result<ProcessWindowPolicy, String> {
        let [
            Some(unserved),
            Some(discarded),
            Some(frames),
            Some(reads),
            Some(span),
        ] = self.values
        else {
            return Err("Select all five live observed-window policy fields explicitly.".into());
        };
        let policy = ProcessWindowPolicy {
            max_initial_unserved_frames: unserved,
            max_initial_prime_discarded_frames: discarded,
            min_output_frames: frames,
            min_output_reads: reads,
            min_output_span_100ns: span,
        };
        policy.validate()?;
        Ok(policy)
    }
}

pub(super) fn open_button(cx: &Context<StudioShell>) -> impl IntoElement {
    let this = cx.entity();
    button(
        "measured-live-window-open",
        "Use live observed-window admission",
    )
    .on_click(move |_, _, cx| {
        this.update(cx, |shell, cx| {
            if !shell.settings.busy() {
                shell.settings.measured.live_open = true;
                cx.notify();
            }
        })
    })
}

fn policy_summary(policy: ProcessWindowPolicy) -> String {
    format!(
        "{} / {} / {} / {} / {}",
        policy.max_initial_unserved_frames,
        policy.max_initial_prime_discarded_frames,
        policy.min_output_frames,
        policy.min_output_reads,
        policy.min_output_span_100ns
    )
}

fn selection_summary(
    settings: crate::service::TransportLatencySettings,
    measured: Option<&MeasuredProcessSummary>,
) -> String {
    let input = transport::process_summary(settings, measured);
    let mode = match measured {
        Some(summary) => match &summary.observed_window {
            Some(window) => format!(
                "live observed-window policy (initial unserved / prime discard / output frames / reads / span in raw 100ns): {}; cutoff {} (100ns); replayed packet/read events {} / {}; physical latency remains unqualified",
                policy_summary(window.policy),
                window.cutoff_100ns,
                window.replayed_packet_events,
                window.replayed_read_events
            ),
            None => "per-wake measured admission; no observed-window selection".into(),
        },
        None => "no observed-window selection".into(),
    };
    format!("{input}; {mode}")
}

pub(super) fn editor(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let measured = &shell.settings.measured;
    let policy = measured.live_policy.policy();
    let request = policy
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|_| measured.draft.live_request(snapshot));
    let blocked = conflict(shell, cx);
    let feedback = measured.error.clone().or_else(|| measured.message.clone())
        .or_else(|| blocked.clone())
        .or_else(|| request.as_ref().err().cloned())
        .unwrap_or_else(|| "Ready to evaluate one bounded window from the original running Process owner. Explicit Apply replaces the owner only if that exact policy is admitted; physical latency remains unqualified.".into());
    let this = cx.entity();
    let mut view = div().flex().flex_col().gap(px(8.))
        .child(button("measured-live-window-close", t!("Back to measured input"))
            .disabled(shell.settings.busy())
            .on_click(move |_, _, cx| this.update(cx, |shell, cx| {
                if !shell.settings.busy() {
                    shell.settings.measured.live_open = false;
                    cx.notify();
                }
            })))
        .child(heading("Live observed-window Process admission"))
        .child(label("Opt-in bounded native prefix evaluation and explicit owner replacement, not continuous checkpoints, future-safe delivery, source sample age or qualified physical latency. Missing, overwritten or unsupported evidence is refused; there is no per-wake fallback."))
        .child(label(format!("{}: {} / {} / {}", t!("Capacity / target / jitter (frames)"),
            measured.draft.capacity_frames, measured.draft.target_frames, measured.draft.jitter_frames)));
    for (kind, title, values) in [
        (
            0,
            "Maximum initial unserved (output frames)",
            vec![0_u64, 256, 4096, 16384],
        ),
        (
            1,
            "Maximum initial prime discard (input frames)",
            vec![0, 256, 4096, 16384],
        ),
        (2, "Minimum output frames", vec![96, 480, 4800, 48000]),
        (3, "Minimum output reads", vec![2, 10, 100]),
        (
            4,
            "Minimum output span (raw 100ns)",
            vec![10_000, 100_000, 1_000_000, 10_000_000],
        ),
    ] {
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (index, value) in values.into_iter().enumerate() {
            let this = cx.entity();
            row = row.child(
                button(
                    SharedString::from(format!("measured-live-window-policy-{kind}-{index}")),
                    value.to_string(),
                )
                .variant(if measured.live_policy.values[kind] == Some(value) {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .disabled(shell.settings.busy())
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        if !shell.settings.busy() {
                            shell.settings.measured.live_policy.values[kind] = Some(value);
                            shell.settings.measured.changed();
                            cx.notify();
                        }
                    })
                }),
            );
        }
        view = view.child(field(title, row));
    }
    let this = cx.entity();
    view.child(
        button(
            "measured-live-window-apply",
            "Apply live observed-window budget and restart",
        )
        .variant(ButtonVariant::Primary)
        .disabled(blocked.is_some() || request.is_err())
        .on_click(move |_, _, cx| this.update(cx, |shell, cx| shell.apply_live_process_window(cx))),
    )
    .child(
        div()
            .id("measured-live-window-feedback")
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
        "measured-live-window-requested",
        &t!("Requested process input"),
        selection_summary(
            snapshot.transport_latency,
            snapshot.measured_process.as_ref(),
        ),
    ))
    .child(summary(
        "measured-live-window-active",
        &t!("Active process request"),
        if snapshot.phase == SessionPhase::Running {
            snapshot.active_transport_latency.map(|settings| {
                selection_summary(settings, snapshot.active_measured_process.as_ref())
            })
        } else {
            None
        }
        .unwrap_or_else(|| t!("No active audio configuration").into()),
    ))
}

impl StudioShell {
    fn apply_live_process_window(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = conflict(self, cx) {
            if !self.settings.measured.busy() {
                self.settings.measured.error = Some(error);
            }
            cx.notify();
            return;
        }
        let snapshot = self.state.read(cx).snapshot.clone();
        let preflight = self
            .settings
            .measured
            .live_policy
            .policy()
            .and_then(|policy| {
                self.settings
                    .measured
                    .draft
                    .live_request(&snapshot)
                    .map(|request| (request, policy))
            });
        let (request, policy) = match preflight {
            Ok(value) => value,
            Err(error) => {
                self.settings.measured.error = Some(error);
                cx.notify();
                return;
            }
        };
        match self.state.update(cx, |state, _| {
            state.try_dispatch(AppRequest::ApplyWindowedProcessBridge { request, policy })
        }) {
            Ok(id) => {
                self.settings
                    .measured
                    .begin_live(id, request, policy, &snapshot);
                self.measured_deadline(id, cx);
            }
            Err(error) => self.settings.measured.error = Some(format!("{error:?}")),
        }
        cx.notify();
    }
}
