//! Real settings clicks against a held, software-only service fixture. These
//! tests prove UI dispatch/confirmation, not native replacement or audio quality.
use crate::service::{
    AppRequest, AppSnapshot, InputSelection, JobPool, MeasuredProcessRequest,
    NativeSessionObservations, RoundBudget, RunLimit, ServiceContext, ServiceDriver,
    ServiceRuntime, SessionPhase, SessionSpec,
    measured_process::{MeasuredProcessProfile, tests::observation},
};
use crate::ui::{
    state::UiState,
    window::{
        StudioShell,
        test_harness::{finish_service, shell_with_state, wait_for, wait_until},
    },
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};
use moiren_core::graph::NodeId;
use moiren_windows_audio::engine_backend::NativeObservationSnapshot;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const REFUSAL: &str = "fixture measured Process apply refused: actual consumer demand changed";

#[derive(Clone, Copy, Debug)]
enum Evidence {
    Current,
    MissingNative,
    MissingProducer,
    StaleSession,
    StaleOwner,
    ReplacedProcess,
}

#[derive(Clone, Copy)]
enum Confirmation {
    SameSession,
    WrongSelection,
    WrongRequest,
    Exact,
}

#[derive(Default)]
struct Control {
    reject: bool,
    clear_active: bool,
    retain_prefix: bool,
    confirmation: Option<Confirmation>,
}

struct MeasuredDriver {
    log: Arc<Mutex<Vec<AppRequest>>>,
    control: Arc<Mutex<Control>>,
    evidence: Evidence,
    initial_selection: bool,
    initialized: bool,
    spec: SessionSpec,
    selected: Option<MeasuredProcessProfile>,
}

impl MeasuredDriver {
    fn initialize(&mut self, context: &mut ServiceContext) {
        context.graph.configure_session(&self.spec).unwrap();
        let generation = context.core.start(self.spec.clone()).unwrap();
        assert!(context.core.activated(generation));
        assert!(context.core.owner_started(generation));
        let graph = context.graph.snapshot();
        let node = graph
            .nodes
            .iter()
            .find(|node| node.source == Some(self.spec.source.clone()))
            .unwrap()
            .id;
        context
            .graph
            .preparing(generation, graph.candidate_revision);
        context.graph.running(generation, graph.candidate_revision);
        context.active_audio_settings = Some(context.audio_settings.clone());
        context.active_transport_latency = Some(context.transport_latency);
        context.active_latency_profile = Some(context.latency_profile);
        if self.initial_selection {
            let request = MeasuredProcessRequest {
                node,
                session_generation: generation,
                native_owner_generation: 7,
                graph_revision: graph.candidate_revision,
                capacity_frames: 4096,
                target_frames: 512,
                jitter_frames: 96,
            };
            let profile =
                MeasuredProcessProfile::from_observation(16, request, &observation(node)).unwrap();
            context.active_measured_process = Some(profile.summary().clone());
            context.measured_process = Some(profile);
        }
        let mut native = NativeSessionObservations {
            generation,
            observation: NativeObservationSnapshot {
                owner_generation: 7,
                running: true,
                streams: Vec::new(),
                bridges: Vec::new(),
                boundary_probes: Vec::new(),
                render_padding: Vec::new(),
                process_delivery: vec![observation(node)],
            },
        };
        match self.evidence {
            Evidence::Current | Evidence::MissingNative => {}
            Evidence::MissingProducer => native.observation.process_delivery[0].producer = None,
            Evidence::StaleSession => native.generation.0 += 1,
            Evidence::StaleOwner => native.observation.owner_generation += 1,
            Evidence::ReplacedProcess => {
                native.observation.process_delivery[0]
                    .process
                    .creation_time_100ns += 1;
            }
        }
        context.native_observations =
            (!matches!(self.evidence, Evidence::MissingNative)).then_some(native);
    }

