use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

use super::{
    parameters::Router,
    preparation::{self, Basis, Completion, PrepareJob, Ready},
    retirement::{self, Cleanup, Reclaimed},
    *,
};
use crate::{
    compiler::CompiledBindings,
    control::ControlPort,
    runtime::{PlanControlPort, PlanSnapshot, RetireOutcome},
    sample::ProcessingSample,
};

mod requests;
mod snapshot;
mod updates;

struct Active<S: ProcessingSample> {
    owner: Box<dyn BackendOwner<S>>,
    plans: PlanControlPort<S>,
    control: ControlPort,
    bindings: CompiledBindings,
    snapshot: PlanSnapshot,
    project: EngineProject,
    graph: u64,
    signals: moiren_signal::HostControl,
    confirmed: bool,
}
struct Candidate {
    control: ControlPort,
    bindings: CompiledBindings,
    snapshot: PlanSnapshot,
    project: EngineProject,
    graph: u64,
}
struct Supervisor<S: ProcessingSample> {
    config: HostConfig,
    shared: Arc<Shared>,
    backend: Arc<dyn EngineBackend<S>>,
    commands: Receiver<Command>,
    preparations: SyncSender<PrepareJob>,
    prepared: Receiver<Completion<S>>,
    cleanup: SyncSender<Cleanup<S>>,
    reclaimed: Receiver<Reclaimed>,
    prepare_worker: thread::JoinHandle<()>,
    cleanup_worker: thread::JoinHandle<()>,
    project: EngineProject,
    desired: u64,
    plan_revision: u64,
    owner_generation: u64,
    resources: u64,
    epoch: u64,
    resource_nodes: BTreeMap<moiren_core::graph::NodeId, (u64, bool)>,
    active: Option<Active<S>>,
    pending: Option<Candidate>,
    preparing: Option<(Basis, CancellationToken)>,
    cleanup_inflight: usize,
    blocked: Option<(Basis, Diagnostic)>,
    fault: Option<Diagnostic>,
    cleanup_error: Option<BackendError>,
    edits: Vec<(u64, Arc<ReceiptCell<EditOutcome>>)>,
    router: Router,
}

pub(crate) fn spawn<S: ProcessingSample>(
    config: HostConfig,
    shared: Arc<Shared>,
    commands: Receiver<Command>,
    backend: Arc<dyn EngineBackend<S>>,
) -> Result<thread::JoinHandle<()>, HostError> {
    let (preparations, jobs) = mpsc::sync_channel(1);
    let (completed, prepared) = mpsc::sync_channel(1);
    let (cleanup, retirements) = mpsc::sync_channel(config.cleanup_capacity);
    let (returned, reclaimed) = mpsc::sync_channel(config.cleanup_capacity);
    let prepare_worker = preparation::spawn(
        config.clone(),
        shared.clone(),
        backend.clone(),
        jobs,
        completed,
    )
    .map_err(|error| HostError::Worker(error.to_string()))?;
    let cleanup_worker = match retirement::spawn(shared.clone(), retirements, returned) {
        Ok(worker) => worker,
        Err(error) => {
            drop(preparations);
            let _ = prepare_worker.join();
            return Err(HostError::Worker(error.to_string()));
        }
    };
    let project = shared
        .snapshot
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .desired
        .as_ref()
        .clone();
    let supervisor = Supervisor {
        config,
        shared,
        backend,
        commands,
        preparations,
        prepared,
        cleanup,
        reclaimed,
        prepare_worker,
        cleanup_worker,
        project,
        desired: 1,
        plan_revision: 0,
        owner_generation: 0,
        resources: 0,
        epoch: 0,
        resource_nodes: BTreeMap::new(),
        active: None,
        pending: None,
        preparing: None,
        cleanup_inflight: 0,
        blocked: None,
        fault: None,
        cleanup_error: None,
        edits: Vec::new(),
        router: Router::default(),
    };
    thread::Builder::new()
        .name("moiren-host-control".into())
        .spawn(move || supervisor.run())
        .map_err(|error| HostError::Worker(error.to_string()))
}

