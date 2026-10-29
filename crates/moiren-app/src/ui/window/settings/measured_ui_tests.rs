//! Real settings clicks against a held, software-only service fixture. These
//! tests prove UI dispatch/confirmation, not native replacement or audio quality.
use crate::service::{
    AppRequest, AppSnapshot, InputSelection, JobPool, MeasuredProcessRequest,
    NativeSessionObservations, ProcessWindowPolicy, RoundBudget, RunLimit, ServiceContext,
    ServiceDriver, ServiceRuntime, SessionPhase, SessionSpec,
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
    measured_prefix: bool,
    finish_live_window: bool,
    wrong_live_policy: bool,
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
    pending_window: Option<(MeasuredProcessRequest, ProcessWindowPolicy)>,
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
        context.process_trace_status = Some(crate::service::ProcessTraceCollectionStatus {
            origin: Some(crate::service::ProcessTraceSessionOrigin {
                generation,
                document: 9,
                node,
                process: observation(node).process,
                native_owner_generation: 7,
                host_revision: graph.candidate_revision,
                graph_revision: graph.candidate_revision,
                audio_settings: context.audio_settings.clone(),
                transport_latency: context.transport_latency,
                latency_profile: context.latency_profile,
            }),
            refusal: None,
        });
    }

    fn confirm(&mut self, context: &mut ServiceContext, confirmation: Confirmation) {
        let mut active = self.selected.as_ref().unwrap().summary().clone();
        if !matches!(confirmation, Confirmation::SameSession)
            && (context.core.generation() == active.request.session_generation
                || context.core.owner_generation().is_none())
        {
            // Only simulate the service's confirmed projection. Existing managed
            // tests, not this fixture, verify the actual replacement factories.
            if let Some(old) = context.core.owner_generation() {
                assert!(context.core.owner_reaped(old));
            }
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
            if context.native_observations.is_none() {
                context.native_observations = Some(NativeSessionObservations {
                    generation,
                    observation: NativeObservationSnapshot {
                        owner_generation: 7,
                        running: true,
                        streams: vec![],
                        bridges: vec![],
                        boundary_probes: vec![],
                        render_padding: vec![],
                        process_delivery: vec![observation(active_node(context))],
                    },
                });
            }
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
        if let AppRequest::ApplyWindowedProcessBridge { request, policy } = request {
            if self.control.lock().unwrap().reject {
                context.graph.record_error(
                    "fixture live Process window refused: measurement history overwritten",
                );
            } else {
                // Holding this completion exercises the real mailbox/UI seam;
                // it does not issue a native token or replace an audio owner.
                self.pending_window = Some((request, policy));
            }
            return;
        }
        if let AppRequest::ApplyEvaluatedProcessBridge(candidate) = &request {
            let profile = candidate.profile(17).unwrap();
            context.measured_process = Some(profile.clone());
            self.selected = Some(profile);
            return;
        }
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
        let (finish_window, wrong_policy) = {
            let mut control = self.control.lock().unwrap();
            (
                std::mem::take(&mut control.finish_live_window),
                control.wrong_live_policy,
            )
        };
        if finish_window && let Some((request, mut policy)) = self.pending_window.take() {
            if wrong_policy {
                policy.min_output_reads += 1;
            }
            let profile = software_live_window_profile(context, request, policy);
            context.measured_process = Some(profile.clone());
            self.selected = Some(profile);
            context
                .graph
                .preparing(context.core.generation(), request.graph_revision);
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
            let mut evidence = crate::service::process_trace::tests::evidence(node);
            evidence.origin.generation = context.core.generation();
            evidence.origin.graph_revision = context.graph.snapshot().candidate_revision;
            evidence.origin.audio_settings = context.audio_settings.clone();
            evidence.origin.transport_latency = context.transport_latency;
            evidence.origin.latency_profile = context.latency_profile;
            if self.control.lock().unwrap().measured_prefix {
                let observation = observation(node);
                evidence.measurement = observation
                    .measured(&observation.process, 7)
                    .map_err(|error| error.to_string());
            }
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
        self.selected = None;
        self.pending_window = None;
        if let Some(generation) = context.core.owner_generation() {
            context.core.owner_reaped(generation);
        }
    }

    fn is_drained(&self) -> bool {
        // A held stopped Apply still has a confirmation to publish. Do not let
        // the service park indefinitely before the fixture releases it.
        self.selected.is_none() && self.pending_window.is_none()
    }
}

fn active_node(context: &ServiceContext) -> NodeId {
    context
        .graph
        .snapshot()
        .nodes
        .iter()
        .find(|node| matches!(node.source, Some(InputSelection::Process { .. })))
        .unwrap()
        .id
}

fn software_live_window_profile(
    context: &ServiceContext,
    request: MeasuredProcessRequest,
    policy: ProcessWindowPolicy,
) -> MeasuredProcessProfile {
    // Actual replay and profile checks, through the existing cfg(test) seam.
    // This fixture's archive remains CallerSupplied; it proves UI behavior,
    // not issuance of production native proof or successful audio replacement.
    let observed = observation(request.node);
    let measurement = observed
        .measured(&observed.process, request.native_owner_generation)
        .unwrap();
    let options = measurement
        .admit_for_consumer(
            &observed.process,
            request.native_owner_generation,
            moiren_windows_audio::latency::transport::TransportInputPolicy::Target {
                capacity_frames: request.capacity_frames,
                target_frames: request.target_frames,
                jitter_frames: request.jitter_frames,
            },
            observed.consumer_sample_rate,
            observed.prepared_consumer_demand_frames,
        )
        .unwrap();
    let evidence = crate::service::process_trace::tests::evidence(request.node);
    let report = evidence
        .artifact()
        .replay_live_window(
            &observed.process,
            request.native_owner_generation,
            options,
            policy.native(),
        )
        .unwrap();
    assert_eq!(
        report.trace_origin,
        moiren_windows_audio::process_loopback::ProcessTraceOrigin::CallerSupplied
    );
    crate::service::measured_process::live::LiveProcessEvaluation::software(
        measurement,
        report,
        true,
    )
    .profile(
        17,
        request,
        policy,
        context
            .process_trace_status
            .as_ref()
            .unwrap()
            .origin
            .as_ref()
            .unwrap()
            .clone(),
    )
    .unwrap()
}

#[gpui_kit::test]
fn retained_apply_settings_click_requires_separate_admission_then_confirms_new_owner(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, false);
    {
        let mut control = fixture.control.lock().unwrap();
        control.retain_prefix = true;
        control.measured_prefix = true;
    }
    let (view, cx) = fixture.shell(cx);
    wait_snapshot(&view, cx, |snapshot| snapshot.last_cleanup.is_some());
    select_target(cx);
    cx.update(|window, cx| {
        window.click("process-window-open", cx);
        window.render_frame(cx);
        window.click("process-window-evaluate", cx);
    });
    wait_until(Duration::from_secs(3), || {
        poll(&view, cx);
        row(cx, "process-window-admission").contains("retained native")
    });
    assert!(
        fixture.requests().is_empty(),
        "evaluation never starts audio"
    );
    // Only this private software fixture injects a token. Production issuance
    // above correctly refuses CallerSupplied despite valid measured observations.
    view.update(cx, |shell, cx| {
        shell.inject_software_window_candidate(cx);
    });
    reveal(cx, "process-window-apply");
    cx.update(|window, cx| {
        assert!(window.find("process-window-apply").visible());
        assert!(
            window
                .find("settings-scroll")
                .bounds()
                .contains(&window.find("process-window-apply").bounds().center())
        );
        window.click("process-window-apply", cx);
        window.click("process-window-apply", cx);
    });
    view.update(cx, |shell, _| {
        assert!(
            shell.settings.measured.busy(),
            "{:?}",
            shell.settings.measured.error
        );
    });
    wait_snapshot(&view, cx, |snapshot| snapshot.measured_process.is_some());
    view.update(cx, |shell, _| {
        assert!(
            shell.settings.measured.busy(),
            "{:?}",
            shell.settings.measured.error
        )
    });
    fixture.control.lock().unwrap().confirmation = Some(Confirmation::Exact);
    wait_snapshot(&view, cx, |snapshot| {
        snapshot.phase == SessionPhase::Running
    });
    wait_until(Duration::from_secs(3), || {
        poll(&view, cx);
        view.update(cx, |shell, _| !shell.settings.measured.busy())
    });
    assert!(row(cx, "process-window-activation").contains("budget is active"));
    assert_eq!(fixture.requests().len(), 1);
    assert!(matches!(
        fixture.requests()[0],
        AppRequest::ApplyEvaluatedProcessBridge(_)
    ));
    fixture.finish();
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
    reveal(cx, "process-window-policy-0-0");
    cx.update(|window, cx| window.click("process-window-policy-0-0", cx));
    assert!(!row(cx, "process-window-feedback").contains("No violation in this window"));
    reveal(cx, "process-window-evaluate");
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
            pending_window: None,
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
    reveal(cx, id);
    cx.update(|window, _| {
        let element = window.find(id);
        assert!(element.visible(), "{id} must be visible");
        element
            .label()
            .expect("visible diagnostic aria label")
            .to_owned()
    })
}

