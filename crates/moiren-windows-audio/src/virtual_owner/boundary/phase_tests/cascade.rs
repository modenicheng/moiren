//! Coupled compiled Bus, not two unrelated local-phase controls.
use super::*;
use crate::resample::SrcCursor;
use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::{
    compiler::{CompileConfig, NodeBindings, compile},
    runtime::EngineConfig,
};
use std::{collections::VecDeque, fs::OpenOptions, io::Write, path::Path, sync::Mutex};

const BLOCK: usize = 32;
const CALLBACKS: usize = 512;

#[derive(Default)]
struct Captured {
    bus: Vec<[f64; 2]>,
    playback_ppm: f64,
    microphone_ppm: f64,
}

struct SourceCheck(VirtualPlaybackSource, Arc<Mutex<Captured>>);
impl<S: ProcessingSample> RtAudioSource<S> for SourceCheck {
    fn channel_count(&self) -> usize {
        self.0.channels
    }
    fn read(&mut self, ctx: &ProcessContext, output: AudioBlockMut<'_, S>) -> BoundaryReport {
        let report = self.0.read(ctx, output);
        let mut capture = self.1.lock().unwrap();
        capture.playback_ppm = self.0.drift.as_ref().unwrap().servo.correction();
        report
    }
}

struct SinkCheck(VirtualMicrophoneSink, Arc<Mutex<Captured>>);
impl<S: ProcessingSample> RtAudioSink<S> for SinkCheck {
    fn channel_count(&self) -> usize {
        self.0.channels
    }
    fn write(&mut self, ctx: &ProcessContext, input: AudioBlock<'_, S>) -> BoundaryReport {
        let mut capture = self.1.lock().unwrap();
        capture.bus.clear();
        for frame in 0..ctx.frames {
            capture.bus.push([
                input.channel(0)[frame].to_f64(),
                if self.0.channels == 2 {
                    input.channel(1)[frame].to_f64()
                } else {
                    0.0
                },
            ]);
        }
        let report = self.0.write(ctx, input);
        capture.microphone_ppm = self.0.drift.as_ref().unwrap().servo.correction();
        report
    }
}

fn sample(index: usize) -> [f32; 2] {
    let bits = (index as u32)
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    let value = ((bits >> 8) as f32 / 16_777_216.0 - 0.5) * 0.5;
    [value, -value]
}

fn json_line(output: &mut impl Write, value: &serde_json::Value) -> usize {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    output.write_all(&bytes).unwrap();
    bytes.len()
}

fn save_json(path: &Path, value: &serde_json::Value) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    json_line(&mut file, value);
    file.flush().unwrap();
}

fn run_rate<S: ProcessingSample>(rate: u32, directory: Option<&Path>) {
    for channels in [1, 2] {
        run_case::<S>(rate, channels, directory);
    }
}

