use super::*;
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::audio_config::SampleFormat;
use moiren_engine::{
    boundary::{BoundaryReport, RtAudioSource},
    buffer::AudioBlockMut,
    compiler::{CompileConfig, NodeBindings, compile},
    processor::ProcessContext,
    runtime::EngineConfig,
};
use std::marker::PhantomData;

struct DemandStream {
    demand: usize,
    written: usize,
}

impl NativeStream for DemandStream {
    fn info(&self) -> NativeStreamInfo {
        NativeStreamInfo {
            sample_rate: 48_000,
            format: SampleFormat::Float32,
            buffer_frames: 256,
            channels: 2,
        }
    }
    fn start(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        Ok(true)
    }
    fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
        Ok(0)
    }
    fn render_demand(&mut self) -> Result<usize, BackendError> {
        Ok(self.demand)
    }
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        self.written += input.len();
        Ok(input.len())
    }
}

#[test]
fn pump_renders_current_demand_without_prefetching_native_capacity() {
    let (_tx, rx) = rtrb::RingBuffer::new(1024);
    let counters = Arc::new(Counters::default());
    let mut stream = DemandStream {
        demand: 32,
        written: 0,
    };
    let mut pump = Pump::new(
        None,
        None,
        Some(rx),
        stream.info(),
        48_000,
        1024,
        counters.clone(),
        false,
    )
    .unwrap();
    let timeline = Timeline::default();
    assert!(pump.cycle::<f32>(&mut stream, None, &timeline).unwrap());
    assert_eq!(stream.written, 32);
    assert_eq!(counters.silent.load(Ordering::Relaxed), 32);

    stream.demand = 0;
    assert!(!pump.cycle::<f32>(&mut stream, None, &timeline).unwrap());
    assert_eq!(stream.written, 32);
    assert_eq!(counters.silent.load(Ordering::Relaxed), 32);

    stream.demand = 257;
    assert!(pump.cycle::<f32>(&mut stream, None, &timeline).is_err());
    assert_eq!(stream.written, 32);
}

struct IndexedSource {
    next: u64,
}

fn indexed_sample(index: u64) -> StereoFrame {
    let value = ((index.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) % 65536) as f64
        / 65536.0
        - 0.5)
        * 0.1;
    [value, -value]
}

impl<S: ProcessingSample> RtAudioSource<S> for IndexedSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        for frame in 0..output.frames() {
            let sample = indexed_sample(self.next);
            self.next += 1;
            output.channel_mut(0)[frame] = S::from_f64(sample[0]);
            output.channel_mut(1)[frame] = S::from_f64(sample[1]);
        }
        BoundaryReport {
            transferred_frames: output.frames(),
            ..Default::default()
        }
    }
}

struct MasterDemandStream {
    info: NativeStreamInfo,
    demand: usize,
    write_limit: usize,
    samples: Vec<StereoFrame>,
}

impl NativeStream for MasterDemandStream {
    fn info(&self) -> NativeStreamInfo {
        self.info
    }
    fn start(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        Ok(true)
    }
    fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
        Ok(0)
    }
    fn render_demand(&mut self) -> Result<usize, BackendError> {
        Ok(self.demand)
    }
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        let count = input.len().min(self.write_limit);
        self.samples.extend_from_slice(&input[..count]);
        Ok(count)
    }
}

fn master_fixture<S: ProcessingSample>(
    rate: u32,
    native_rate: u32,
    block: usize,
) -> (Pump, Engine<S>, MasterDemandStream) {
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let bus = graph.create_node(NodeKind::Bus, 2).unwrap();
    let bus_input = graph.add_input_port(bus, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    graph
        .connect(
            graph.get_node(source).unwrap().outputs()[0].id(),
            bus_input,
            Default::default(),
        )
        .unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(sink).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    let (tx, rx) = rtrb::RingBuffer::new(8192);
    let counters = Arc::new(Counters::default());
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(source, IndexedSource { next: 0 })
        .unwrap();
    bindings
        .bind_sink(
            sink,
            crate::bridge::RingSink::<S> {
                tx,
                counters: counters.clone(),
                marker: PhantomData,
            },
        )
        .unwrap();
    let compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: f64::from(rate),
                max_block_frames: block,
                max_events_per_block: 8,
            },
            audio_byte_budget: 1024 * 1024,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 8,
            control_horizon_frames: 48000,
        },
    )
    .unwrap();
    let stream = MasterDemandStream {
        info: NativeStreamInfo {
            sample_rate: native_rate,
            format: SampleFormat::Float32,
            buffer_frames: 738,
            channels: 2,
        },
        demand: 0,
        write_limit: usize::MAX,
        samples: Vec::with_capacity(8192),
    };
    let pump = Pump::new(
        None,
        None,
        Some(rx),
        stream.info(),
        rate,
        8192,
        counters,
        true,
    )
    .unwrap();
    (pump, compiled.engine, stream)
}

