//! Control-side backend ownership is kept apart from graph-to-plan construction.
use std::collections::{BTreeMap, btree_map::Entry};

use moiren_core::graph::{LogicalGraph, NodeId, NodeKind, PortId, PortRole};

use super::CompileError;
use crate::{
    boundary::{RtAudioSink, RtAudioSource, SinkAdapter, SourceAdapter},
    buffer::{PreparedIo, ProcessIo},
    control::ProcessParameters,
    processor::{CompressorSettings, ProcessContext, ProcessorError, ProcessorRole, RtProcessor},
    sample::ProcessingSample,
};

pub(super) enum Binding<S: ProcessingSample> {
    Io(Box<dyn RtProcessor<S>>),
    ReusedIo(
        crate::runtime::PlanSnapshot,
        moiren_core::protocol::ProcessorId,
    ),
    Gain(f64),
    Pan {
        position: f64,
        mono: bool,
        swap: bool,
    },
    Compressor(
        CompressorSettings,
        Option<moiren_signal::PreparedPublisher<moiren_signal::LatestValue<f32>>>,
    ),
}

/// Prepared backend ownership and initial DSP values, separate from editable
/// topology. All methods and all destruction are control-side operations.
pub struct NodeBindings<S: ProcessingSample> {
    nodes: BTreeMap<NodeId, Binding<S>>,
    observers: BTreeMap<PortId, Vec<Box<dyn crate::processor::RtObserver<S>>>>,
    input_observers: BTreeMap<PortId, Vec<Box<dyn crate::processor::RtObserver<S>>>>,
}
impl<S: ProcessingSample> Default for NodeBindings<S> {
    fn default() -> Self {
        Self::new()
    }
}
impl<S: ProcessingSample> NodeBindings<S> {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            observers: BTreeMap::new(),
            input_observers: BTreeMap::new(),
        }
    }

    pub fn contains_node(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    pub(super) fn bind_reused_io(
        &mut self,
        node: NodeId,
        basis: crate::runtime::PlanSnapshot,
        processor: moiren_core::protocol::ProcessorId,
    ) -> Result<(), CompileError> {
        self.insert(node, Binding::ReusedIo(basis, processor))
    }

    pub(super) fn take(&mut self, node: NodeId) -> Option<Binding<S>> {
        self.nodes.remove(&node)
    }

    pub(super) fn take_observers(
        &mut self,
        port: PortId,
    ) -> Option<Vec<Box<dyn crate::processor::RtObserver<S>>>> {
        self.observers.remove(&port)
    }
    pub(super) fn take_input_observers(
        &mut self,
        port: PortId,
    ) -> Option<Vec<Box<dyn crate::processor::RtObserver<S>>>> {
        self.input_observers.remove(&port)
    }

    pub(super) fn validate(&self, graph: &LogicalGraph) -> Result<(), CompileError> {
        for (&id, binding) in &self.nodes {
            let kind = graph.get_node(id)?.kind();
            let valid = match (kind, binding) {
                (NodeKind::Source, Binding::Io(p)) => p.role() == ProcessorRole::Source,
                (NodeKind::Sink, Binding::Io(p)) => p.role() == ProcessorRole::Sink,
                (NodeKind::Source, Binding::ReusedIo(basis, processor)) => {
                    basis.processor_role(*processor) == Some(ProcessorRole::Source)
                }
                (NodeKind::Sink, Binding::ReusedIo(basis, processor)) => {
                    basis.processor_role(*processor) == Some(ProcessorRole::Sink)
                }
                (NodeKind::Gain, Binding::Gain(_)) | (NodeKind::Pan, Binding::Pan { .. }) => true,
                (NodeKind::Compressor, Binding::Compressor(_, _)) => true,
                _ => false,
            };
            if !valid {
                return Err(CompileError::BindingKind { node: id });
            }
        }
        for &port in self.observers.keys() {
            let graph_port = graph
                .get_port(port)
                .map_err(|_| CompileError::ObserverPortNotFound { port })?;
            if graph_port.role() != PortRole::Output {
                return Err(CompileError::ObserverInputPort { port });
            }
        }
        for &port in self.input_observers.keys() {
            let graph_port = graph
                .get_port(port)
                .map_err(|_| CompileError::ObserverPortNotFound { port })?;
            if graph_port.role() != PortRole::Input {
                return Err(CompileError::ObserverOutputPort { port });
            }
        }
        Ok(())
    }

    fn insert(&mut self, node: NodeId, binding: Binding<S>) -> Result<(), CompileError> {
        // Entry preserves the first binding on error and avoids a second lookup.
        match self.nodes.entry(node) {
            Entry::Vacant(entry) => {
                entry.insert(binding);
                Ok(())
            }
            Entry::Occupied(_) => Err(CompileError::DuplicateBinding { node }),
        }
    }
    pub fn bind_source(
        &mut self,
        node: NodeId,
        source: impl RtAudioSource<S> + 'static,
    ) -> Result<(), CompileError> {
        self.bind_io(node, SourceAdapter(source))
    }
    pub fn bind_sink(
        &mut self,
        node: NodeId,
        sink: impl RtAudioSink<S> + 'static,
    ) -> Result<(), CompileError> {
        self.bind_io(node, SinkAdapter(sink))
    }
    /// Also accepts already prepared InputNode/OutputNode with their telemetry.
    pub fn bind_io(
        &mut self,
        node: NodeId,
        processor: impl RtProcessor<S> + 'static,
    ) -> Result<(), CompileError> {
        self.insert(node, Binding::Io(Box::new(processor)))
    }
    pub fn bind_gain(&mut self, node: NodeId, initial: f64) -> Result<(), CompileError> {
        self.insert(node, Binding::Gain(initial))
    }
    pub fn bind_pan(&mut self, node: NodeId, initial: f64) -> Result<(), CompileError> {
        self.bind_pan_with_stereo(node, initial, false, false)
    }
    pub fn bind_pan_with_stereo(
        &mut self,
        node: NodeId,
        position: f64,
        mono: bool,
        swap: bool,
    ) -> Result<(), CompileError> {
        self.insert(
            node,
            Binding::Pan {
                position,
                mono,
                swap,
            },
        )
    }
    pub fn bind_compressor(
        &mut self,
        node: NodeId,
        settings: CompressorSettings,
    ) -> Result<(), CompileError> {
        self.insert(node, Binding::Compressor(settings, None))
    }

    pub fn bind_compressor_with_signal(
        &mut self,
        node: NodeId,
        settings: CompressorSettings,
        publisher: moiren_signal::PreparedPublisher<moiren_signal::LatestValue<f32>>,
    ) -> Result<(), CompileError> {
        self.insert(node, Binding::Compressor(settings, Some(publisher)))
    }

    pub fn observe_output(
        &mut self,
        port: PortId,
        observer: impl crate::processor::RtObserver<S> + 'static,
    ) -> Result<(), CompileError> {
        self.observers
            .entry(port)
            .or_default()
            .push(Box::new(observer));
        Ok(())
    }
    /// Observe the actual node input after edge sends, including initialized
    /// silence when disconnected. Preparation owns the observer allocation.
    pub fn observe_input(
        &mut self,
        port: PortId,
        observer: impl crate::processor::RtObserver<S> + 'static,
    ) -> Result<(), CompileError> {
        self.input_observers
            .entry(port)
            .or_default()
            .push(Box::new(observer));
        Ok(())
    }
}