    fn confirm(&mut self, context: &mut ServiceContext, confirmation: Confirmation) {
        let mut active = self.selected.as_ref().unwrap().summary().clone();
        if !matches!(confirmation, Confirmation::SameSession)
            && context.core.generation() == active.request.session_generation
        {
            // Only simulate the service's confirmed projection. Existing managed
            // tests, not this fixture, verify the actual replacement factories.
            let old = context.core.owner_generation().unwrap();
            assert!(context.core.owner_reaped(old));
            let next = context.core.start(self.spec.clone()).unwrap();
            assert!(context.core.activated(next));
            assert!(context.core.owner_started(next));
        }
        match confirmation {
            Confirmation::WrongSelection => active.selection_id += 1,
            Confirmation::WrongRequest => active.request.target_frames += 1,
            Confirmation::SameSession | Confirmation::Exact => {}
        }
        let generation = context.core.generation();
        let revision = context.graph.snapshot().candidate_revision;
        context.graph.preparing(generation, revision);
        context.graph.running(generation, revision);
        context.active_measured_process = Some(active);
        context.active_audio_settings = Some(context.audio_settings.clone());
        context.active_transport_latency = Some(context.transport_latency);
        context.active_latency_profile = Some(context.latency_profile);
        if !matches!(confirmation, Confirmation::SameSession) {
            let native = context.native_observations.as_mut().unwrap();
            native.generation = generation;
            // Native generations are backend-local; a new app session may
            // legitimately publish the same numeric native generation again.
            native.observation.owner_generation = 7;
            native.observation.process_delivery[0].owner_generation = 7;
        }
    }
}

impl ServiceDriver for MeasuredDriver {
    fn request(&mut self, context: &mut ServiceContext, _: &mut JobPool, request: AppRequest) {
        self.log.lock().unwrap().push(request.clone());
        if let AppRequest::ApplyMeasuredProcessBridge(request) = request {
            if self.control.lock().unwrap().reject {
                context.graph.record_error(REFUSAL);
                return;
            }
            let profile =
                MeasuredProcessProfile::from_observation(17, request, &observation(request.node))
                    .unwrap();
            context.measured_process = Some(profile.clone());
            self.selected = Some(profile);
            context
                .graph
                .preparing(context.core.generation(), request.graph_revision);
        }
    }

    fn poll(&mut self, context: &mut ServiceContext, _: &mut JobPool, _: RoundBudget) {
        if !self.initialized {
            self.initialized = true;
            self.initialize(context);
        }
        if self.control.lock().unwrap().retain_prefix && context.last_cleanup.is_none() {
            let node = context
                .graph
                .snapshot()
                .nodes
                .iter()
                .find(|node| matches!(node.source, Some(InputSelection::Process { .. })))
                .unwrap()
                .id;
            let evidence = crate::service::process_trace::tests::evidence(node);
            context.last_cleanup = Some(crate::service::SessionCleanupObservation {
                generation: evidence.origin().generation,
                state: super::measured::window_tests::cleanup_state(),
                process_trace: Some(Arc::new(evidence)),
            });
            let owner = context.core.owner_generation().unwrap();
            context.core.stop().unwrap();
            assert!(context.core.owner_reaped(owner));
            context.native_observations = None;
            context.active_audio_settings = None;
            context.active_transport_latency = None;
            context.active_latency_profile = None;
        }
        let (clear_active, confirmation) = {
            let mut control = self.control.lock().unwrap();
            (
                std::mem::take(&mut control.clear_active),
                control.confirmation.take(),
            )
        };
        if clear_active {
            context.active_transport_latency = None;
        }
        if let Some(confirmation) = confirmation {
            self.confirm(context, confirmation);
        }
    }

    fn begin_exit(&mut self, context: &mut ServiceContext, _: &mut JobPool) {
        if let Some(generation) = context.core.owner_generation() {
            context.core.owner_reaped(generation);
        }
    }

    fn is_drained(&self) -> bool {
        true
    }
}

