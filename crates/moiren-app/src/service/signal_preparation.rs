use super::LevelTopic;
use moiren_core::graph::{LogicalGraph, PortId, PortRole};
use moiren_engine::{
    compiler::{CompileConfig, CompileError, CompiledGraph, NodeBindings, compile_with_signals},
    meter::signal::{MeterError, SignalLevelMeter},
    runtime::SignalAttachError,
};
use moiren_signal::{Bus, HostControl, HostError, RtHost, Topic};

#[derive(Debug, thiserror::Error)]
pub enum SignalPreparationError {
    #[error(transparent)]
    Host(#[from] HostError),
    #[error(transparent)]
    Meter(#[from] MeterError),
    #[error(transparent)]
    Graph(#[from] moiren_core::graph::GraphError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Attach(#[from] SignalAttachError),
}

pub struct SignalPreparation {
    control: HostControl,
    host: Option<RtHost>,
    input: Option<Topic<LevelTopic>>,
    output: Topic<LevelTopic>,
}

impl SignalPreparation {
    pub fn new(
        bus: &mut Bus,
        input: Option<Topic<LevelTopic>>,
        output: Topic<LevelTopic>,
    ) -> Result<Self, HostError> {
        let (control, host) = bus.create_host(if input.is_some() { 2 } else { 1 })?;
        Ok(Self {
            control,
            host: Some(host),
            input,
            output,
        })
    }

    pub(crate) fn compile(
        &mut self,
        graph: &LogicalGraph,
        mut bindings: NodeBindings<f32>,
        config: CompileConfig,
        input_port: PortId,
        output_port: PortId,
    ) -> Result<CompiledGraph<f32>, SignalPreparationError> {
        for (port, topic) in self
            .input
            .as_ref()
            .map(|topic| (input_port, topic))
            .into_iter()
            .chain(std::iter::once((output_port, &self.output)))
        {
            let publisher = self.control.prepare(topic, self.control.new_binding()?)?;
            let observer = SignalLevelMeter::new(
                graph.get_port(port)?.channels(),
                moiren_engine::meter::ui_window_frames(config.engine.processing_sr),
                publisher,
            )?;
            match graph.get_port(port)?.role() {
                PortRole::Input => bindings.observe_input(port, observer)?,
                PortRole::Output => bindings.observe_output(port, observer)?,
            }
        }
        let mut compiled = compile_with_signals(graph, bindings, config, &self.control)?;
        let host = self.host.take().ok_or(HostError::Closed)?;
        if let Err(failure) = compiled.engine.attach_signal_host(host) {
            self.host = Some(failure.host);
            return Err(failure.error.into());
        }
        Ok(compiled)
    }
}
