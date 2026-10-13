use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender},
    },
    thread,
};

use moiren_core::{
    graph::{EdgeId, NodeId},
    protocol::{ParamValue, ParameterId, ProcessorId},
};

use super::{
    BackendError, BackendOwner, CancellationToken, Diagnostic, EngineBackend, EngineProject,
    HostConfig, NodeConfig, ParameterTarget, PrepareMode, PrepareRequest, SendParameter, Shared,
};
use crate::{
    compiler::{self, CompiledBindings, CompiledUpdate},
    runtime::{PlanControlPort, PlanSnapshot},
    sample::ProcessingSample,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Basis {
    pub graph: u64,
    pub owner: u64,
    pub resources: u64,
    pub active_plan: u64,
}
pub(crate) struct PrepareJob {
    pub basis: Basis,
    pub plan_revision: u64,
    pub epoch: u64,
    pub project: EngineProject,
    pub reuse: Vec<(NodeId, ProcessorId)>,
    pub edge_reuse: Vec<(EdgeId, ProcessorId)>,
    pub active: Option<PlanSnapshot>,
    pub signals: Option<moiren_signal::HostControl>,
    pub cancel: CancellationToken,
}
pub(crate) struct Initial<S: ProcessingSample> {
    pub owner: Box<dyn BackendOwner<S>>,
    pub plans: PlanControlPort<S>,
    pub control: crate::control::ControlPort,
    pub bindings: CompiledBindings,
    pub snapshot: PlanSnapshot,
    pub signals: moiren_signal::HostControl,
}
pub(crate) enum Ready<S: ProcessingSample> {
    Initial(Initial<S>),
    Update(CompiledUpdate<S>),
}
pub(crate) struct Completion<S: ProcessingSample> {
    pub basis: Basis,
    pub project: EngineProject,
    pub result: Result<Ready<S>, Diagnostic>,
}

pub(crate) fn spawn<S: ProcessingSample>(
    config: HostConfig,
    shared: Arc<Shared>,
    backend: Arc<dyn EngineBackend<S>>,
    jobs: Receiver<PrepareJob>,
    completed: SyncSender<Completion<S>>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("moiren-host-prepare".into())
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    prepare(&config, &shared, backend.as_ref(), &job)
                }))
                .unwrap_or_else(|_| Err(Diagnostic::Fault("preparation worker panicked".into())));
                let PrepareJob {
                    basis,
                    project,
                    active,
                    signals,
                    cancel,
                    ..
                } = job;
                drop(active);
                drop(signals);
                drop(cancel);
                if completed
                    .send(Completion {
                        basis,
                        project,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
                shared.notify();
            }
        })
}