#[gpui_kit::test]
fn process_window_settings_click_evaluates_retained_prefix_without_audio_requests(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, false);
    fixture.control.lock().unwrap().retain_prefix = true;
    let (view, cx) = fixture.shell(cx);
    wait_snapshot(&view, cx, |snapshot| snapshot.last_cleanup.is_some());
    select_target(cx);
    cx.update(|window, cx| {
        window.click("process-window-open", cx);
        window.render_frame(cx);
        assert!(window.find("process-window-policy-0-1").visible());
        assert!(window.find("process-window-evaluate").visible());
        window.click("process-window-evaluate", cx);
    });
    wait_until(Duration::from_secs(3), || {
        poll(&view, cx);
        row(cx, "process-window-feedback").contains("No violation in this window")
    });
    let feedback = row(cx, "process-window-feedback");
    assert!(feedback.contains("CallerSupplied"), "{feedback}");
    assert!(feedback.contains("Not qualified"), "{feedback}");
    assert!(feedback.contains("Declared"), "{feedback}");
    assert!(feedback.contains("2048 / 256 / 0"), "{feedback}");
    assert!(
        fixture.requests().is_empty(),
        "diagnostic replay must never restart audio"
    );
    cx.update(|window, cx| window.click("process-window-policy-0-0", cx));
    assert!(!row(cx, "process-window-feedback").contains("No violation in this window"));
    cx.update(|window, cx| window.click("process-window-evaluate", cx));
    wait_until(Duration::from_secs(3), || {
        poll(&view, cx);
        row(cx, "process-window-feedback").contains("Observed violation")
    });
    assert!(fixture.requests().is_empty());
    fixture.finish();
}

#[gpui_kit::test]
fn process_window_settings_missing_prefix_is_a_visible_refusal(cx: &mut TestAppContext) {
    let fixture = Fixture::new(Evidence::Current, false);
    let (_, cx) = fixture.shell(cx);
    cx.update(|window, cx| {
        window.click("process-window-open", cx);
        window.render_frame(cx);
        window.click("process-window-evaluate", cx);
    });
    assert!(row(cx, "process-window-feedback").contains("Stop a collected Process session"));
    assert!(fixture.requests().is_empty());
    fixture.finish();
}

struct Fixture {
    runtime: ServiceRuntime,
    log: Arc<Mutex<Vec<AppRequest>>>,
    control: Arc<Mutex<Control>>,
    initial: Arc<AppSnapshot>,
    node: NodeId,
}

impl Fixture {
    fn new(evidence: Evidence, initial_selection: bool) -> Self {
        let log = Arc::new(Mutex::new(Vec::new()));
        let control = Arc::new(Mutex::new(Control::default()));
        let runtime = ServiceRuntime::spawn_with_driver(MeasuredDriver {
            log: log.clone(),
            control: control.clone(),
            evidence,
            initial_selection,
            initialized: false,
            spec: SessionSpec {
                source: InputSelection::Process {
                    pid: 42,
                    creation_time_100ns: 123,
                },
                output_endpoint_id: "fixture-output".into(),
                limit: RunLimit::UntilStopped,
                gain: 1.0,
                pan: 0.0,
                max_block_frames: 256,
            },
            selected: None,
        })
        .unwrap();
        let initial = wait_for(Duration::from_secs(2), || {
            runtime
                .handle()
                .try_snapshot()
                .filter(|snapshot| snapshot.phase == SessionPhase::Running)
        });
        let node = initial
            .graph
            .nodes
            .iter()
            .find(|node| {
                node.source
                    == Some(InputSelection::Process {
                        pid: 42,
                        creation_time_100ns: 123,
                    })
            })
            .unwrap()
            .id;
        Self {
            runtime,
            log,
            control,
            initial,
            node,
        }
    }

    fn shell<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<StudioShell>, &'a mut VisualTestContext) {
        let state = cx.new(|_| UiState::new(self.runtime.handle(), self.initial.clone()));
        let (view, cx) = shell_with_state(cx, state);
        cx.update(|window, cx| {
            window.resize(size(px(1440.), px(1000.)));
            window.render_frame(cx);
            window.click("open-settings", cx);
            window.click("settings-transport", cx);
            window.render_frame(cx);
        });
        (view, cx)
    }

    fn expected(&self) -> MeasuredProcessRequest {
        MeasuredProcessRequest {
            node: self.node,
            session_generation: self.initial.generation,
            native_owner_generation: 7,
            graph_revision: self.initial.graph.candidate_revision,
            capacity_frames: 2048,
            target_frames: 256,
            jitter_frames: 0,
        }
    }

    fn requests(&self) -> Vec<AppRequest> {
        self.log.lock().unwrap().clone()
    }

    fn finish(self) {
        finish_service(self.runtime);
    }
}

fn select_target(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        for id in [
            "measured-process-node-0",
            "measured-capacity-2",
            "measured-target-3",
            "measured-jitter-0",
        ] {
            assert!(
                window.find(id).visible(),
                "{id} must be reachable without scrolling"
            );
            window.click(id, cx);
        }
    });
}

