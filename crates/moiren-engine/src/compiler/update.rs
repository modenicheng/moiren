//! Prepare schedule changes without reconstructing uniquely owned IO endpoints.
use super::{CompileConfig, CompileError, CompileStats, CompiledBindings, NodeBindings};
use crate::{
    control::ControlPort,
    runtime::{PlanSnapshot, PlanSwapError, PreparedPlan, ProcessorReuse},
    sample::ProcessingSample,
};
use moiren_core::{
    graph::{EdgeId, LogicalGraph, NodeId, NodeKind},
    protocol::ProcessorId,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UpdateCompileError {
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Swap(#[from] PlanSwapError),
}

pub struct CompiledUpdate<S: ProcessingSample> {
    pub plan: PreparedPlan<S>,
    pub control: ControlPort,
    pub bindings: CompiledBindings,
    pub stats: CompileStats,
}

pub struct UpdateReuse<'a> {
    pub nodes: &'a [(NodeId, ProcessorId)],
    pub edges: &'a [(EdgeId, ProcessorId)],
    /// Stable Rack slot identities; callers assert unchanged static settings
    /// and actual slot IO, just as they do for top-level processor reuse.
    pub rack_slots: &'a [((NodeId, u32), ProcessorId)],
}

/// Non-RT only. Every reused IO node must be absent from `bindings`: its
/// candidate slot contains metadata until the render boundary transfers the
/// original instance. The caller asserts unchanged static/backend identity.
pub fn compile_update<S: ProcessingSample>(
    graph: &LogicalGraph,
    bindings: NodeBindings<S>,
    config: CompileConfig,
    basis: &PlanSnapshot,
    reuse: &[(NodeId, ProcessorId)],
    host: Option<&moiren_signal::HostControl>,
) -> Result<CompiledUpdate<S>, UpdateCompileError> {
    compile_update_with_reuse(
        graph,
        bindings,
        config,
        basis,
        UpdateReuse {
            nodes: reuse,
            edges: &[],
            rack_slots: &[],
        },
        host,
    )
}

/// Includes stable edge processors so send parameter ramps survive unrelated
/// topology changes as well as node-local DSP state.
pub fn compile_update_with_reuse<S: ProcessingSample>(
    graph: &LogicalGraph,
    mut bindings: NodeBindings<S>,
    config: CompileConfig,
    basis: &PlanSnapshot,
    reuse: UpdateReuse<'_>,
    host: Option<&moiren_signal::HostControl>,
) -> Result<CompiledUpdate<S>, UpdateCompileError> {
    let observer_reuse = std::mem::take(&mut bindings.observer_reuse);
    for &(node, previous) in reuse.nodes {
        let kind = graph.get_node(node).map_err(CompileError::from)?.kind();
        if matches!(kind, NodeKind::Source | NodeKind::Sink) {
            bindings.bind_reused_io(node, basis.clone(), previous)?;
        }
    }
    let compiled = match host {
        Some(host) => super::compile_with_signals(graph, bindings, config, host)?,
        None => super::compile(graph, bindings, config)?,
    };
    let mut mappings = reuse
        .nodes
        .iter()
        .map(|&(node, previous)| {
            Ok(ProcessorReuse {
                old: previous,
                new: compiled
                    .bindings
                    .node(node)
                    .ok_or(PlanSwapError::MissingProcessor)?,
            })
        })
        .collect::<Result<Vec<_>, PlanSwapError>>()?;
    for &(edge, previous) in reuse.edges {
        let target = compiled
            .bindings
            .edge(edge)
            .ok_or(PlanSwapError::MissingProcessor)?;
        mappings.push(ProcessorReuse {
            old: previous,
            new: target.gain.processor,
        });
    }
    for &((node, slot), previous) in reuse.rack_slots {
        mappings.push(ProcessorReuse {
            old: previous,
            new: compiled
                .bindings
                .rack_slot(node, slot)
                .ok_or(PlanSwapError::MissingProcessor)?,
        });
    }
    for (node, previous) in observer_reuse {
        mappings.push(ProcessorReuse {
            old: previous,
            new: compiled
                .bindings
                .observer(node)
                .ok_or(PlanSwapError::MissingProcessor)?,
        });
    }
    let plan = PreparedPlan::new(compiled.engine)?.with_reuse(basis, &mappings)?;
    Ok(CompiledUpdate {
        plan,
        control: compiled.control,
        bindings: compiled.bindings,
        stats: compiled.stats,
    })
}