fn run_case<S: ProcessingSample>(rate: u32, channels: usize, directory: Option<&Path>) {
    let mut graph = LogicalGraph::new();
    let source_node = graph.create_node(NodeKind::Source, channels).unwrap();
    let bus = graph.create_node(NodeKind::Bus, channels).unwrap();
    let sink_node = graph.create_node(NodeKind::Sink, channels).unwrap();
    let bus_input = graph.add_input_port(bus, channels).unwrap();
    graph
        .connect(
            graph.get_node(source_node).unwrap().outputs()[0].id(),
            bus_input,
            Default::default(),
        )
        .unwrap();
    graph
        .connect(
            graph.get_node(bus).unwrap().outputs()[0].id(),
            graph.get_node(sink_node).unwrap().inputs()[0].id(),
            Default::default(),
        )
        .unwrap();
    assert_eq!(graph.nodes().len(), 3);
    assert_eq!(graph.edges().len(), 2);
    let (mut producer, reader) = audio_bridge(2, 1024, BUDGET).unwrap();
    let (writer, mut consumer) = audio_bridge(2, 1024, BUDGET).unwrap();
    let playback = Arc::new(BoundaryProbe::default());
    let microphone = Arc::new(BoundaryProbe::default());
    let source = VirtualPlaybackSource::new(reader, channels, BLOCK, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(playback.clone());
    let sink = VirtualMicrophoneSink::new(writer, channels, BLOCK, rate, BUDGET)
        .unwrap()
        .with_clock_drift_target(384)
        .unwrap()
        .with_probe(microphone.clone());
    let captured = Arc::new(Mutex::new(Captured::default()));
    let mut bindings = NodeBindings::<S>::new();
    bindings
        .bind_source(source_node, SourceCheck(source, captured.clone()))
        .unwrap();
    bindings
        .bind_sink(sink_node, SinkCheck(sink, captured.clone()))
        .unwrap();
    let config = CompileConfig {
        engine: EngineConfig {
            processing_sr: f64::from(rate),
            max_block_frames: BLOCK,
            max_events_per_block: 8,
        },
        audio_byte_budget: BUDGET,
        plan_revision: 13,
        timeline_epoch: 1,
        control_capacity: 8,
        control_horizon_frames: 48_000,
    };
    let mut compiled = compile(&graph, bindings, config).unwrap();
    assert_eq!(compiled.stats.mix_operation_count, 1);
    let prepared = serde_json::json!({
        "schema_version":2,"prepared_graph_revision":41,"prepared_plan_revision":config.plan_revision,
        "source":format!("{source_node:?}"),"bus":format!("{bus:?}"),"sink":format!("{sink_node:?}"),
        "source_endpoint":"01010101010101010101010101010101",
        "sink_endpoint":"02020202020202020202020202020202",
        "channels":channels,"engine_sample_rate":rate,"engine_block_frames":BLOCK,
    });
    // Test-only archive assembly from the exact graph/bindings above, not a
    // native collector or imported-data provenance constructor.
    let case_dir = directory.map(|root| {
        let path = root.join(format!("{rate}-{}-{channels}", std::any::type_name::<S>()));
        std::fs::create_dir(&path).unwrap();
        path
    });
    let mut chunks: Box<dyn Write> = match &case_dir {
        Some(path) => Box::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path.join("chunks.jsonl"))
                .unwrap(),
        ),
        None => Box::new(std::io::sink()),
    };
    let mut playback_oracle = StereoResampler::new(48_000, rate, 4096).unwrap();
    let mut microphone_oracle = StereoResampler::new(rate, 48_000, 4096).unwrap();
    let mut playback_centers: VecDeque<(u64, SrcCursor)> = VecDeque::new();
    let (mut submitted, mut consumed, mut byte_count, mut changing_frames) = (0, 0, 0, 0);
    let mut queries = Vec::new();
    let mut oracle_queries = Vec::new();
    for index in 0..CALLBACKS {
        let target: usize = if index < 192 { 768 } else { 384 };
        let queued = producer.capacity_frames() - producer.available_frames();
        let fill = target.saturating_sub(queued);
        let input: Vec<_> = (submitted..submitted + fill).flat_map(sample).collect();
        producer.write_interleaved_exact(&input).unwrap();
        submitted += fill;
        compiled.engine.render(BLOCK).unwrap();
        let source = playback.src_checkpoint().unwrap();
        let sink = microphone.src_checkpoint().unwrap();
        for point in [source, sink] {
            assert_eq!(point.execution.unwrap().plan_revision, 13);
            assert_eq!(point.execution.unwrap().control_generation, 0);
            assert_eq!(point.timeline_start, (index * BLOCK) as u64);
            assert_eq!(point.xruns, 0);
            assert_eq!(point.transferred_frames, BLOCK as u64);
        }
        let capture = captured.lock().unwrap();
        playback_oracle.set_correction_ppm(capture.playback_ppm);
        assert_eq!(Some(playback_oracle.cursor().step), source.processing_step);
        let pushed =
            (source.after.unwrap().input_frames - source.before.unwrap().input_frames) as usize;
        for frame in consumed..consumed + pushed {
            assert!(playback_oracle.push_frame(sample(frame).map(f64::from)));
        }
        consumed += pushed;
        for (offset, captured_frame) in capture.bus.iter().enumerate().take(BLOCK) {
            let cursor = playback_oracle.cursor();
            let expected = playback_oracle.next_frame().unwrap();
            for (channel, value) in expected.iter().enumerate().take(channels) {
                let expected = if offset < source.muted_frames as usize {
                    0.0
                } else {
                    S::from_f64(*value).to_f64()
                };
                assert_eq!(
                    captured_frame[channel], expected,
                    "compiled unity Bus must preserve every sample"
                );
            }
            playback_centers.push_back(((index * BLOCK + offset) as u64, cursor));
            if playback_centers.len() > 4 * BLOCK {
                playback_centers.pop_front();
            }
        }
        assert_eq!(playback_oracle.cursor(), source.after.unwrap());
        microphone_oracle.set_correction_ppm(capture.microphone_ppm);
        assert_eq!(Some(microphone_oracle.cursor().step), sink.processing_step);
        for frame in &capture.bus {
            assert!(microphone_oracle.push_frame(*frame));
        }
        let produced =
            (sink.after.unwrap().output_frames - sink.before.unwrap().output_frames) as usize;
        let mut actual = vec![0.0_f32; produced * 2];
        assert_eq!(
            consumer
                .read_interleaved(&mut actual)
                .unwrap()
                .transferred_frames,
            produced
        );
        for offset in 0..produced {
            let cursor = microphone_oracle.cursor();
            let expected = microphone_oracle.next_frame().unwrap().map(|v| v as f32);
            assert_eq!(&actual[offset * 2..offset * 2 + 2], &expected);
            changing_frames += usize::from(expected[0] != 0.0 && expected[0] != expected[1]);
            if [15, 255, 511].contains(&index) && [0, produced / 2, produced - 1].contains(&offset)
            {
                let first_engine = cursor.next_input_frame.checked_sub(15).unwrap();
                let last_engine = cursor.next_input_frame + 16;
                let dependencies: Vec<_> = playback_centers
                    .iter()
                    .filter(|(frame, _)| (first_engine..=last_engine).contains(frame))
                    .collect();
                assert_eq!(dependencies.len(), 32);
                let first = dependencies
                    .iter()
                    .map(|(_, cursor)| cursor.next_input_frame - 15)
                    .min()
                    .unwrap();
                let last = dependencies
                    .iter()
                    .map(|(_, cursor)| cursor.next_input_frame + 16)
                    .max()
                    .unwrap();
                let query = serde_json::json!({"sequence":index + 1,"output_offset":offset});
                queries.push(query.clone());
                let centers: Vec<_> = dependencies
                    .iter()
                    .map(
                        |(frame, cursor)| serde_json::json!({"engine_frame":frame,"cursor":cursor}),
                    )
                    .collect();
                oracle_queries.push(serde_json::json!({"query":query,"microphone_cursor":cursor,
                    "playback_centers":centers,"first_engine_frame":first_engine,
                    "last_engine_frame":last_engine,"first_source_input_frame":first,"last_source_input_frame":last}));
            }
        }
        assert_eq!(microphone_oracle.cursor(), sink.after.unwrap());
        byte_count += json_line(
            &mut chunks,
            &serde_json::json!({"record":"chunk","chunk":{
            "schema_version":1,"owner_generation":7,"sequence":index + 1,"boundaries":[
                {"node":format!("{source_node:?}"),"kind":"virtual_playback","checkpoints":[source]},
                {"node":format!("{sink_node:?}"),"kind":"virtual_microphone","checkpoints":[sink]},
            ]}}),
        );
    }
    assert!(
        changing_frames > CALLBACKS * 8,
        "changing PCM, not a constant/silent route"
    );
    drop(compiled);
    let boundaries: Vec<_> = [(source_node, "virtual_playback", playback), (sink_node, "virtual_microphone", microphone)]
        .into_iter().map(|(node, kind, probe)| {
            let history = probe.src_history().unwrap();
            assert!(history.writer_finished);
            assert_eq!(history.total_callbacks, CALLBACKS as u64);
            serde_json::json!({"node":format!("{node:?}"),"kind":kind,"total_callbacks":CALLBACKS,
                "missing_callbacks":0,"unretained_callbacks":0,"unavailable_polls":0,"writer_finished":true,
                "drained_checkpoints":CALLBACKS,"checkpoints":[]})
        }).collect();
    let trace = serde_json::json!({"schema_version":4,"owner_generation":7,
        "route":{"prepared":prepared,"invalidated":false},"checkpoint_limit":4096,"retained_checkpoints":0,
        "drained_checkpoints":CALLBACKS * 2,"drained_chunks":CALLBACKS,"finalized":true,"history_complete":true,
        "qualified":false,"refusal":null,"boundaries":boundaries});
    byte_count += json_line(
        &mut chunks,
        &serde_json::json!({"record":"final","summary":trace}),
    );
    chunks.flush().unwrap();
    if let Some(path) = case_dir {
        save_json(
            &path.join("summary.json"),
            &serde_json::json!({"schema_version":2,"trace":trace,
            "stream":{"complete":true,"chunks":CALLBACKS,"checkpoints":CALLBACKS*2,"bytes":byte_count,
                "failure":null,"failed_chunks":[]},"chunks_path":"chunks.jsonl","history_complete":true,"qualified":false,
            "limits":{"pending_checkpoints":4096,"queued_chunks":4,"record_bytes":8*1024*1024,
                "stream_bytes":32_u64*1024*1024*1024}}),
        );
        save_json(
            &path.join("queries.json"),
            &serde_json::json!({"schema_version":1,"owner_generation":7,"queries":queries}),
        );
        save_json(
            &path.join("oracle.json"),
            &serde_json::json!({"schema_version":1,"native_execution":false,
            "qualified":false,"physical_delay":"not_evaluated","changing_native_frames":changing_frames,
            "scope":"actual compiled unity Bus; production boundaries; separately fed production FIR PCM/cursor controls sharing only exact applied ppm; test-only archive assembly, not native acquisition or independent FIR kernel",
            "queries":oracle_queries}),
        );
    }
}

#[test]
fn src_cascade_compiled_bus_matches_changing_pcm_and_complete_support_oracles() {
    for rate in [44_100, 48_000, 88_200, 96_000] {
        run_rate::<f32>(rate, None);
        run_rate::<f64>(rate, None);
    }
}

#[test]
#[ignore = "create-new host-only coupled fixture in MOIREN_SRC_CASCADE_FIXTURE; no native devices"]
fn src_cascade_export_compiled_bus_fixture() {
    let root = std::path::PathBuf::from(
        std::env::var_os("MOIREN_SRC_CASCADE_FIXTURE").expect("explicit fixture root"),
    );
    std::fs::create_dir(&root).unwrap();
    for rate in [44_100, 48_000, 88_200, 96_000] {
        run_rate::<f32>(rate, Some(&root));
        run_rate::<f64>(rate, Some(&root));
    }
}

#[path = "cascade/amplitude.rs"]
mod amplitude;