fn reveal(cx: &mut VisualTestContext, id: &'static str) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let target = window.find(id).bounds().center();
        let viewport = window.find("settings-scroll").bounds();
        // Partially clipped elements are marked visible, but pointer dispatch
        // uses their centers. Exercise the actual scrollable Settings page.
        if !viewport.contains(&target) {
            window.scroll(
                "settings-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    px(0.),
                    viewport.center().y - target.y,
                )),
                cx,
            );
        }
    });
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

fn live_window_policy() -> ProcessWindowPolicy {
    ProcessWindowPolicy {
        max_initial_unserved_frames: 4096,
        max_initial_prime_discarded_frames: 4096,
        min_output_frames: 96,
        min_output_reads: 2,
        min_output_span_100ns: 10_000,
    }
}

fn open_live_window(cx: &mut VisualTestContext) {
    reveal(cx, "measured-live-window-open");
    cx.update(|window, cx| {
        assert!(
            window.find("measured-live-window-open").visible(),
            "live observed-window mode requires an explicit opt-in action"
        );
        window.click("measured-live-window-open", cx);
        window.render_frame(cx);
    });
}

fn choose_live_window_policy(cx: &mut VisualTestContext) {
    for id in [
        "measured-live-window-policy-0-2",
        "measured-live-window-policy-1-2",
        "measured-live-window-policy-2-0",
        "measured-live-window-policy-3-0",
        "measured-live-window-policy-4-0",
    ] {
        reveal(cx, id);
        cx.update(|window, cx| window.click(id, cx));
    }
}