fn poll(view: &Entity<StudioShell>, cx: &mut VisualTestContext) {
    view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| state.poll());
    });
    cx.update(|window, cx| window.render_frame(cx));
    cx.run_until_parked();
}

fn wait_snapshot(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    predicate: impl Fn(&AppSnapshot) -> bool,
) {
    wait_until(Duration::from_secs(3), || {
        poll(view, cx);
        view.update(cx, |shell, cx| predicate(&shell.state.read(cx).snapshot))
    });
}

fn assert_draft(
    view: &Entity<StudioShell>,
    cx: &mut VisualTestContext,
    expected: MeasuredProcessRequest,
) {
    view.update(cx, |shell, _| {
        let draft = &shell.settings.measured.draft;
        assert_eq!(draft.node, Some(expected.node));
        assert_eq!(draft.capacity_frames, expected.capacity_frames);
        assert_eq!(draft.target_frames, expected.target_frames);
        assert_eq!(draft.jitter_frames, expected.jitter_frames);
    });
}

fn row(cx: &mut VisualTestContext, id: &'static str) -> String {
    cx.update(|window, _| {
        let element = window.find(id);
        assert!(element.visible(), "{id} must be visible");
        element
            .label()
            .expect("visible diagnostic aria label")
            .to_owned()
    })
}

#[gpui_kit::test]
fn measured_review_absent_active_configuration_is_not_reported_as_compatible(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, false);
    let (view, cx) = fixture.shell(cx);
    assert!(row(cx, "measured-process-active").contains("Compatible"));
    fixture.control.lock().unwrap().clear_active = true;
    wait_snapshot(&view, cx, |snapshot| {
        snapshot.active_transport_latency.is_none()
    });

    let active = row(cx, "measured-process-active");
    assert!(
        active.contains("No active audio configuration"),
        "an absent active configuration is unknown, not a Compatible request: {active}"
    );
    assert!(!active.contains("Compatible"), "{active}");
    assert!(row(cx, "measured-process-requested").contains("Compatible"));
    assert!(fixture.requests().is_empty());
    fixture.finish();
}

#[gpui_kit::test]
fn measured_review_deadline_wakes_without_poll_render_or_new_confirmation(cx: &mut TestAppContext) {
    let fixture = Fixture::new(Evidence::Current, false);
    let expected = fixture.expected();
    let (view, cx) = fixture.shell(cx);
    select_target(cx);
    cx.update(|window, cx| window.click("measured-process-apply", cx));
    wait_snapshot(&view, cx, |snapshot| {
        snapshot
            .measured_process
            .as_ref()
            .is_some_and(|summary| summary.selection_id == 17)
    });
    view.update(cx, |shell, cx| {
        assert!(shell.settings.measured.busy());
        assert!(
            !shell.state.read(cx).pending_request,
            "wait for real service consumption before advancing UI time"
        );
    });
    assert_eq!(
        fixture.requests(),
        vec![AppRequest::ApplyMeasuredProcessBridge(expected)]
    );

    // Do not change the fixture, poll UiState, render a frame, or sleep through
    // std::Instant here: only the real UI's scheduled deadline may release it.
    cx.background_executor
        .advance_clock(Duration::from_secs(31));
    cx.run_until_parked();
    assert_eq!(
        fixture.requests(),
        vec![AppRequest::ApplyMeasuredProcessBridge(expected)]
    );
    view.update(cx, |shell, cx| {
        assert!(
            !shell.settings.measured.busy(),
            "the deadline must wake without another snapshot or user action"
        );
        let error = shell
            .settings
            .measured
            .error
            .as_deref()
            .expect("deadline refusal must remain visible");
        assert!(error.contains("30 seconds"), "{error}");
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(snapshot.phase, SessionPhase::Running);
        assert_eq!(snapshot.generation, expected.session_generation);
        assert!(snapshot.active_measured_process.is_none());
    });
    assert_draft(&view, cx, expected);
    assert!(row(cx, "measured-process-feedback").contains("30 seconds"));
    fixture.finish();
}