impl<S: ProcessingSample> Supervisor<S> {
    fn closing(&self) -> bool {
        self.shared.closing.load(Ordering::Acquire)
    }
    fn basis(&self) -> Basis {
        Basis {
            graph: self.desired,
            owner: self.owner_generation,
            resources: self.resources,
            active_plan: self
                .active
                .as_ref()
                .map_or(0, |active| active.control.identity().0),
        }
    }
    fn unavailable(&self) -> bool {
        self.resource_nodes
            .iter()
            .any(|(node, (_, available))| self.project.nodes.contains_key(node) && !available)
    }
    fn run(mut self) {
        loop {
            self.reclaimed();
            self.commands();
            self.resources();
            self.retired();
            self.completions();
            self.owner_status();
            let closing = self.closing();
            if closing
                || self.fault.is_some()
                || self.unavailable()
                || !self.project.has_output()
                || self
                    .active
                    .as_ref()
                    .is_some_and(|active| !transport_compatible(&active.project, &self.project))
            {
                if let Some((_, cancel)) = &self.preparing {
                    cancel.cancel();
                }
                if let Some(active) = &mut self.active {
                    active.plans.cancel_pending();
                    active.owner.cancel();
                }
                self.stop_active();
            } else {
                self.schedule();
            }
            if let Some(active) = &mut self.active {
                self.router
                    .poll(&mut active.control, &mut self.project, closing);
                if active.confirmed && !closing {
                    let frame = active
                        .owner
                        .timeline()
                        .filter(|timeline| {
                            (timeline.revision, timeline.epoch) == active.control.identity()
                        })
                        .map(|timeline| timeline.frame);
                    if let Some(frame) = frame {
                        self.router.route(
                            &self.project,
                            &active.project,
                            &active.bindings,
                            &mut active.control,
                            frame,
                        );
                    }
                }
            }
            self.shared.signals.pump();
            if self.preparing.is_none() && self.pending.is_none() && self.cleanup_inflight == 0 {
                if let Some(active) = &self.active {
                    self.shared.signals.prune(&active.project);
                }
                if !closing && !self.project.has_output() && self.active.is_none() {
                    self.activate_edits(self.desired, None);
                }
            }
            self.publish_snapshot();
            if closing
                && self.active.is_none()
                && self.preparing.is_none()
                && self.pending.is_none()
                && self.cleanup_inflight == 0
            {
                break;
            }
            let guard = self
                .shared
                .wake_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let _guard = self
                .shared
                .wake
                .wait_timeout(guard, self.config.poll_interval)
                .unwrap_or_else(|poison| poison.into_inner());
        }
        self.router.close();
        for (_, receipt) in self.edits.drain(..) {
            receipt.set(EditOutcome::Closed);
        }
        while let Ok(command) = self.commands.try_recv() {
            close_command(command);
        }
        drop(self.preparations);
        let _ = self.prepare_worker.join();
        drop(self.cleanup);
        let _ = self.cleanup_worker.join();
        drop(self.backend);
        let mut snapshot = self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if snapshot.desired.as_ref() != &self.project {
            snapshot.desired = Arc::new(self.project);
        }
        snapshot.active = None;
        snapshot.state.host = HostState::Closed;
        snapshot.state.execution = ExecutionState::Dormant;
        snapshot.state.active_revision = None;
        snapshot.state.plan_revision = None;
        snapshot.state.timeline = None;
        snapshot.state.update = UpdateState::Idle;
    }
    fn reclaimed(&mut self) {
        while let Ok(result) = self.reclaimed.try_recv() {
            self.cleanup_inflight -= 1;
            let closing = self.closing();
            for reply in result.replies {
                self.router.reply(reply, &mut self.project, closing);
            }
            if let Some(error) = result.error {
                self.cleanup_error = Some(error.clone());
                if matches!(error, BackendError::WorkerPanicked) {
                    self.fault = Some(Diagnostic::Backend(error));
                }
            }
        }
    }
    fn cleanup(&mut self, job: Cleanup<S>) {
        self.cleanup.try_send(job).unwrap_or_else(|_| {
            unreachable!("cleanup completion capacity reserved before handoff")
        });
        self.cleanup_inflight += 1;
    }
    fn stop_active(&mut self) {
        if self.cleanup_inflight >= self.config.cleanup_capacity {
            return;
        }
        let Some(active) = self.active.take() else {
            return;
        };
        let mut controls = vec![active.control];
        if let Some(pending) = self.pending.take() {
            controls.push(pending.control);
        }
        self.cleanup(Cleanup::Owner {
            owner: active.owner,
            plans: active.plans,
            controls,
            signals: active.signals,
        });
    }
}
fn close_command(command: Command) {
    match command {
        Command::Edit { receipt, .. } => receipt.set(EditOutcome::Closed),
        Command::Parameter { receipt, .. } => receipt.cell.set(ParameterOutcome::Closed),
    }
}
fn transport_compatible(previous: &EngineProject, desired: &EngineProject) -> bool {
    if previous.processing.engine != desired.processing.engine {
        return false;
    }
    let io = |project: &EngineProject| {
        project
            .nodes
            .iter()
            .filter(|(_, config)| matches!(config, NodeConfig::Input(_) | NodeConfig::Output(_)))
            .map(|(&node, config)| {
                (
                    node,
                    config.clone(),
                    project.graph.get_node(node).unwrap().clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    io(previous) == io(desired)
}
fn same_structure(previous: &EngineProject, desired: &EngineProject) -> bool {
    previous.nodes == desired.nodes
        && previous.processing == desired.processing
        && previous.graph.nodes() == desired.graph.nodes()
        && previous.graph.edges().len() == desired.graph.edges().len()
        && previous
            .graph
            .edges()
            .iter()
            .zip(desired.graph.edges())
            .all(|(previous, desired)| {
                previous.id() == desired.id()
                    && previous.src_port() == desired.src_port()
                    && previous.dst_port() == desired.dst_port()
                    && previous.params().tap == desired.params().tap
            })
}
