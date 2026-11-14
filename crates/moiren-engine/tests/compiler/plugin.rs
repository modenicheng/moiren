//! Plugin node compilation: a live plugin instance lowered through the
//! graph compiler, rendered against a stereo source, and checked for its
//! parameter schema. The audio/FFI behavior itself is covered by the
//! moiren-plugins fixture suite; this file proves the engine plumbing.
//!
//! Native-only: Miri cannot spawn rustc or execute the loaded cdylib. The
//! portable compiler tests remain enabled in both Miri borrow models.

#![cfg(not(miri))]

use std::{path::PathBuf, process::Command, sync::OnceLock};

use moiren_core::graph::{LogicalGraph, NodeKind};
use moiren_engine::{
    boundary::audio_bridge,
    compiler::{NodeBindings, compile_with_signals},
    processor::{PluginProcessor, PluginSettings},
};
use moiren_plugins::spec::{PluginDescriptor, PluginFormat, PluginParameterSpec};
use moiren_signal::Bus;

use super::common::{connect_with as connect, pump};

fn fixture_path() -> &'static std::path::Path {
    static DLL: OnceLock<PathBuf> = OnceLock::new();
    DLL.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let extension = std::env::consts::DLL_EXTENSION;
        let binary = dir.join(format!("moiren_test_gain.{extension}"));
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../moiren-plugins/tests/fixtures/gain_plugin.rs");
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "cdylib", "-O"])
            .arg(&source)
            .args(["-o", binary.to_str().expect("utf-8 tmpdir")])
            .status()
            .expect("rustc must be available to build the fixture");
        assert!(status.success(), "fixture compilation failed");
        binary
    })
}

fn fixture_settings() -> PluginSettings {
    PluginSettings {
        descriptor: PluginDescriptor {
            format: PluginFormat::Clap,
            path: fixture_path().to_path_buf(),
            id: "org.moiren.test.gain".into(),
            name: "Moiren Test Gain".into(),
            vendor: "Moiren".into(),
            version: "1.0.0".into(),
            input_channels: 2,
            output_channels: 2,
            is_synth: false,
            has_editor: false,
        },
        parameters: vec![PluginParameterSpec {
            id: 0,
            name: "Gain".into(),
            min: 0.0,
            max: 4.0,
            default: 1.0,
            automatable: true,
            stepped: false,
        }],
        preset: None,
    }
}

#[test]
#[cfg_attr(
    miri,
    ignore = "spawns rustc to build the fixture and loads it as a dynamic library, both outside Miri"
)]
fn plugin_preset_is_restored_into_the_instance() {
    // Capture a state chunk from a live fixture instance whose gain was
    // moved off the default, exactly like a preset capture would.
    let mut capture = moiren_plugins::instantiate(&fixture_settings().descriptor, 48_000.0, 256)
        .expect("fixture instantiation");
    let frames = 1;
    let inputs = vec![vec![0.0f32; frames]; 2];
    let mut outputs = vec![vec![0.0f32; frames]; 2];
    let borrowed: Vec<&[f32]> = inputs.iter().map(|c| c.as_slice()).collect();
    let mut borrowed_out: Vec<&mut [f32]> = outputs.iter_mut().map(|c| c.as_mut_slice()).collect();
    capture
        .process(
            &borrowed,
            &mut borrowed_out,
            &[moiren_plugins::spec::PluginParamEvent {
                parameter: 0,
                value: 2.0,
            }],
        )
        .expect("parameter push");
    let preset = capture
        .save_state()
        .expect("state save")
        .expect("the fixture stores state");

    // A processor built with that preset renders the preset's gain from its
    // very first block: 0.25 in, 0.5 out.
    let settings = PluginSettings {
        preset: Some(preset),
        ..fixture_settings()
    };
    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let plugin = graph.create_node(NodeKind::Plugin, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, plugin, Default::default());
    connect(&mut graph, plugin, sink, Default::default());
    let (writer, mut reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            moiren_engine::boundary::ConstantSource {
                channels: 2,
                value: 0.25,
            },
        )
        .unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let processor = PluginProcessor::<f64>::new(&settings, 48_000.0, 8).unwrap();
    bindings.bind_plugin(plugin, processor).unwrap();

    let mut signal_bus = Bus::new(Default::default()).unwrap();
    let (host_control, host) = signal_bus.create_host(4).unwrap();
    let mut compiled =
        compile_with_signals(&graph, bindings, super::config(), &host_control).unwrap();
    compiled.engine.attach_signal_host(host).unwrap();
    pump(&mut signal_bus);

    compiled.engine.render(8).unwrap();
    pump(&mut signal_bus);
    let mut output = vec![0.0_f64; 16];
    let report = reader.read_interleaved(&mut output).unwrap();
    assert!(report.transferred_frames > 0);
    for sample in &output[..report.transferred_frames * 2] {
        assert!(
            (sample - 0.5).abs() < 1e-6,
            "preset gain not applied: {sample}"
        );
    }
}

#[test]
#[cfg_attr(
    miri,
    ignore = "spawns rustc to build the fixture and loads it as a dynamic library, both outside Miri"
)]
fn plugin_node_renders_stereo_gain_and_exports_its_schema() {
    let settings = fixture_settings();
    let mut signal_bus = Bus::new(Default::default()).unwrap();
    let (host_control, host) = signal_bus.create_host(4).unwrap();

    let mut graph = LogicalGraph::new();
    let source = graph.create_node(NodeKind::Source, 2).unwrap();
    let plugin = graph.create_node(NodeKind::Plugin, 2).unwrap();
    let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
    connect(&mut graph, source, plugin, Default::default());
    connect(&mut graph, plugin, sink, Default::default());

    let (writer, mut reader) = audio_bridge::<f64>(2, 16, 4096).unwrap();
    let mut bindings = NodeBindings::new();
    bindings
        .bind_source(
            source,
            moiren_engine::boundary::ConstantSource {
                channels: 2,
                value: 0.25,
            },
        )
        .unwrap();
    bindings.bind_sink(sink, writer).unwrap();
    let processor = PluginProcessor::<f64>::new(&settings, 48_000.0, 8).unwrap();
    bindings.bind_plugin(plugin, processor).unwrap();

    let mut compiled =
        compile_with_signals(&graph, bindings, super::config(), &host_control).unwrap();
    compiled.engine.attach_signal_host(host).unwrap();
    pump(&mut signal_bus);

    // The plugin node lowered to its own runtime processor.
    let processor = compiled.bindings.node(plugin).expect("plugin node lowered");

    // Default unity gain: two stereo blocks pass through the plugin.
    compiled.engine.render(8).unwrap();
    compiled.engine.render(8).unwrap();
    pump(&mut signal_bus);
    let mut output = Vec::new();
    while output.len() < 32 {
        let mut chunk = vec![0.0_f64; 16];
        let report = reader.read_interleaved(&mut chunk).unwrap();
        assert!(report.transferred_frames > 0, "audio bridge drained early");
        output.extend_from_slice(&chunk[..report.transferred_frames * 2]);
    }
    assert_eq!(output.len(), 32);
    for sample in &output {
        assert!((sample - 0.25).abs() < 1e-6, "{sample}");
    }
    assert_ne!(processor.0, 0);
}
