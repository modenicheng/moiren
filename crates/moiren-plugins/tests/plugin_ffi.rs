//! End-to-end FFI tests against the self-built gain fixture: one cdylib
//! exporting `clap_entry`, `VSTPluginMain` and `GetPluginFactory`. The
//! fixture is an independent ABI transcription, so a passing suite means
//! both sides agree byte-for-byte on every struct layout and vtable order.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use moiren_plugins::{
    PluginInstance, instantiate, scan,
    spec::{PluginDescriptor, PluginFormat, PluginParamEvent},
};

const SAMPLE_RATE: f64 = 48_000.0;
const BLOCK: u32 = 256;

/// Build the fixture once per test binary and expose it under three
/// format-specific names so directory scanning can discover each one.
fn fixture_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let extension = std::env::consts::DLL_EXTENSION;
        let binary = dir.join(format!("moiren_test_gain.{extension}"));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gain_plugin.rs");
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "cdylib", "-O"])
            .arg(&source)
            .args(["-o", binary.to_str().expect("utf-8 tmpdir")])
            .status()
            .expect("rustc must be available to build the fixture");
        assert!(status.success(), "fixture compilation failed");
        for alias in ["moiren_test_gain.clap", "moiren_test_gain.vst3"] {
            let alias = dir.join(alias);
            if !alias.exists() {
                std::fs::copy(&binary, &alias).expect("alias copy");
            }
        }
        dir
    })
}

fn descriptor(format: PluginFormat, id: &str) -> PluginDescriptor {
    let extension = std::env::consts::DLL_EXTENSION;
    PluginDescriptor {
        format,
        path: fixture_dir().join(format!("moiren_test_gain.{extension}")),
        id: id.to_string(),
        name: "Moiren Test Gain".into(),
        vendor: "Moiren".into(),
        version: "1.0.0".into(),
        input_channels: 2,
        output_channels: 2,
        is_synth: false,
        has_editor: false,
    }
}

fn run_block(instance: &mut Box<dyn PluginInstance>, gain_events: &[PluginParamEvent]) -> [f32; 4] {
    let left = [0.25f32, -0.5, 0.75, -1.0];
    let right = [1.0f32, 0.5, -0.25, 0.125];
    let mut out_left = [0f32; 4];
    let mut out_right = [0f32; 4];
    let inputs = [left.as_slice(), right.as_slice()];
    let mut outputs = [out_left.as_mut_slice(), out_right.as_mut_slice()];
    instance
        .process(&inputs, &mut outputs, gain_events)
        .expect("process succeeds");
    [out_left[1], out_right[1], out_left[3], out_right[3]]
}

fn assert_close(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "{context}: {actual} vs {expected}"
    );
}

#[test]
fn scan_discovers_all_three_formats() {
    let outcome = scan::scan_directories(&[fixture_dir().to_path_buf()]);
    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    let formats: Vec<_> = outcome
        .descriptors
        .iter()
        .map(|d| (d.format, d.id.clone(), d.name.clone()))
        .collect();
    let has = |format: PluginFormat| formats.iter().any(|(f, _, _)| *f == format);
    assert!(has(PluginFormat::Clap), "{formats:?}");
    assert!(has(PluginFormat::Vst3), "{formats:?}");
    assert!(has(PluginFormat::Vst2), "{formats:?}");
    for (_, _, name) in &formats {
        assert_eq!(name, "Moiren Test Gain");
    }
}

#[test]
fn clap_lifecycle_process_and_state() {
    let source = descriptor(PluginFormat::Clap, "org.moiren.test.gain");
    let mut instance = instantiate(&source, SAMPLE_RATE, BLOCK).expect("clap instantiate");

    let live = instance.descriptor();
    assert_eq!(live.input_channels, 2);
    assert_eq!(live.output_channels, 2);
    assert_eq!(instance.latency_frames(), 3);
    let params = instance.parameters();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "Gain");
    assert_eq!(params[0].min, 0.0);
    assert_eq!(params[0].max, 4.0);
    assert_eq!(params[0].default, 1.0);

    // Default gain multiplies by one.
    for (sample, expected) in run_block(&mut instance, &[])
        .iter()
        .zip([-0.5, 0.5, -1.0, 0.125])
    {
        assert_close(*sample, expected, "clap default gain");
    }
    // Real-unit event of 2.0 doubles the signal.
    let events = [PluginParamEvent {
        parameter: 0,
        value: 2.0,
    }];
    for (sample, expected) in run_block(&mut instance, &events)
        .iter()
        .zip([-1.0, 1.0, -2.0, 0.25])
    {
        assert_close(*sample, expected, "clap param event");
    }

    // State round-trip into a fresh instance restores the automated gain.
    let state = instance.save_state().expect("state present");
    let state = state.expect("clap state chunk");
    let mut restored = instantiate(&source, SAMPLE_RATE, BLOCK).expect("second instance");
    restored.load_state(&state).expect("state load");
    for (sample, expected) in run_block(&mut restored, &[])
        .iter()
        .zip([-1.0, 1.0, -2.0, 0.25])
    {
        assert_close(*sample, expected, "clap restored gain");
    }
}

