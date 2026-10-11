use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
use moiren_engine::{
    boundary::ConstantSource,
    compiler::{CompileConfig, NodeBindings, compile_with_signals},
    processor::CompressorSettings,
    runtime::EngineConfig,
};
use moiren_signal::{Bus, PumpBudget};

fn main() -> anyhow::Result<()> {
    let mut bus = Bus::new(Default::default())?;
    let topic = bus.register_latest_value("example/compressor/reduction_db", 0.0_f32)?;
    let mut subscriber = bus.subscribe(&topic, Default::default())?;
    let (control, host) = bus.create_host(1)?;
    let publisher = control.prepare(&topic, control.new_binding()?)?;
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2)?;
    let compressor = graph.create_node(NodeKind::Compressor, 2)?;
    graph.connect(
        graph.get_node(source)?.outputs()[0].id(),
        graph.get_node(compressor)?.inputs()[0].id(),
        SendParams::default(),
    )?;
    let mut bindings = NodeBindings::<f64>::new();
    bindings.bind_source(
        source,
        ConstantSource {
            channels: 2,
            value: 1.0,
        },
    )?;
    bindings.bind_compressor_with_signal(
        compressor,
        CompressorSettings {
            threshold_db: -12.0,
            ratio: 4.0,
            attack_ms: 2.0,
            knee_db: 0.0,
            ..Default::default()
        },
        publisher,
    )?;
    let mut compiled = compile_with_signals(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 64,
                max_events_per_block: 16,
            },
            audio_byte_budget: 65536,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 16,
            control_horizon_frames: 48_000,
        },
        &control,
    )?;
    compiled
        .engine
        .attach_signal_host(host)
        .map_err(|failure| anyhow::anyhow!("signal host: {:?}", failure.error))?;
    for block in 0..8 {
        compiled.engine.render(64)?;
        bus.pump(PumpBudget {
            topic_checks: 1024,
            deliveries: 128,
            copy_bytes: 65536,
        });
        let read = subscriber
            .read_latest()
            .expect("subscribed compressor publishes after DSP");
        assert!(read.value.is_finite() && read.value > 0.0);
        println!(
            "block={block} reduction_db={:.6} sequence={}",
            read.value, read.meta.sequence
        );
    }
    Ok(())
}