#[gpui_kit::test]
fn measured_click_enqueues_exact_request_once_and_waits_beyond_mailbox_consumption(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, false);
    let expected = fixture.expected();
    let (view, cx) = fixture.shell(cx);
    select_target(cx);
    assert_draft(&view, cx, expected);
    assert!(fixture.requests().is_empty(), "choices are not an apply");
    cx.update(|window, cx| {
        window.click("measured-process-apply", cx);
        window.click("measured-process-apply", cx);
    });
    view.update(cx, |shell, _| assert!(shell.settings.measured.busy()));
    wait_snapshot(&view, cx, |snapshot| snapshot.measured_process.is_some());
    assert_eq!(
        fixture.requests(),
        vec![AppRequest::ApplyMeasuredProcessBridge(expected)]
    );
    view.update(cx, |shell, cx| {
        assert!(
            !shell.state.read(cx).pending_request,
            "mailbox receipt is published"
        );
        assert!(
            shell.settings.measured.busy(),
            "receipt is not owner activation"
        );
        assert!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_measured_process
                .is_none()
        );
    });
    let desired = fixture
        .runtime
        .handle()
        .try_snapshot()
        .unwrap()
        .measured_process
        .clone()
        .unwrap();
    assert_eq!(desired.selection_id, 17);
    assert_eq!(desired.request, expected);
    assert!(
        row(cx, "measured-process-requested")
            .contains(&super::process_input_summary(None, Some(&desired)))
    );
    assert!(!row(cx, "measured-process-active").contains("selection 17"));
    cx.update(|window, cx| window.click("measured-process-apply", cx));
    poll(&view, cx);
    assert_eq!(
        fixture.requests(),
        vec![AppRequest::ApplyMeasuredProcessBridge(expected)]
    );

    fixture.control.lock().unwrap().confirmation = Some(Confirmation::Exact);
    wait_until(Duration::from_secs(3), || {
        poll(&view, cx);
        view.update(cx, |shell, _| !shell.settings.measured.busy())
    });
    view.update(cx, |shell, cx| {
        let snapshot = &shell.state.read(cx).snapshot;
        assert_ne!(snapshot.generation, expected.session_generation);
        assert_eq!(snapshot.phase, SessionPhase::Running);
        assert_eq!(snapshot.measured_process, Some(desired.clone()));
        assert_eq!(snapshot.active_measured_process, Some(desired.clone()));
        let native = snapshot.native_observations.as_ref().unwrap();
        assert_eq!(native.generation, snapshot.generation);
        assert_eq!(
            native.observation.owner_generation,
            expected.native_owner_generation
        );
    });
    assert!(
        row(cx, "measured-process-active")
            .contains(&super::process_input_summary(None, Some(&desired)))
    );
    assert_draft(&view, cx, expected);
    assert_eq!(
        fixture.requests(),
        vec![AppRequest::ApplyMeasuredProcessBridge(expected)]
    );
    fixture.finish();
}

#[gpui_kit::test]
fn measured_refusal_is_visible_and_preserves_editable_draft_and_previous_active_selection(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, true);
    fixture.control.lock().unwrap().reject = true;
    let expected = fixture.expected();
    let old = fixture.initial.active_measured_process.clone().unwrap();
    let (view, cx) = fixture.shell(cx);
    select_target(cx);
    cx.update(|window, cx| window.click("measured-process-apply", cx));
    wait_snapshot(&view, cx, |snapshot| {
        snapshot.graph.last_error.as_deref() == Some(REFUSAL)
    });
    assert!(row(cx, "measured-process-feedback").contains(REFUSAL));
    assert!(
        row(cx, "measured-process-active")
            .contains(&super::process_input_summary(None, Some(&old)))
    );
    assert_draft(&view, cx, expected);
    view.update(cx, |shell, cx| {
        assert!(!shell.settings.measured.busy());
        assert_eq!(
            shell.state.read(cx).snapshot.active_measured_process,
            Some(old)
        );
    });
    cx.update(|window, cx| window.click("measured-jitter-2", cx));
    view.update(cx, |shell, _| {
        assert_eq!(shell.settings.measured.draft.jitter_frames, 64)
    });
    assert_eq!(
        fixture.requests(),
        vec![AppRequest::ApplyMeasuredProcessBridge(expected)]
    );
    fixture.finish();
}