fn prepare<S: ProcessingSample>(
    config: &HostConfig,
    shared: &Shared,
    backend: &dyn EngineBackend<S>,
    job: &PrepareJob,
) -> Result<Ready<S>, Diagnostic> {
    let fail = |error| Diagnostic::Backend(error);
    if job.cancel.is_cancelled() {
        return Err(fail(BackendError::Cancelled));
    }
    let (signals, rt_host) = match &job.signals {
        Some(signals) => (signals.clone(), None),
        None => {
            let (signals, rt_host) = shared
                .signals
                .create_host(job.basis.owner, config.signal_bindings)
                .map_err(fail)?;
            (signals, Some(rt_host))
        }
    };
    let mode = if job.active.is_some() {
        PrepareMode::Update
    } else {
        PrepareMode::Initial
    };
    let mut prepared = backend
        .prepare(
            PrepareRequest {
                project: &job.project,
                mode,
                reuse: &job.reuse,
                graph_revision: job.basis.graph,
                owner_generation: job.basis.owner,
                resource_generation: job.basis.resources,
                signal_host: &signals,
            },
            &job.cancel,
        )
        .map_err(fail)?;
    if job.cancel.is_cancelled() {
        return Err(fail(BackendError::Cancelled));
    }
    for (&node, configuration) in &job.project.nodes {
        if matches!(
            configuration,
            NodeConfig::Input(_) | NodeConfig::Output(_) | NodeConfig::Bus
        ) {
            continue;
        }
        if prepared.bindings.contains_node(node) {
            return Err(Diagnostic::Fault(
                "backend bound a host-owned builtin processor".into(),
            ));
        }
        let initial = |parameter, fallback| {
            job.project
                .parameters
                .iter()
                .find(|value| value.target == ParameterTarget::Node { node, parameter })
                .map_or(fallback, |value| value.value)
        };
        let result = match configuration {
            NodeConfig::Gain { initial: value } => prepared.bindings.bind_gain(
                node,
                float(initial(
                    crate::processor::Gain::LEVEL,
                    ParamValue::Float(*value),
                )),
            ),
            NodeConfig::Pan { initial: value } => prepared.bindings.bind_pan(
                node,
                float(initial(
                    crate::processor::Pan::POSITION,
                    ParamValue::Float(*value),
                )),
            ),
            NodeConfig::Compressor(settings) => {
                let mut settings = *settings;
                let values = [
                    &mut settings.input_gain_db,
                    &mut settings.threshold_db,
                    &mut settings.ratio,
                    &mut settings.attack_ms,
                    &mut settings.release_ms,
                    &mut settings.hold_ms,
                    &mut settings.knee_db,
                    &mut settings.makeup_gain_db,
                    &mut settings.output_gain_db,
                    &mut settings.mix,
                ];
                for (index, value) in values.into_iter().enumerate() {
                    *value = float(initial(
                        ParameterId(index as u32),
                        ParamValue::Float(*value),
                    ));
                }
                prepared.bindings.bind_compressor_with_signal(
                    node,
                    settings,
                    shared.signals.prepare(node, &signals).map_err(fail)?,
                )
            }
            _ => unreachable!(),
        };
        result.map_err(|error| Diagnostic::Compile(error.to_string()))?;
    }
    let mut graph = job.project.graph.clone();
    for parameter in &job.project.parameters {
        if let ParameterTarget::Edge {
            edge,
            parameter: kind,
        } = parameter.target
        {
            let mut params = *graph
                .get_edge(edge)
                .map_err(|error| Diagnostic::Compile(error.to_string()))?
                .params();
            match (kind, parameter.value) {
                (SendParameter::Gain, value) => params.gain = float(value),
                (SendParameter::Pan, value) => params.pan = float(value),
                (SendParameter::Mute, ParamValue::Bool(value)) => params.mute = value,
                _ => unreachable!(),
            }
            graph
                .set_send_params(edge, params)
                .map_err(|error| Diagnostic::Compile(error.to_string()))?;
        }
    }
    let compile_config = job
        .project
        .processing
        .compile_config(job.plan_revision, job.epoch);
    if let Some(basis) = &job.active {
        if prepared.transport.is_some() {
            return Err(Diagnostic::Fault(
                "compatible preparation replaced the native transport".into(),
            ));
        }
        compiler::compile_update_with_reuse(
            &graph,
            prepared.bindings,
            compile_config,
            basis,
            compiler::UpdateReuse {
                nodes: &job.reuse,
                edges: &job.edge_reuse,
            },
            Some(&signals),
        )
        .map(Ready::Update)
        .map_err(|error| Diagnostic::Compile(error.to_string()))
    } else {
        let transport = prepared
            .transport
            .take()
            .ok_or_else(|| Diagnostic::Fault("initial preparation omitted transport".into()))?;
        let mut compiled =
            compiler::compile_with_signals(&graph, prepared.bindings, compile_config, &signals)
                .map_err(|error| Diagnostic::Compile(error.to_string()))?;
        compiled
            .engine
            .attach_signal_host(rt_host.expect("initial signal owner"))
            .map_err(|error| Diagnostic::Compile(error.error.to_string()))?;
        let plans = compiled
            .engine
            .enable_plan_switching(config.cleanup_capacity)
            .map_err(|error| Diagnostic::Compile(error.to_string()))?;
        let snapshot = compiled.engine.plan_snapshot();
        if job.cancel.is_cancelled() {
            return Err(fail(BackendError::Cancelled));
        }
        let owner = transport
            .attach(compiled.engine, &compiled.bindings, job.cancel.clone())
            .map_err(|failure| {
                drop(failure.engine);
                fail(failure.error)
            })?;
        Ok(Ready::Initial(Initial {
            owner,
            plans,
            control: compiled.control,
            bindings: compiled.bindings,
            snapshot,
            signals,
        }))
    }
}
fn float(value: ParamValue) -> f64 {
    match value {
        ParamValue::Float(value) => value,
        _ => unreachable!("validated float parameter"),
    }
}
