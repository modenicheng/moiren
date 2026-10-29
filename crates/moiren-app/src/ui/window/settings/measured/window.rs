//! Separate diagnostic evaluation and explicit, conservatively admitted Apply.
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
        .child(label(t!("Diagnostic replay uses declared assumptions. Explicit Apply separately requires final whole-drain and whole-wake admission plus the exact evaluated window; future delivery and physical latency remain unqualified.")))
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
    let admission = editor.admission_feedback(snapshot, measured.draft).map(str::to_owned)
        .unwrap_or_else(|| if let Some(candidate) = editor.candidate(snapshot, measured.draft) {
            let result = candidate.assessment();
            let options = result.report.options;
            let delivery = options.delivery.expect("evaluated candidate has measured delivery");
            format!("{} {}: {} / {} @ {} Hz / {} @ {} Hz; {}: {} / {} / {} / {} / {} (100ns)",
                t!("Final whole-wake admission and this window permit explicit Apply; physical latency is unqualified."),
                t!("Empirical whole-drain / gap / whole-wake"),
                delivery.max_packet_frames, delivery.max_gap_frames, delivery.sample_rate,
                options.max_demand_frames, options.engine_sample_rate,
                t!("Window policy: initial unserved / prime discard / output frames / reads / span"),
                result.assessment.policy.max_initial_unserved_frames,
                result.assessment.policy.max_initial_prime_discarded_frames,
                result.assessment.policy.min_output_frames,
                result.assessment.policy.min_output_reads,
                result.assessment.policy.min_output_span_100ns)
        } else { t!("No evaluated native candidate is available.").into() });
    let apply_conflict = conflict(shell, cx);
    let stopped = snapshot.phase == SessionPhase::Idle;
    let this_apply = cx.entity();
    let activation = measured
        .error
        .clone()
        .or_else(|| measured.message.clone())
        .unwrap_or_default();
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
            .id("process-window-admission")
            .aria_label(admission.clone())
            .text_size(px(theme::TEXT_SM))
            .child(admission)
            .gpui_test_observe(),
    )
    .child(
        button(
            "process-window-apply",
            t!("Apply evaluated budget and start audio"),
        )
        .variant(ButtonVariant::Primary)
        .disabled(
            !stopped
                || editor.busy()
                || apply_conflict.is_some()
                || editor.candidate(snapshot, measured.draft).is_none(),
        )
        .on_click(move |_, _, cx| {
            this_apply.update(cx, |shell, cx| shell.apply_evaluated_process(cx))
        }),
    )
    .child(
        div()
            .id("process-window-activation")
            .aria_label(activation.clone())
            .text_size(px(theme::TEXT_SM))
            .child(activation)
            .gpui_test_observe(),
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
                .spawn(async move { (work.evaluate(), work.admit()) })
                .await;
            let _ = this.update(cx, |shell, cx| {
                let snapshot = shell.state.read(cx).snapshot.clone();
                let measured = &mut shell.settings.measured;
                measured
                    .window
                    .finish(&job, &snapshot, measured.draft, result.0);
                measured
                    .window
                    .finish_admission(&job, &snapshot, measured.draft, result.1);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_evaluated_process(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = conflict(self, cx) {
            if !self.settings.measured.busy() {
                self.settings.measured.error = Some(error);
            }
            cx.notify();
            return;
        }
        let snapshot = self.state.read(cx).snapshot.clone();
        let candidate = self
            .settings
            .measured
            .window
            .candidate(&snapshot, self.settings.measured.draft)
            .cloned();
        let Some(candidate) = candidate.filter(|_| {
            snapshot.phase == SessionPhase::Idle && !self.settings.measured.window.busy()
        }) else {
            self.settings.measured.error =
                Some(t!("A stopped matching evaluated native candidate is required.").into());
            cx.notify();
            return;
        };
        let request = candidate.request();
        match self.state.update(cx, |state, _| {
            state.try_dispatch(AppRequest::ApplyEvaluatedProcessBridge(candidate))
        }) {
            Ok(id) => {
                self.settings
                    .measured
                    .begin_retained(id, request, &snapshot);
                self.measured_deadline(id, cx);
            }
            Err(error) => self.settings.measured.error = Some(format!("{error:?}")),
        }
        cx.notify();
    }
}
