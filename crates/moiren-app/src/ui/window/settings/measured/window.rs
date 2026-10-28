//! Desktop access to finite diagnostics; never dispatches an audio request.
use super::*;
use moiren_windows_audio::process_loopback::ReplayWindowStatus;

pub(super) fn open_button(cx: &Context<StudioShell>) -> impl IntoElement {
    let this = cx.entity();
    button("process-window-open", t!("Evaluate closed Process prefix")).on_click(move |_, _, cx| {
        this.update(cx, |shell, cx| {
            shell.settings.measured.window_open = true;
            cx.notify();
        })
    })
}

pub(super) fn editor(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let measured = &shell.settings.measured;
    let editor = &measured.window;
    let policy = editor.policy;
    let this = cx.entity();
    let mut view = div().flex().flex_col().gap(px(8.))
        .child(button("process-window-close", t!("Back to measured input"))
            .on_click(move |_, _, cx| this.update(cx, |shell, cx| {
                shell.settings.measured.window_open = false;
                cx.notify();
            })))
        .child(heading(t!("Closed Process prefix evaluation")))
        .child(label(t!("Diagnostic replay only. Declared packet/gap assumptions do not bound native drains, future delivery or sample age. Live Apply still requires independent whole-wake admission.")))
        .child(label(format!("{}: {} / {} / {}", t!("Capacity / target / jitter (frames)"),
            measured.draft.capacity_frames, measured.draft.target_frames, measured.draft.jitter_frames)));
    if let Some(evidence) = window_state::retained(snapshot) {
        let origin = evidence.origin();
        view = view.child(label(format!(
            "{}: session {} / document {} / graph {} / native {} / PID {} / created {} / node {:?}",
            t!("Retained origin"),
            origin.generation.0,
            origin.document,
            origin.graph_revision,
            origin.native_owner_generation,
            origin.process.pid,
            origin.process.creation_time_100ns,
            origin.node
        )));
    }
    for (kind, title, values, selected) in [
        (
            0,
            "Maximum initial unserved (output frames)",
            vec![0, 256, 4096, 16384],
            policy.max_initial_unserved_frames,
        ),
        (
            1,
            "Maximum initial prime discard (input frames)",
            vec![0, 256, 4096, 16384],
            policy.max_initial_prime_discarded_frames,
        ),
        (
            2,
            "Minimum output frames",
            vec![96, 480, 4800, 48000],
            policy.min_output_frames,
        ),
        (
            3,
            "Minimum output reads",
            vec![2, 10, 100],
            policy.min_output_reads,
        ),
        (
            4,
            "Minimum output span (ms)",
            vec![1, 10, 100, 1000],
            policy.min_output_span_100ns / 10_000,
        ),
    ] {
        let mut row = div().flex().flex_wrap().gap(px(4.));
        for (index, value) in values.into_iter().enumerate() {
            let this = cx.entity();
            row = row.child(
                button(
                    SharedString::from(format!("process-window-policy-{kind}-{index}")),
                    value.to_string(),
                )
                .variant(if selected == value {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        let window = &mut shell.settings.measured.window;
                        match kind {
                            0 => window.policy.max_initial_unserved_frames = value,
                            1 => window.policy.max_initial_prime_discarded_frames = value,
                            2 => window.policy.min_output_frames = value,
                            3 => window.policy.min_output_reads = value,
                            _ => window.policy.min_output_span_100ns = value * 10_000,
                        }
                        window.changed();
                        cx.notify();
                    })
                }),
            );
        }
        view = view.child(field(&t!(title), row));
    }
    let feedback = if let Some(result) = editor.result(snapshot, measured.draft) {
        let status = match result.assessment.status {
            ReplayWindowStatus::ObservedViolation => t!("Observed violation"),
            ReplayWindowStatus::Inconclusive => t!("Insufficient exposure"),
            ReplayWindowStatus::NoViolationInWindow => t!("No violation in this window"),
        };
        let report = &result.report;
        let delivery = report.options.delivery.unwrap();
        format!(
            "{status}; {}. {:?}; Declared packet/gap {} / {} @ {} Hz; whole wake {} @ {} Hz; {} / {} / {}; output {} frames / {} reads / {:?} (100ns); violations {:?}; insufficient frames/reads/span {} / {} / {}",
            t!("Not qualified: no sustained, waveform, sample-age or physical latency proof"),
            report.trace_origin,
            delivery.max_packet_frames,
            delivery.max_gap_frames,
            delivery.sample_rate,
            report.options.max_demand_frames,
            report.options.engine_sample_rate,
            measured.draft.capacity_frames,
            measured.draft.target_frames,
            measured.draft.jitter_frames,
            report.candidate.output_frames,
            report.inventory.output_reads,
            report.inventory.output_span_lower_100ns,
            result.assessment.violations,
            result.assessment.insufficient_output_frames,
            result.assessment.insufficient_output_reads,
            result.assessment.insufficient_output_span
        )
    } else if editor.busy() {
        t!("Evaluating one bounded prefix on a background worker.").into()
    } else {
        editor
            .feedback_error(snapshot, measured.draft)
            .map(str::to_owned)
            .or_else(|| editor.available(snapshot, measured.draft).err())
            .unwrap_or_else(|| t!("Ready to evaluate; audio will not change.").into())
    };
    let this = cx.entity();
    view.child(
        button("process-window-evaluate", t!("Evaluate candidate window"))
            .variant(ButtonVariant::Primary)
            .disabled(editor.busy() || editor.available(snapshot, measured.draft).is_err())
            .on_click(move |_, _, cx| {
                this.update(cx, |shell, cx| shell.evaluate_process_window(cx))
            }),
    )
    .child(
        div()
            .id("process-window-feedback")
            .aria_label(feedback.clone())
            .text_size(px(theme::TEXT_SM))
            .child(feedback)
            .gpui_test_observe(),
    )
}

impl StudioShell {
    fn evaluate_process_window(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.state.read(cx).snapshot.clone();
        let measured = &mut self.settings.measured;
        let job = match measured.window.begin(&snapshot, measured.draft) {
            Ok(job) => job,
            Err(error) => {
                measured.window.error = Some(error);
                cx.notify();
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let work = job.clone();
            let result = cx
                .background_executor()
                .spawn(async move { work.evaluate() })
                .await;
            let _ = this.update(cx, |shell, cx| {
                let snapshot = shell.state.read(cx).snapshot.clone();
                let measured = &mut shell.settings.measured;
                measured
                    .window
                    .finish(&job, &snapshot, measured.draft, result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
