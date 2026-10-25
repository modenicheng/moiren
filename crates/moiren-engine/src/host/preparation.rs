use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender},
    },
    thread,
};

use moiren_core::{
    graph::{EdgeId, LogicalGraph, NodeId},
    protocol::{ParamValue, ProcessorId},
};

use super::{
    BackendError, BackendOwner, CancellationToken, Diagnostic, EngineBackend, EngineProject,
    HostConfig, NodeConfig, ParameterTarget, PrepareMode, PrepareRequest, PreparedBackend,
    SendParameter, Shared, StripParameter, signals::PreparedReaders,
};
use crate::{
    compiler::{self, CompileConfig, CompiledBindings, CompiledUpdate, NodeBindings},
    processor::{Compressor, Delay, Eq, Gain, Gate, Limiter, Pan, builtin::ChannelStrip},
    runtime::{PlanControlPort, PlanSnapshot},
    sample::ProcessingSample,
    spectrum::SignalSpectrumAnalyzer,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Basis {
    pub graph: u64,
    pub owner: u64,
    pub resources: u64,
    pub active_plan: u64,
    pub parameter_intent: u64,
}
pub(crate) struct PrepareJob {
    pub basis: Basis,
    pub plan_revision: u64,
    pub epoch: u64,
    pub project: EngineProject,
    pub reuse: Vec<(NodeId, ProcessorId)>,
    pub edge_reuse: Vec<(EdgeId, ProcessorId)>,
    pub rack_reuse: Vec<((NodeId, u32), ProcessorId)>,
    pub initial_parameters: Vec<ParameterTarget>,
    pub observer_reuse: Vec<(NodeId, ProcessorId)>,
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
    pub readers: PreparedReaders,
    pub owner_generation: u64,
}
pub(crate) enum Ready<S: ProcessingSample> {
    Initial(Initial<S>),
    Update {
        update: CompiledUpdate<S>,
        readers: PreparedReaders,
    },
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
    let (signals, rt_host) = acquire_signal_host(config, shared, job)?;
    let mut readers = PreparedReaders::default();
    let mut prepared = prepare_backend(backend, job, &signals)?;
    if job.cancel.is_cancelled() {
        return Err(fail(BackendError::Cancelled));
    }
    bind_host_processors(shared, job, &signals, &mut prepared.bindings, &mut readers)?;
    let inputs = CompileInputs {
        graph: apply_edge_parameters(job)?,
        backend: prepared,
        config: job
            .project
            .processing
            .compile_config(job.plan_revision, job.epoch),
        signals,
        rt_host,
        readers,
    };
    if let Some(basis) = &job.active {
        compile_update_ready(job, basis, inputs)
    } else {
        compile_initial_ready(config, job, inputs)
    }
}

/// Inputs of the compile phase: the intent-adjusted graph, the backend's
/// bindings and transport, the compilation identity, the signal host halves
/// and the observer readers accumulated by the earlier phases.
struct CompileInputs<S: ProcessingSample> {
    graph: LogicalGraph,
    backend: PreparedBackend<S>,
    config: CompileConfig,
    signals: moiren_signal::HostControl,
    rt_host: Option<moiren_signal::RtHost>,
    readers: PreparedReaders,
}

/// Signal-host phase: an owner-carried host is reused, otherwise a fresh bus
/// host is created. The runtime half is present only when this worker created
/// it, because only initial preparations attach the RT pump.
fn acquire_signal_host(
    config: &HostConfig,
    shared: &Shared,
    job: &PrepareJob,
) -> Result<(moiren_signal::HostControl, Option<moiren_signal::RtHost>), Diagnostic> {
    match &job.signals {
        Some(signals) => Ok((signals.clone(), None)),
        None => {
            let (signals, rt_host) = shared
                .signals
                .create_host(job.basis.owner, config.signal_bindings)
                .map_err(Diagnostic::Backend)?;
            Ok((signals, Some(rt_host)))
        }
    }
}

/// Backend phase: the transport-owning backend resolves IO and observers for
/// the requested mode. It never compiles the logical graph.
fn prepare_backend<S: ProcessingSample>(
    backend: &dyn EngineBackend<S>,
    job: &PrepareJob,
    signals: &moiren_signal::HostControl,
) -> Result<PreparedBackend<S>, Diagnostic> {
    let mode = if job.active.is_some() {
        PrepareMode::Update
    } else {
        PrepareMode::Initial
    };
    backend
        .prepare(
            PrepareRequest {
                project: &job.project,
                mode,
                reuse: &job.reuse,
                graph_revision: job.basis.graph,
                owner_generation: job.basis.owner,
                resource_generation: job.basis.resources,
                signal_host: signals,
            },
            &job.cancel,
        )
        .map_err(Diagnostic::Backend)
}

/// Node-binding phase: strips, one dispatch per node configuration (with
/// recorded intent applied through each kernel's `resolve*` entry point),
/// recorded rack-slot parameters, and the level-meter signal bindings.
fn bind_host_processors<S: ProcessingSample>(
    shared: &Shared,
    job: &PrepareJob,
    signals: &moiren_signal::HostControl,
    bindings: &mut NodeBindings<S>,
    readers: &mut PreparedReaders,
) -> Result<(), Diagnostic> {
    for (&node, config) in &job.project.strips {
        // Recorded strip parameter intent overrides the configuration's
        // defaults, so a recompile keeps live values instead of snapping back.
        let channels = job.project.strip_width(node).unwrap_or(2);
        let intent = |parameter, fallback| {
            let strip = match parameter {
                ChannelStrip::GAIN => StripParameter::Gain,
                ChannelStrip::BALANCE => StripParameter::Balance,
                ChannelStrip::MODE => StripParameter::Mode,
                _ => StripParameter::EffectiveMute,
            };
            job.project
                .parameters
                .iter()
                .find(|value| {
                    value.target
                        == ParameterTarget::Strip {
                            node,
                            parameter: strip,
                        }
                })
                .map_or(fallback, |value| value.value)
        };
        let config = ChannelStrip::resolve(*config, channels, intent);
        bindings.bind_strip(node, config);
    }
    for (&node, configuration) in &job.project.nodes {
        if matches!(
            configuration,
            NodeConfig::Input(_) | NodeConfig::Output(_) | NodeConfig::Bus
        ) {
            continue;
        }
        if bindings.contains_node(node) {
            return Err(Diagnostic::Fault(
                "backend bound a host-owned builtin processor".into(),
            ));
        }
        // One intent lookup per parameter id: recorded node parameters
        // override the configuration's default initial values. The kernels'
        // own `resolve*` entry points apply it, so the settings-to-parameter
        // mapping lives with each processor instead of here.
        let initial = |parameter, fallback| {
            job.project
                .parameters
                .iter()
                .find(|value| value.target == ParameterTarget::Node { node, parameter })
                .map_or(fallback, |value| value.value)
        };
        let result = match configuration {
            NodeConfig::Gain { initial: value } => {
                bindings.bind_gain(node, Gain::resolve_level(*value, initial))
            }
            NodeConfig::Pan { initial: value } => {
                let (position, mono, swap) = Pan::resolve(*value, false, false, initial);
                bindings.bind_pan_with_stereo(node, position, mono, swap)
            }
            NodeConfig::Compressor(settings) => {
                let settings = Compressor::resolve_settings(*settings, initial);
                bindings.bind_compressor_with_signal(
                    node,
                    settings,
                    shared
                        .signals
                        .prepare(node, signals)
                        .map_err(Diagnostic::Backend)?,
                )
            }
            NodeConfig::Eq(settings) => {
                bindings.bind_eq(node, Eq::resolve_settings(*settings, initial))
            }
            NodeConfig::Gate(settings) => {
                bindings.bind_gate(node, Gate::resolve_settings(*settings, initial))
            }
            NodeConfig::Limiter(settings) => {
                bindings.bind_limiter(node, Limiter::resolve_settings(*settings, initial))
            }
            NodeConfig::Delay(settings) => {
                bindings.bind_delay(node, Delay::resolve_settings(*settings, initial))
            }
            NodeConfig::ChannelMapper(settings) => {
                bindings.bind_channel_mapper(node, settings.clone())
            }
            NodeConfig::Recorder(settings) => bindings
                .bind_recorder(
                    node,
                    job.project
                        .graph
                        .get_node(node)
                        .map_err(|e| Diagnostic::Compile(e.to_string()))?
                        .channels(),
                    *settings,
                )
                .map(|reader| {
                    readers.recorders.insert(node, Some(reader));
                }),
            NodeConfig::Meter(settings) => bindings.bind_meter(node, *settings).map(|reader| {
                readers.meters.insert(node, Some(reader));
            }),
            NodeConfig::SpectrumAnalyzer(settings) => {
                let channels = job
                    .project
                    .graph
                    .get_node(node)
                    .map_err(|e| Diagnostic::Compile(e.to_string()))?
                    .channels();
                let publisher = shared
                    .signals
                    .prepare_spectrum(node, channels, *settings, signals)
                    .map_err(Diagnostic::Backend)?;
                SignalSpectrumAnalyzer::new(channels, *settings, publisher)
                    .map_err(|_| compiler::CompileError::BindingKind { node })
                    .and_then(|analyzer| bindings.bind_spectrum(node, analyzer))
            }
            NodeConfig::Rack(settings) => bindings.bind_rack(node, settings.clone()),
            _ => unreachable!(),
        };
        result.map_err(|error| Diagnostic::Compile(error.to_string()))?;
    }
    for parameter in &job.project.parameters {
        if let ParameterTarget::RackSlot {
            node,
            slot,
            parameter: id,
        } = parameter.target
        {
            bindings
                .bind_rack_parameter(node, slot, id, parameter.value)
                .map_err(|error| Diagnostic::Compile(error.to_string()))?;
        }
    }
    shared
        .signals
        .prepare_levels(&job.project, signals, bindings)
        .map_err(Diagnostic::Backend)?;
    Ok(())
}

/// Edge-writeback phase: recorded send intent is applied to a private graph
/// clone, so compilation sees the adjusted gains, pans and mutes without
/// mutating the project's own topology.
fn apply_edge_parameters(job: &PrepareJob) -> Result<LogicalGraph, Diagnostic> {
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
    Ok(graph)
}

/// Update-compile phase: binds observer reuse, compiles against the active
/// plan's basis so live state survives, and reclaims the reader slots of
/// observers whose transfer succeeded.
fn compile_update_ready<S: ProcessingSample>(
    job: &PrepareJob,
    basis: &PlanSnapshot,
    inputs: CompileInputs<S>,
) -> Result<Ready<S>, Diagnostic> {
    let CompileInputs {
        graph,
        backend: PreparedBackend {
            mut bindings,
            transport,
        },
        config: compile_config,
        signals,
        rt_host: _,
        mut readers,
    } = inputs;
    if transport.is_some() {
        return Err(Diagnostic::Fault(
            "compatible preparation replaced the native transport".into(),
        ));
    }
    for &(node, processor) in &job.observer_reuse {
        bindings
            .bind_observer_reuse(node, processor)
            .map_err(|error| Diagnostic::Compile(error.to_string()))?;
    }
    let mut update = compiler::compile_update_with_reuse(
        &graph,
        bindings,
        compile_config,
        basis,
        compiler::UpdateReuse {
            nodes: &job.reuse,
            edges: &job.edge_reuse,
            rack_slots: &job.rack_reuse,
        },
        Some(&signals),
    )
    .map_err(|error| Diagnostic::Compile(error.to_string()))?;
    let initial: Vec<_> = job
        .initial_parameters
        .iter()
        .flat_map(|&target| super::parameters::resolve_all(&update.bindings, target))
        .collect();
    update.plan = update.plan.with_initial_parameters(&initial);
    // Only a successfully validated observer transfer may keep its original
    // reader. Placeholder endpoints are reclaimed here, never on RT.
    for &(node, _) in &job.observer_reuse {
        if let Some(reader) = readers.meters.get_mut(&node) {
            *reader = None;
        }
        if let Some(reader) = readers.recorders.get_mut(&node) {
            *reader = None;
        }
    }
    Ok(Ready::Update { update, readers })
}

/// Initial-compile phase: attaches the RT signal pump and the transport
/// owner to a freshly compiled engine and snapshots the published plan.
fn compile_initial_ready<S: ProcessingSample>(
    config: &HostConfig,
    job: &PrepareJob,
    inputs: CompileInputs<S>,
) -> Result<Ready<S>, Diagnostic> {
    let CompileInputs {
        graph,
        backend: PreparedBackend {
            bindings,
            transport,
        },
        config: compile_config,
        signals,
        rt_host,
        readers,
    } = inputs;
    let transport = transport
        .ok_or_else(|| Diagnostic::Fault("initial preparation omitted transport".into()))?;
    let mut compiled = compiler::compile_with_signals(&graph, bindings, compile_config, &signals)
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
        return Err(Diagnostic::Backend(BackendError::Cancelled));
    }
    let owner = transport
        .attach(compiled.engine, &compiled.bindings, job.cancel.clone())
        .map_err(|failure| {
            drop(failure.engine);
            Diagnostic::Backend(failure.error)
        })?;
    Ok(Ready::Initial(Initial {
        owner,
        plans,
        control: compiled.control,
        bindings: compiled.bindings,
        snapshot,
        signals,
        readers,
        owner_generation: job.basis.owner,
    }))
}
fn float(value: ParamValue) -> f64 {
    match value {
        ParamValue::Float(value) => value,
        _ => unreachable!("validated float parameter"),
    }
}