#[test]
fn vst2_lifecycle_process_and_chunk() {
    let source = descriptor(PluginFormat::Vst2, "4D746731");
    let mut instance = instantiate(&source, SAMPLE_RATE, BLOCK).expect("vst2 instantiate");

    let live = instance.descriptor();
    assert_eq!(live.input_channels, 2);
    assert_eq!(live.output_channels, 2);
    assert_eq!(instance.latency_frames(), 3);
    let params = instance.parameters();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "Gain");
    assert_eq!(params[0].min, 0.0);
    assert_eq!(params[0].max, 1.0);
    // Default comes from getParameter right after open: 0.25 normalized.
    assert_close(params[0].default as f32, 0.25, "vst2 default");

    // 0.25 normalized over the fixture's 4.0 span is unity gain.
    for (sample, expected) in run_block(&mut instance, &[])
        .iter()
        .zip([-0.5, 0.5, -1.0, 0.125])
    {
        assert_close(*sample, expected, "vst2 default gain");
    }
    // 0.5 normalized doubles the signal.
    let events = [PluginParamEvent {
        parameter: 0,
        value: 0.5,
    }];
    for (sample, expected) in run_block(&mut instance, &events)
        .iter()
        .zip([-1.0, 1.0, -2.0, 0.25])
    {
        assert_close(*sample, expected, "vst2 param event");
    }

    let chunk = instance.save_state().expect("state present");
    let chunk = chunk.expect("vst2 chunk");
    let mut restored = instantiate(&source, SAMPLE_RATE, BLOCK).expect("second instance");
    restored.load_state(&chunk).expect("chunk load");
    for (sample, expected) in run_block(&mut restored, &[])
        .iter()
        .zip([-1.0, 1.0, -2.0, 0.25])
    {
        assert_close(*sample, expected, "vst2 restored gain");
    }
}

#[test]
fn vst3_lifecycle_process_and_state() {
    let scanned = scan::scan_directories(&[fixture_dir().to_path_buf()]);
    let found = scanned
        .descriptors
        .iter()
        .find(|d| d.format == PluginFormat::Vst3)
        .expect("vst3 descriptor scanned");
    let source = descriptor(PluginFormat::Vst3, &found.id);
    let mut instance = instantiate(&source, SAMPLE_RATE, BLOCK).expect("vst3 instantiate");

    let live = instance.descriptor();
    assert_eq!(live.input_channels, 2);
    assert_eq!(live.output_channels, 2);
    assert_eq!(instance.latency_frames(), 3);
    let params = instance.parameters();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "Gain");
    assert_close(params[0].default as f32, 0.25, "vst3 default");

    // 0.25 normalized over the fixture's 4.0 span is unity gain.
    for (sample, expected) in run_block(&mut instance, &[])
        .iter()
        .zip([-0.5, 0.5, -1.0, 0.125])
    {
        assert_close(*sample, expected, "vst3 default gain");
    }
    // 0.5 normalized doubles the signal through IParameterChanges.
    let events = [PluginParamEvent {
        parameter: 0,
        value: 0.5,
    }];
    for (sample, expected) in run_block(&mut instance, &events)
        .iter()
        .zip([-1.0, 1.0, -2.0, 0.25])
    {
        assert_close(*sample, expected, "vst3 param event");
    }

    let state = instance.save_state().expect("state present");
    let state = state.expect("vst3 state blob");
    let mut restored = instantiate(&source, SAMPLE_RATE, BLOCK).expect("second instance");
    restored.load_state(&state).expect("state load");
    for (sample, expected) in run_block(&mut restored, &[])
        .iter()
        .zip([-1.0, 1.0, -2.0, 0.25])
    {
        assert_close(*sample, expected, "vst3 restored gain");
    }
}

#[test]
fn instantiate_rejects_mismatched_plugin_id() {
    let source = descriptor(PluginFormat::Clap, "org.unknown.plugin");
    let error = match instantiate(&source, SAMPLE_RATE, BLOCK) {
        Err(error) => error,
        Ok(instance) => panic!("unknown id must fail, got {:?}", instance.descriptor()),
    };
    assert!(error.to_string().contains("not in factory"));
}
