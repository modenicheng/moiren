//! Hardware-free managed host: callers edit the project, not its runtime lifetime.
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use moiren_core::{
    graph::{NodeKind, SendParams},
    protocol::ParamValue,
};
use moiren_engine::{
    boundary::{BoundaryReport, ConstantSource, RtAudioSink},
    buffer::AudioBlock,
    compiler::{CompiledBindings, NodeBindings},
    host::*,
    node::{DeviceOptions, InputConfig, InputSource, OutputConfig, OutputTarget},
    processor::{Gain, ProcessContext},
    runtime::Engine,
};

struct SoftwareBackend(Arc<AtomicU64>);
struct Sink(Arc<AtomicU64>);
impl RtAudioSink<f64> for Sink {
    fn channel_count(&self) -> usize {
        1
    }
    fn write(&mut self, _: &ProcessContext, input: AudioBlock<'_, f64>) -> BoundaryReport {
        self.0
            .store(input.channel(0)[0].to_bits(), Ordering::Relaxed);
        BoundaryReport {
            transferred_frames: input.frames(),
            ..Default::default()
        }
    }
}

impl EngineBackend<f64> for SoftwareBackend {
    fn prepare(
        &self,
        request: PrepareRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<PreparedBackend<f64>, BackendError> {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        let mut bindings = NodeBindings::new();
        for (&node, config) in &request.project.nodes {
            if request.reuse.iter().any(|&(reused, _)| reused == node) {
                continue;
            }
            match config {
                NodeConfig::Input(InputConfig {
                    source: InputSource::Software { name },
                    channels: 1,
                    ..
                }) if name == "constant" => {
                    bindings
                        .bind_source(
                            node,
                            ConstantSource {
                                channels: 1,
                                value: 1.0,
                            },
                        )
                        .map_err(|error| BackendError::Failed(error.to_string()))?;
                }
                NodeConfig::Output(OutputConfig {
                    target: OutputTarget::Software { name },
                    channels: 1,
                    ..
                }) if name == "probe" => {
                    bindings
                        .bind_sink(node, Sink(self.0.clone()))
                        .map_err(|error| BackendError::Failed(error.to_string()))?;
                }
                NodeConfig::Input(_) | NodeConfig::Output(_) => {
                    return Err(BackendError::Unsupported(
                        "demo supports one-channel constant/probe IO".into(),
                    ));
                }
                _ => {}
            }
        }
        let transport = (request.mode == PrepareMode::Initial)
            .then(|| Box::new(Transport) as Box<dyn PreparedTransport<f64>>);
        Ok(PreparedBackend {
            bindings,
            transport,
        })
    }
}

struct Progress {
    activated: AtomicBool,
    sequence: AtomicU64,
    frame: AtomicU64,
    revision: AtomicU64,
    epoch: u64,
    failed: AtomicBool,
}
impl Progress {
    fn publish(&self, engine: &Engine<f64>) {
        let sequence = self.sequence.load(Ordering::SeqCst);
        self.sequence
            .store(sequence.wrapping_add(1), Ordering::SeqCst);
        self.frame.store(engine.timeline(), Ordering::SeqCst);
        self.revision.store(engine.revision(), Ordering::SeqCst);
        self.sequence
            .store(sequence.wrapping_add(2), Ordering::SeqCst);
    }
    fn timeline(&self) -> Option<OwnerTimeline> {
        let sequence = self.sequence.load(Ordering::SeqCst);
        if !sequence.is_multiple_of(2) {
            return None;
        }
        let timeline = OwnerTimeline {
            epoch: self.epoch,
            revision: self.revision.load(Ordering::SeqCst),
            frame: self.frame.load(Ordering::SeqCst),
        };
        (sequence == self.sequence.load(Ordering::SeqCst)).then_some(timeline)
    }
}

struct Transport;
struct Driver {
    progress: Arc<Progress>,
    cancel: CancellationToken,
    worker: JoinHandle<OwnerReturn<f64>>,
}
impl PreparedTransport<f64> for Transport {
    fn attach(
        self: Box<Self>,
        mut engine: Engine<f64>,
        _: &CompiledBindings,
        cancel: CancellationToken,
    ) -> Result<Box<dyn BackendOwner<f64>>, Box<AttachFailure<f64>>> {
        let progress = Arc::new(Progress {
            activated: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            revision: AtomicU64::new(engine.revision()),
            epoch: engine.epoch(),
            failed: AtomicBool::new(false),
        });
        let running = progress.clone();
        let cancellation = cancel.clone();
        let worker = thread::spawn(move || {
            while !running.activated.load(Ordering::Acquire) && !cancellation.is_cancelled() {
                cancellation.wait_cancelled(Duration::from_millis(1));
            }
            let mut error = None;
            while !cancellation.is_cancelled() {
                match catch_unwind(AssertUnwindSafe(|| {
                    engine.render(engine.config().max_block_frames)
                })) {
                    Ok(Ok(_)) => running.publish(&engine),
                    Ok(Err(failure)) => {
                        error = Some(BackendError::Failed(failure.to_string()));
                        break;
                    }
                    Err(_) => {
                        error = Some(BackendError::WorkerPanicked);
                        break;
                    }
                }
                cancellation.wait_cancelled(Duration::from_millis(1));
            }
            running.failed.store(error.is_some(), Ordering::Release);
            OwnerReturn { engine, error }
        });
        Ok(Box::new(Driver {
            progress,
            cancel,
            worker,
        }))
    }
}
impl BackendOwner<f64> for Driver {
    fn activate(&self) -> Result<(), BackendError> {
        if self.cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        self.progress.activated.store(true, Ordering::Release);
        Ok(())
    }
    fn cancel(&self) {
        self.cancel.cancel();
    }
    fn status(&self) -> OwnerStatus {
        if self.progress.failed.load(Ordering::Acquire) {
            OwnerStatus::Failed(BackendError::Failed("software driver stopped".into()))
        } else if self.progress.frame.load(Ordering::Acquire) > 0 {
            OwnerStatus::Running
        } else {
            OwnerStatus::Starting
        }
    }
    fn timeline(&self) -> Option<OwnerTimeline> {
        self.progress.timeline()
    }
    fn stop_and_join(self: Box<Self>) -> OwnerReturn<f64> {
        self.cancel.cancel();
        self.worker
            .join()
            .expect("driver retains Engine across processing failures")
    }
}

fn main() -> anyhow::Result<()> {
    let mut project = EngineProject::default();
    let input = project.graph.create_node(NodeKind::Source, 1)?;
    let gain = project.graph.create_node(NodeKind::Gain, 1)?;
    let output = project.graph.create_node(NodeKind::Sink, 1)?;
    project.nodes.insert(
        input,
        NodeConfig::Input(InputConfig {
            source: InputSource::Software {
                name: "constant".into(),
            },
            channels: 1,
            device: DeviceOptions::default(),
        }),
    );
    project
        .nodes
        .insert(gain, NodeConfig::Gain { initial: 0.5 });
    project.nodes.insert(
        output,
        NodeConfig::Output(OutputConfig {
            target: OutputTarget::Software {
                name: "probe".into(),
            },
            channels: 1,
            device: DeviceOptions::default(),
        }),
    );
    project.graph.connect(
        project.graph.get_node(input)?.outputs()[0].id(),
        project.graph.get_node(gain)?.inputs()[0].id(),
        SendParams::default(),
    )?;
    let edge = project.graph.connect(
        project.graph.get_node(gain)?.outputs()[0].id(),
        project.graph.get_node(output)?.inputs()[0].id(),
        SendParams::default(),
    )?;
    let sample = Arc::new(AtomicU64::new(0));
    let observer = {
        let host = EngineHost::open(
            HostConfig::default(),
            project,
            SoftwareBackend(sample.clone()),
        )?;
        let receipt = host.control().parameter(
            ParameterTarget::Node {
                node: gain,
                parameter: Gain::LEVEL,
            },
            ParamValue::Float(0.25),
            ParameterTiming::NextBlock,
            0,
        )?;
        anyhow::ensure!(
            matches!(
                receipt.wait(Duration::from_secs(5)),
                ParameterOutcome::Applied(_)
            ),
            "parameter did not apply"
        );
        let edited = host.control().edit(move |project| {
            project.graph.disconnect(edge)?;
            let added = project.graph.create_node(NodeKind::Gain, 1)?;
            project
                .nodes
                .insert(added, NodeConfig::Gain { initial: 0.5 });
            project.graph.connect(
                project.graph.get_node(gain)?.outputs()[0].id(),
                project.graph.get_node(added)?.inputs()[0].id(),
                SendParams::default(),
            )?;
            project.graph.connect(
                project.graph.get_node(added)?.outputs()[0].id(),
                project.graph.get_node(output)?.inputs()[0].id(),
                SendParams::default(),
            )?;
            Ok(())
        })?;
        anyhow::ensure!(
            matches!(
                edited.wait(Duration::from_secs(5)),
                EditOutcome::Activated {
                    graph_revision: 2,
                    ..
                }
            ),
            "graph did not activate"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while f64::from_bits(sample.load(Ordering::Acquire)) != 0.125 {
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "updated output is incorrect"
            );
            thread::sleep(Duration::from_millis(1));
        }
        println!("Automatic graph update: {:?}", host.observer().snapshot());
        host.observer()
    };
    anyhow::ensure!(
        observer.snapshot().host == HostState::Closed,
        "scope exit did not finish shutdown"
    );
    println!("Host automatically closed and reclaimed its runtime.");
    Ok(())
}