#[gpui_kit::test]
fn desktop_live_window_requires_opt_in_and_each_policy_field_before_dispatch(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, false);
    let (_, cx) = fixture.shell(cx);
    select_target(cx);
    assert!(fixture.requests().is_empty());
    open_live_window(cx);
    let feedback = row(cx, "measured-live-window-feedback");
    assert!(feedback.contains("all five"), "{feedback}");
    reveal(cx, "measured-live-window-apply");
    cx.update(|window, cx| window.click("measured-live-window-apply", cx));
    for id in [
        "measured-live-window-policy-0-2",
        "measured-live-window-policy-1-2",
        "measured-live-window-policy-2-0",
        "measured-live-window-policy-3-0",
    ] {
        reveal(cx, id);
        cx.update(|window, cx| window.click(id, cx));
    }
    reveal(cx, "measured-live-window-apply");
    cx.update(|window, cx| window.click("measured-live-window-apply", cx));
    assert!(
        fixture.requests().is_empty(),
        "the fifth field must not be guessed from diagnostic defaults"
    );
    assert!(row(cx, "measured-live-window-feedback").contains("all five"));
    fixture.finish();
}

#[gpui_kit::test]
fn desktop_live_window_click_dispatches_exact_policy_and_waits_for_async_selection_and_owner(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, true);
    let expected = fixture.expected();
    let (view, cx) = fixture.shell(cx);
    select_target(cx);
    open_live_window(cx);
    choose_live_window_policy(cx);
    reveal(cx, "measured-live-window-apply");
    cx.update(|window, cx| {
        window.click("measured-live-window-apply", cx);
        window.click("measured-live-window-apply", cx);
    });
    wait_snapshot(&view, cx, |snapshot| snapshot.consumed_request_id.0 > 0);
    let expected_request = AppRequest::ApplyWindowedProcessBridge {
        request: expected,
        policy: live_window_policy(),
    };
    assert_eq!(fixture.requests(), vec![expected_request.clone()]);
    view.update(cx, |shell, cx| {
        assert!(!shell.state.read(cx).pending_request);
        assert!(
            shell.settings.measured.busy(),
            "asynchronous replay has not yet selected a new profile"
        );
        assert_eq!(
            shell
                .state
                .read(cx)
                .snapshot
                .active_measured_process
                .as_ref()
                .unwrap()
                .selection_id,
            16
        );
    });
    reveal(cx, "measured-live-window-apply");
    cx.update(|window, cx| window.click("measured-live-window-apply", cx));
    fixture.control.lock().unwrap().finish_live_window = true;
    wait_snapshot(&view, cx, |snapshot| {
        snapshot
            .measured_process
            .as_ref()
            .is_some_and(|s| s.selection_id == 17)
    });
    view.update(cx, |shell, _| {
        assert!(shell.settings.measured.busy(), "requested is not active")
    });
    let requested = row(cx, "measured-live-window-requested");
    assert!(
        requested.contains("4096 / 4096 / 96 / 2 / 10000"),
        "{requested}"
    );
    assert!(!row(cx, "measured-live-window-active").contains("4096 / 4096 / 96 / 2 / 10000"));
    fixture.control.lock().unwrap().confirmation = Some(Confirmation::Exact);
    wait_until(Duration::from_secs(3), || {
        poll(&view, cx);
        view.update(cx, |shell, _| !shell.settings.measured.busy())
    });
    view.update(cx, |shell, cx| {
        assert!(
            shell.settings.measured.error.is_none(),
            "{:?}",
            shell.settings.measured.error
        );
        let snapshot = &shell.state.read(cx).snapshot;
        assert_ne!(snapshot.generation, expected.session_generation);
        assert_eq!(snapshot.measured_process, snapshot.active_measured_process);
        assert_eq!(
            snapshot
                .active_measured_process
                .as_ref()
                .unwrap()
                .observed_window
                .as_ref()
                .unwrap()
                .policy,
            live_window_policy()
        );
    });
    let active = row(cx, "measured-live-window-active");
    assert!(active.contains("4096 / 4096 / 96 / 2 / 10000"), "{active}");
    let feedback = row(cx, "measured-live-window-feedback");
    assert!(feedback.contains("unqualified"), "{feedback}");
    assert_eq!(
        fixture.requests(),
        vec![expected_request],
        "no ApplyGraph, Stop, fallback or second apply"
    );
    fixture.finish();
}

