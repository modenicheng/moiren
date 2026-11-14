//! The plugin node's realtime path allocates nothing: the per-call pointer
//! tables and the parameter-event buffer are preallocated at construction
//! and rebuilt by overwrite. Driven through the real gain fixture (a cdylib
//! compiled at test time) so the tables cross an actual FFI boundary.

use super::{
    allocation::track_allocations,
    support::{EPOCH, REV},
};
use moiren_core::graph::*;
use moiren_engine::{
    boundary::{ConstantSource, audio_bridge},
    compiler::*,
    processor::{PluginProcessor, PluginSettings},
};
use std::{path::PathBuf, process::Command, sync::OnceLock};

const SAMPLE_RATE: f64 = 48_000.0;
/// Must match `common::engine_config().max_block_frames`: the instance
/// activates for exactly the engine's block bound.
const MAX_BLOCK: usize = 8;

/// The gain fixture compiled once per test binary into the tmpdir as a
/// single-file VST3 module — which also exercises the file-root scan path.
fn fixture_settings() -> &'static PluginSettings {
    static SETTINGS: OnceLock<PluginSettings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let binary = dir.join("moiren_test_gain.vst3");
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("moiren-plugins")
            .join("tests")
            .join("fixtures")
            .join("gain_plugin.rs");
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "cdylib", "-O"])
            .arg(&source)
            .args(["-o", binary.to_str().expect("utf-8 tmpdir")])
            .status()
            .expect("rustc must be available to build the fixture");
        assert!(status.success(), "fixture compilation failed");
        let outcome = moiren_plugins::scan::scan_directories(&[dir]);
        assert!(
            outcome.failures.is_empty(),
            "fixture scan failures: {:?}",
            outcome.failures
        );
        let descriptor = outcome
            .descriptors
            .into_iter()
            .find(|descriptor| descriptor.format == moiren_plugins::spec::PluginFormat::Vst3)
            .expect("scanned vst3 fixture");
        // The engine automation table derives from the probe's snapshot, as
        // in production.
        let probe = moiren_plugins::instantiate(&descriptor, SAMPLE_RATE, MAX_BLOCK as u32)
            .expect("fixture instantiation");
        PluginSettings {
            descriptor,
            parameters: probe.parameters().to_vec(),
        }
    })
}

#[test]
fn plugin_node_renders_without_allocation_or_deallocation() {
    fn check<S: moiren_engine::sample::ProcessingSample>() {
        let settings = fixture_settings();
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2).unwrap();
        let plugin = graph.create_node(NodeKind::Plugin, 2).unwrap();
        let sink = graph.create_node(NodeKind::Sink, 2).unwrap();
        graph
            .connect(
                graph.get_node(source).unwrap().outputs()[0].id(),
                graph.get_node(plugin).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
        graph
            .connect(
                graph.get_node(plugin).unwrap().outputs()[0].id(),
                graph.get_node(sink).unwrap().inputs()[0].id(),
                SendParams::default(),
            )
            .unwrap();
        let (writer, mut reader) = audio_bridge::<S>(2, 32, 4096).unwrap();
        let mut bindings = NodeBindings::new();
        bindings
            .bind_source(
                source,
                ConstantSource {
                    channels: 2,
                    value: 0.25,
                },
            )
            .unwrap();
        bindings.bind_sink(sink, writer).unwrap();
        bindings
            .bind_plugin(
                plugin,
                PluginProcessor::<S>::new(settings, SAMPLE_RATE, MAX_BLOCK).unwrap(),
            )
            .unwrap();
        let mut compiled = compile(
            &graph,
            bindings,
            CompileConfig {
                timeline_epoch: EPOCH,
                ..super::common::compile_config(REV)
            },
        )
        .unwrap();
        let (results, counts) = track_allocations(|| {
            [
                compiled.engine.render(3),
                compiled.engine.render(5),
                compiled.engine.render(MAX_BLOCK),
            ]
        });
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(counts, (0, 0));
        let mut samples = [S::ZERO; 32];
        let read = reader
            .read_interleaved(&mut samples)
            .unwrap()
            .transferred_frames;
        assert_eq!(read, 3 + 5 + MAX_BLOCK);
        // The fixture's default gain is 1.0, so every output frame is the
        // untouched 0.25 constant — the tables carried real audio through.
        for sample in samples[..read].iter() {
            assert!((sample.to_f64() - 0.25).abs() < 1e-6);
        }
    }
    check::<f32>();
    check::<f64>();
}