fn exact_master_demand<S: ProcessingSample>() {
    for rate in moiren_engine::audio_config::SAMPLE_RATES {
        for native_rate in moiren_engine::audio_config::SAMPLE_RATES {
            for block in [32, 64, 128, 256, 512, 1024, 2048, 4096] {
                let (mut pump, mut engine, mut native) =
                    master_fixture::<S>(rate, native_rate, block);
                let mut reference = StereoResampler::new(rate, native_rate, 8192).unwrap();
                let timeline = Timeline::default();
                let mut generated = 0;
                for demand in [32, 0, 512, 1, 336, 7, 64, 738] {
                    native.demand = demand;
                    let needed = reference.input_frames_needed(demand).unwrap();
                    for index in generated..generated + needed {
                        let frame =
                            indexed_sample(index as u64).map(|sample| S::from_f64(sample).to_f64());
                        assert!(reference.push_frame(frame));
                    }
                    generated += needed;
                    let mut expected = vec![[0.0; 2]; demand];
                    assert_eq!(reference.produce(&mut expected), demand);
                    let before = native.samples.len();
                    pump.cycle(&mut native, Some(&mut engine), &timeline)
                        .unwrap();
                    assert_eq!(
                        engine.timeline(),
                        generated as u64,
                        "native demand must not pre-render unused Engine block tails at {rate}/{native_rate}, block {block}, demand {demand}"
                    );
                    assert_eq!(&native.samples[before..], expected);
                    assert_eq!(pump.render.as_mut().unwrap().slots(), 0);
                    assert_eq!(pump.counters.dropped.load(Ordering::Relaxed), 0);
                }
            }
        }
    }
}

#[test]
fn master_renders_only_missing_src_input_without_hidden_engine_tail_f32() {
    exact_master_demand::<f32>();
}

#[test]
fn master_renders_only_missing_src_input_without_hidden_engine_tail_f64() {
    exact_master_demand::<f64>();
}

fn backpressured_master<S: ProcessingSample>() {
    let (mut pump, mut engine, mut native) = master_fixture::<S>(48000, 48000, 64);
    let timeline = Timeline::default();
    let mut reference = StereoResampler::new(48000, 48000, 8192).unwrap();
    let needed = reference.input_frames_needed(97).unwrap();
    for index in 0..needed {
        assert!(
            reference.push_frame(
                indexed_sample(index as u64).map(|sample| S::from_f64(sample).to_f64())
            )
        );
    }
    let mut expected = [[0.0; 2]; 97];
    assert_eq!(reference.produce(&mut expected), 97);
    native.demand = 97;
    native.write_limit = 0;
    pump.cycle(&mut native, Some(&mut engine), &timeline)
        .unwrap();
    assert_eq!(engine.timeline(), needed as u64);
    assert!(native.samples.is_empty());
    // A larger reported demand cannot consume another graph input while the
    // original native payload is still waiting on zero/partial writes.
    native.demand = 738;
    for limit in [0, 17, 0, 1, 64, 15] {
        native.write_limit = limit;
        pump.cycle(&mut native, Some(&mut engine), &timeline)
            .unwrap();
        assert_eq!(engine.timeline(), needed as u64);
    }
    assert_eq!(native.samples, expected);
    native.demand = 0;
    pump.cycle(&mut native, Some(&mut engine), &timeline)
        .unwrap();
    assert_eq!(engine.timeline(), needed as u64);
    assert_eq!(native.samples, expected);
    native.demand = 739;
    assert!(
        pump.cycle(&mut native, Some(&mut engine), &timeline)
            .is_err()
    );
    assert_eq!(engine.timeline(), needed as u64);
    assert_eq!(native.samples, expected);
    assert_eq!(pump.counters.dropped.load(Ordering::Relaxed), 0);
    assert_eq!(pump.counters.silent.load(Ordering::Relaxed), 0);
}

#[test]
fn master_pending_payload_never_renders_a_second_demand_under_backpressure_f32() {
    backpressured_master::<f32>();
}

#[test]
fn master_pending_payload_never_renders_a_second_demand_under_backpressure_f64() {
    backpressured_master::<f64>();
}