#[gpui_kit::test]
fn desktop_live_window_refresh_catalog_receipt_preserves_pending_until_exact_new_owner(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, true);
    let expected = fixture.expected();
    let original = fixture.initial.measured_process.clone();
    let (view, cx) = fixture.shell(cx);
    select_target(cx);
    open_live_window(cx);
    choose_live_window_policy(cx);
    reveal(cx, "measured-live-window-apply");
    cx.update(|window, cx| window.click("measured-live-window-apply", cx));
    wait_snapshot(&view, cx, |snapshot| snapshot.consumed_request_id.0 > 0);
    let apply_id = view.update(cx, |shell, cx| {
        assert!(shell.settings.measured.busy());
        shell.state.read(cx).snapshot.consumed_request_id
    });

    // Use the real tracked mailbox, not a fabricated receipt. The service
    // treats catalog refresh as non-mutating and keeps the evaluation alive.
    let refresh_id = view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.try_dispatch(AppRequest::RefreshCatalog).unwrap()
        })
    });
    assert!(refresh_id > apply_id);
    wait_snapshot(&view, cx, |snapshot| {
        snapshot.consumed_request_id >= refresh_id
    });
    view.update(cx, |shell, cx| {
        assert!(
            shell.settings.measured.busy(),
            "a newer harmless catalog receipt does not supersede live evaluation"
        );
        assert!(shell.settings.measured.error.is_none());
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(snapshot.generation, expected.session_generation);
        assert_eq!(snapshot.measured_process, original);
        assert_eq!(snapshot.active_measured_process, original);
    });

    fixture.control.lock().unwrap().finish_live_window = true;
    wait_snapshot(&view, cx, |snapshot| {
        snapshot
            .measured_process
            .as_ref()
            .is_some_and(|selection| selection.selection_id == 17)
    });
    let second_refresh_id = view.update(cx, |shell, cx| {
        shell.state.update(cx, |state, _| {
            state.try_dispatch(AppRequest::RefreshCatalog).unwrap()
        })
    });
    wait_snapshot(&view, cx, |snapshot| {
        snapshot.consumed_request_id >= second_refresh_id
    });
    view.update(cx, |shell, cx| {
        assert!(
            shell.settings.measured.busy(),
            "catalog receipt is not replacement-owner confirmation"
        );
        assert!(shell.settings.measured.error.is_none());
        let snapshot = &shell.state.read(cx).snapshot;
        assert_eq!(snapshot.generation, expected.session_generation);
        assert_eq!(snapshot.active_measured_process, original);
    });

    fixture.control.lock().unwrap().confirmation = Some(Confirmation::Exact);
    wait_until(Duration::from_secs(3), || {
        poll(&view, cx);
        view.update(cx, |shell, _| !shell.settings.measured.busy())
    });
    view.update(cx, |shell, cx| {
        assert!(
            shell.settings.measured.error.is_none(),
            "{:?}",
            shell.settings.measured.error
        );
        assert!(shell.settings.measured.message.is_some());
        let snapshot = &shell.state.read(cx).snapshot;
        assert_ne!(snapshot.generation, expected.session_generation);
        let active = snapshot.active_measured_process.as_ref().unwrap();
        assert_eq!(active.request, expected);
        assert_eq!(
            active.observed_window.as_ref().unwrap().policy,
            live_window_policy()
        );
        assert_eq!(snapshot.measured_process.as_ref(), Some(active));
    });
    assert_eq!(
        fixture.requests(),
        vec![
            AppRequest::ApplyWindowedProcessBridge {
                request: expected,
                policy: live_window_policy(),
            },
            AppRequest::RefreshCatalog,
            AppRequest::RefreshCatalog,
        ],
        "refresh must not cause fallback, a second apply, or a stop"
    );
    fixture.finish();
}

