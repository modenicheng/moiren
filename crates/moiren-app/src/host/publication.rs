use super::renderer::HostSource;
use super::*;
use moiren_engine::runtime::{PreparedPlan, ProcessorReuse, RetireOutcome};

impl AudioHost {
    fn candidate_bindings(
        &mut self,
        consume: bool,
    ) -> Result<(NodeBindings<f32>, AudioReader<f32>), HostError> {
        let mut bindings = NodeBindings::new();
        for source in self.desired.sources.values() {
            bindings.bind_source(
                source.node,
                HostSource {
                    source: if consume {
                        self.backends.remove(&source.id)
                    } else {
                        None
                    },
                    gate: source.gate.clone(),
                    channels: self.config.channels,
                },
            )?;
            bindings.bind_gain(source.gain, source.level)?;
            bindings.bind_pan(source.pan, source.position)?;
        }
        for (&node, &settings) in &self.desired.compressors {
            bindings.bind_compressor(node, settings)?;
        }
        let (writer, reader) = audio_bridge(
            self.config.channels,
            self.config.max_block_frames,
            self.config.audio_byte_budget,
        )?;
        bindings.bind_sink(self.desired.sink, writer)?;
        Ok((bindings, reader))
    }
    fn reuse(&self, bindings: &CompiledBindings) -> Vec<ProcessorReuse> {
        let mut mappings = Vec::new();
        for node in self.desired.graph.nodes() {
            let same = match node.kind() {
                NodeKind::Source => self
                    .desired
                    .sources
                    .values()
                    .find(|s| s.node == node.id())
                    .is_some_and(|s| {
                        self.active
                            .state
                            .sources
                            .get(&s.id)
                            .is_some_and(|old| old.generation == s.generation)
                    }),
                NodeKind::Sink => node.id() == self.active.state.sink,
                NodeKind::Bus => false, // Prepared input counts can change.
                NodeKind::Compressor => {
                    self.desired.compressors.get(&node.id())
                        == self.active.state.compressors.get(&node.id())
                }
                NodeKind::Gain | NodeKind::Pan => true,
            };
            if same
                && let (Some(old), Some(new)) = (
                    self.active.bindings.node(node.id()),
                    bindings.node(node.id()),
                )
            {
                mappings.push(ProcessorReuse { old, new });
            }
        }
        for edge in self.desired.graph.edges() {
            if self
                .active
                .state
                .graph
                .get_edge(edge.id())
                .is_ok_and(|old| old == edge)
                && let (Some(old), Some(new)) = (
                    self.active.bindings.edge(edge.id()),
                    bindings.edge(edge.id()),
                )
            {
                mappings.push(ProcessorReuse {
                    old: old.gain.processor,
                    new: new.gain.processor,
                });
            }
        }
        mappings
    }

    /// Compile and queue the desired graph; confirmation arrives only in poll.
    /// Validation failure leaves edits/backends staged. A transport failure
    /// after preparation rolls desired state back to active and drops new IO on
    /// control. No candidate has an accepted parameter request before activation.
    pub fn publish(&mut self) -> Result<u64, HostError> {
        self.editable()?;
        if !self.dirty {
            return Err(HostError::NoChanges);
        }
        self.desired.validate()?;
        let revision = self.next_revision;
        let next_revision = revision.checked_add(1).ok_or(HostError::IdOverflow)?;
        // Validate using identical concrete placeholder types before moving a
        // newly prepared backend. Failed edits cannot consume its ownership.
        let (bindings, reader) = self.candidate_bindings(false)?;
        let checked = compiler::compile(
            &self.desired.graph,
            bindings,
            compile_config(self.config, revision),
        )?;
        let reuse = self.reuse(&checked.bindings);
        let checked_plan =
            PreparedPlan::new(checked.engine)?.with_reuse(&self.active.snapshot, &reuse)?;
        drop((checked_plan, checked.control, reader));
        let (bindings, reader) = self.candidate_bindings(true)?;
        let compiled = compiler::compile(
            &self.desired.graph,
            bindings,
            compile_config(self.config, revision),
        )?;
        let plan = PreparedPlan::new(compiled.engine)?.with_reuse(&self.active.snapshot, &reuse)?;
        let snapshot = plan.snapshot();
        if let Err(failure) = self.plans.publish(plan) {
            self.desired = self.active.state.clone();
            self.backends.clear();
            self.dirty = false;
            return Err(failure.reason.into());
        }
        self.pending = Some(Pending {
            active: Active {
                state: self.desired.clone(),
                control: compiled.control,
                bindings: compiled.bindings,
                snapshot,
                stats: compiled.stats,
            },
            _placeholder_reader: reader,
        });
        self.next_revision = next_revision;
        self.dirty = false;
        Ok(revision)
    }

    pub fn cancel_pending(&mut self) {
        self.plans.cancel_pending();
    }

    /// Reclaim retired packages and drain *all* terminal receipts on control.
    pub fn poll(&mut self) -> Vec<HostEvent> {
        let mut events = Vec::new();
        drain(&mut self.active.control, &mut events);
        while let Some(mut retired) = self.plans.poll_retired() {
            match retired.outcome() {
                RetireOutcome::Replaced {
                    active_revision,
                    frame,
                } => {
                    // Old control must outlive pending rejection. Queue-full
                    // backpressure is resolved by alternating drain and reject.
                    loop {
                        drain(&mut self.active.control, &mut events);
                        let remaining = retired.reject_pending();
                        drain(&mut self.active.control, &mut events);
                        if remaining == 0 {
                            break;
                        }
                    }
                    let pending = self.pending.take().expect("one host-owned candidate");
                    debug_assert_eq!(pending.active.snapshot.revision(), active_revision);
                    self.active = pending.active;
                    self.desired = self.active.state.clone();
                    events.push(HostEvent::PlanApplied {
                        revision: active_revision,
                        frame,
                    });
                }
                RetireOutcome::Rejected(reason) => {
                    let revision = retired.revision();
                    // Candidate controls are private until activation, hence no
                    // candidate request can be outstanding on rejection.
                    self.pending.take();
                    self.desired = self.active.state.clone();
                    events.push(HostEvent::PlanRejected { revision, reason });
                }
            }
            drop(retired);
        }
        drain(&mut self.active.control, &mut events);
        events
    }

    pub fn finish(self, renderer: HostRenderer) -> Vec<HostEvent> {
        let (engine, reader) = renderer.into_parts();
        self.finish_parts(engine, reader)
    }

    /// Call after backend join. This consumes the matching session's Engine and
    /// output reader and rejects accepted requests without an extra audio block.
    /// Both runtime ownership and SPSC queue destruction remain on control.
    pub fn finish_parts(mut self, engine: Engine<f32>, reader: AudioReader<f32>) -> Vec<HostEvent> {
        let mut events = self.poll();
        let (_, _, mut parameters) = engine.into_parts();
        loop {
            drain(&mut self.active.control, &mut events);
            let remaining = parameters.retire_and_reject_pending();
            drain(&mut self.active.control, &mut events);
            if remaining == 0 {
                break;
            }
        }
        if let Some(pending) = self.pending.take() {
            events.push(HostEvent::PlanRejected {
                revision: pending.active.snapshot.revision(),
                reason: moiren_engine::runtime::PlanSwapError::Cancelled,
            });
        }
        drop((parameters, reader));
        events
    }
}

fn drain(control: &mut ControlPort, events: &mut Vec<HostEvent>) {
    while let Some(reply) = control.poll_applied() {
        events.push(HostEvent::Parameter(reply));
    }
}