// Forwarding a boxed user processor preserves its role and validation hooks.
// This wrapper stays private, avoiding a blanket public Box implementation.
pub(super) struct BoundIo<S: ProcessingSample>(pub(super) Box<dyn RtProcessor<S>>);
impl<S: ProcessingSample> RtProcessor<S> for BoundIo<S> {
    fn state_type_id(&self) -> std::any::TypeId {
        self.0.state_type_id()
    }
    fn role(&self) -> ProcessorRole {
        self.0.role()
    }
    fn latency_frames(&self) -> u32 {
        self.0.latency_frames()
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        self.0.validate_io(io)
    }
    fn validate_parameters(&self, params: ProcessParameters<'_>) -> Result<(), ProcessorError> {
        self.0.validate_parameters(params)
    }
    fn process(
        &mut self,
        ctx: &ProcessContext,
        io: ProcessIo<'_, S>,
        params: ProcessParameters<'_>,
        signals: crate::signal::ProcessSignals<'_>,
    ) {
        self.0.process(ctx, io, params, signals);
    }
    fn signal_bindings(&self) -> &[moiren_signal::ErasedBinding] {
        self.0.signal_bindings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::ConstantSource;

    #[test]
    fn type_erased_io_forwards_concrete_dsp_identity() {
        let bound = BoundIo::<f64>(Box::new(SourceAdapter(ConstantSource {
            channels: 2,
            value: 1.0,
        })));
        assert_eq!(
            bound.state_type_id(),
            std::any::TypeId::of::<SourceAdapter<ConstantSource>>()
        );
        assert_ne!(
            bound.state_type_id(),
            std::any::TypeId::of::<BoundIo<f64>>()
        );
    }
}