#[gpui_kit::test]
fn desktop_live_window_refusal_retains_draft_previous_active_and_never_falls_back(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(Evidence::Current, true);
    fixture.control.lock().unwrap().reject = true;
    let expected = fixture.expected();
    let old = fixture.initial.active_measured_process.clone();
    let (view, cx) = fixture.shell(cx);
    select_target(cx);
    open_live_window(cx);
    choose_live_window_policy(cx);
    reveal(cx, "measured-live-window-apply");
    cx.update(|window, cx| window.click("measured-live-window-apply", cx));
    wait_snapshot(&view, cx, |snapshot| {
        snapshot
            .graph
            .last_error
            .as_ref()
            .is_some_and(|e| e.contains("overwritten"))
    });
    assert!(row(cx, "measured-live-window-feedback").contains("overwritten"));
    view.update(cx, |shell, cx| {
        assert!(!shell.settings.measured.busy());
        assert_eq!(shell.state.read(cx).snapshot.active_measured_process, old);
    });
    assert_draft(&view, cx, expected);
    assert_eq!(
        fixture.requests(),
        vec![AppRequest::ApplyWindowedProcessBridge {
            request: expected,
            policy: live_window_policy()
        }]
    );
    fixture.finish();
}

#[gpui_kit::test]
fn desktop_live_window_different_policy_cannot_confirm_same_request(cx: &mut TestAppContext) {
    let fixture = Fixture::new(Evidence::Current, false);
    fixture.control.lock().unwrap().wrong_live_policy = true;
    let (view, cx) = fixture.shell(cx);
    select_target(cx);
    open_live_window(cx);
    choose_live_window_policy(cx);
    reveal(cx, "measured-live-window-apply");
    cx.update(|window, cx| window.click("measured-live-window-apply", cx));
    wait_snapshot(&view, cx, |snapshot| snapshot.consumed_request_id.0 > 0);
    fixture.control.lock().unwrap().finish_live_window = true;
    wait_snapshot(&view, cx, |snapshot| snapshot.measured_process.is_some());
    view.update(cx, |shell, _| {
        assert!(!shell.settings.measured.busy());
        assert!(
            shell.settings.measured.error.is_some(),
            "a different policy is not successful confirmation"
        );
        assert!(shell.settings.measured.message.is_none());
    });
    assert!(!row(cx, "measured-live-window-feedback").contains("budget is active"));
    assert_eq!(fixture.requests().len(), 1);
    fixture.finish();
}

#[gpui_kit::test]
fn desktop_live_window_missing_or_stale_original_observations_do_not_dispatch(
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
        open_live_window(cx);
        choose_live_window_policy(cx);
        reveal(cx, "measured-live-window-apply");
        cx.update(|window, cx| window.click("measured-live-window-apply", cx));
        poll(&view, cx);
        assert!(fixture.requests().is_empty(), "{evidence:?}");
        assert!(!row(cx, "measured-live-window-feedback").is_empty());
        view.update(cx, |shell, _| assert!(!shell.settings.measured.busy()));
        fixture.finish();
    }
}