#[gpui_kit::test]
fn measured_missing_or_stale_evidence_blocks_click_without_changing_running_owner(
    cx: &mut TestAppContext,
) {
    for evidence in [
        Evidence::MissingNative,
        Evidence::MissingProducer,
        Evidence::StaleSession,
        Evidence::StaleOwner,
        Evidence::ReplacedProcess,
    ] {
        let fixture = Fixture::new(evidence, false);
        let (view, cx) = fixture.shell(cx);
        select_target(cx);
        cx.update(|window, cx| window.click("measured-process-apply", cx));
        poll(&view, cx);
        assert!(
            fixture.requests().is_empty(),
            "{evidence:?} must not enqueue"
        );
        let feedback = row(cx, "measured-process-feedback");
        assert!(
            !feedback.is_empty()
                && !feedback.contains("Ready to validate")
                && !feedback.contains("budget is active"),
            "{evidence:?} requires a visible reason"
        );
        view.update(cx, |shell, cx| {
            assert!(!shell.settings.measured.busy(), "{evidence:?}");
            let snapshot = &shell.state.read(cx).snapshot;
            assert_eq!(snapshot.phase, SessionPhase::Running);
            assert_eq!(snapshot.generation, fixture.initial.generation);
            assert!(snapshot.measured_process.is_none());
            assert!(snapshot.active_measured_process.is_none());
        });
        assert_draft(&view, cx, fixture.expected());
        fixture.finish();
    }
}

#[gpui_kit::test]
fn measured_unrelated_confirmation_cannot_satisfy_captured_request_or_generation(
    cx: &mut TestAppContext,
) {
    for confirmation in [
        Confirmation::SameSession,
        Confirmation::WrongSelection,
        Confirmation::WrongRequest,
    ] {
        let fixture = Fixture::new(Evidence::Current, true);
        let expected = fixture.expected();
        let (view, cx) = fixture.shell(cx);
        select_target(cx);
        cx.update(|window, cx| window.click("measured-process-apply", cx));
        wait_snapshot(&view, cx, |snapshot| {
            snapshot
                .measured_process
                .as_ref()
                .is_some_and(|s| s.selection_id == 17)
        });
        fixture.control.lock().unwrap().confirmation = Some(confirmation);
        wait_snapshot(&view, cx, |snapshot| {
            let Some(active) = snapshot.active_measured_process.as_ref() else {
                return false;
            };
            match confirmation {
                Confirmation::SameSession => {
                    active.selection_id == 17 && snapshot.generation == expected.session_generation
                }
                Confirmation::WrongSelection => active.selection_id == 18,
                Confirmation::WrongRequest => {
                    active.selection_id == 17
                        && active.request.target_frames == expected.target_frames + 1
                }
                Confirmation::Exact => unreachable!(),
            }
        });
        assert_draft(&view, cx, expected);
        if matches!(confirmation, Confirmation::SameSession) {
            view.update(cx, |shell, _| {
                assert!(
                    shell.settings.measured.busy(),
                    "the originating owner is not a replacement"
                )
            });
            cx.update(|window, cx| window.click("measured-process-apply", cx));
            poll(&view, cx);
            fixture.control.lock().unwrap().confirmation = Some(Confirmation::Exact);
            wait_until(Duration::from_secs(3), || {
                poll(&view, cx);
                view.update(cx, |shell, _| !shell.settings.measured.busy())
            });
            view.update(cx, |shell, cx| {
                let snapshot = &shell.state.read(cx).snapshot;
                let desired = snapshot.measured_process.as_ref().unwrap();
                assert_eq!(desired.request, expected);
                assert_eq!(desired.selection_id, 17);
                assert_eq!(snapshot.active_measured_process.as_ref(), Some(desired));
                assert_ne!(snapshot.generation, expected.session_generation);
            });
        } else {
            let feedback = row(cx, "measured-process-feedback");
            assert!(
                !feedback.is_empty() && !feedback.contains("budget is active"),
                "{feedback}"
            );
            view.update(cx, |shell, _| {
                assert!(
                    !shell.settings.measured.busy(),
                    "definitive mismatch is a refusal, not success"
                )
            });
        }
        assert_eq!(
            fixture.requests(),
            vec![AppRequest::ApplyMeasuredProcessBridge(expected)]
        );
        fixture.finish();
    }
}
