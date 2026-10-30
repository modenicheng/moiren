//! Supervised physical IO/Bus route; a request never certifies physical latency.
use super::{
    render_request::PhysicalRenderRequest,
    validation_policy::{ValidationMode, ValidationPolicy},
};
use anyhow::{Context, Result, bail, ensure};
use moiren_engine::{audio_config::BUFFER_SIZES, node::DeviceMode};
use moiren_windows_audio::latency::{LatencyBudgetPolicy, RenderQueuePolicy, TelemetryMode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(any(windows, test))]
#[path = "physical/topology.rs"]
mod topology;

#[cfg(any(windows, test))]
#[path = "physical/session.rs"]
mod session;

#[cfg(windows)]
#[path = "physical/native.rs"]
mod native;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Options {
    pub input_endpoint_id: String,
    pub output_endpoint_id: String,
    pub recorder_config: PathBuf,
    pub artifact_dir: PathBuf,
    pub seconds: u64,
    pub capture: PhysicalRenderRequest,
    pub render: PhysicalRenderRequest,
    pub input_budget: LatencyBudgetPolicy,
    pub render_queue: RenderQueuePolicy,
    pub engine_block_frames: usize,
    pub validation: ValidationPolicy,
    pub telemetry: TelemetryMode,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecorderConfig {
    pub schema_version: u32,
    pub input_endpoint_id: String,
    pub ring_frames: usize,
    pub max_pcm_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_packets: u64,
}

impl RecorderConfig {
    fn validate(&self, options: &Options) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported physical recorder config schema"
        );
        endpoint(&self.input_endpoint_id)?;
        ensure!(
            !self
                .input_endpoint_id
                .eq_ignore_ascii_case(&options.input_endpoint_id)
                && !self
                    .input_endpoint_id
                    .eq_ignore_ascii_case(&options.output_endpoint_id),
            "independent recorder must differ from graph input and output"
        );
        ensure!(
            self.ring_frames > 0
                && self
                    .ring_frames
                    .checked_mul(size_of::<moiren_windows_audio::capture::PcmEvidenceFrame>())
                    .is_some_and(|bytes| bytes <= 8 * 1024 * 1024),
            "physical recorder ring exceeds nonzero 8MiB budget"
        );
        ensure!(
            (1..=3 * 1024 * 1024 * 1024).contains(&self.max_pcm_bytes),
            "physical recorder PCM cap must be 1..3GiB"
        );
        ensure!(
            (1..=256 * 1024 * 1024).contains(&self.max_metadata_bytes),
            "physical recorder metadata cap must be 1..256MiB"
        );
        ensure!(
            (1..=1_000_000).contains(&self.max_packets),
            "physical recorder packet cap must be 1..1000000"
        );
        Ok(())
    }

    #[cfg(windows)]
    fn measurement_config(
        &self,
        options: &Options,
    ) -> crate::measurement::session::MeasurementConfig {
        crate::measurement::session::MeasurementConfig {
            input_endpoint_id: self.input_endpoint_id.clone(),
            capture_mode: Default::default(),
            max_active_duration: std::time::Duration::from_secs(options.seconds + 14),
            ring_frames: self.ring_frames,
            max_pcm_bytes: self.max_pcm_bytes,
            max_metadata_bytes: self.max_metadata_bytes,
            max_packets: self.max_packets,
        }
    }
}

fn endpoint(id: &str) -> Result<()> {
    ensure!(
        !id.trim().is_empty() && !id.contains('\0'),
        "physical endpoint ID must be nonempty without NUL"
    );
    Ok(())
}

fn parse(arguments: Vec<String>) -> Result<Options> {
    let mut values = BTreeMap::new();
    let mut dry_run = false;
    let mut iter = arguments.into_iter();
    while let Some(flag) = iter.next() {
        if flag == "--dry-run" {
            ensure!(!dry_run, "duplicate --dry-run");
            dry_run = true;
            continue;
        }
        ensure!(
            [
                "--input",
                "--output",
                "--recorder-config",
                "--artifact-dir",
                "--seconds",
                "--capture-mode",
                "--capture-period-frames",
                "--render-mode",
                "--render-period-frames",
                "--input-target-frames",
                "--input-capacity-frames",
                "--input-jitter-frames",
                "--render-target-frames",
                "--wake-margin-frames",
                "--callback-budget-frames",
                "--engine-block-frames",
                "--validation-mode",
                "--jitter-tolerance-ms"
            ]
            .contains(&flag.as_str()),
            "unknown physical IO option {flag}"
        );
        let value = iter
            .next()
            .with_context(|| format!("{flag} requires a value"))?;
        ensure!(
            !value.starts_with("--"),
            "{flag} requires a value, found option {value}"
        );
        ensure!(
            values.insert(flag.clone(), value).is_none(),
            "duplicate {flag}"
        );
    }
    let required = |key: &str| {
        values
            .get(key)
            .map(String::as_str)
            .with_context(|| format!("missing {key}"))
    };
    let integer = |key: &str, default: u32| -> Result<u32> {
        values.get(key).map_or(Ok(default), |value| {
            value.parse().with_context(|| format!("invalid {key}"))
        })
    };
    let request = |mode: &str, period: &str| -> Result<PhysicalRenderRequest> {
        let mode = values
            .get(mode)
            .map(|value| match value.as_str() {
                "shared" => Ok(DeviceMode::Shared),
                "exclusive" => Ok(DeviceMode::Exclusive),
                _ => bail!("{mode} must be shared or exclusive"),
            })
            .transpose()?;
        let period_frames = values
            .get(period)
            .map(|value| -> Result<u32> {
                let n = value.parse()?;
                ensure!(
                    BUFFER_SIZES.contains(&n),
                    "{period} must be a power of two in 64..4096"
                );
                Ok(n)
            })
            .transpose()?;
        Ok(PhysicalRenderRequest {
            mode,
            period_frames,
        })
    };
    let capture = request("--capture-mode", "--capture-period-frames")?;
    let render = request("--render-mode", "--render-period-frames")?;
    let render_queue = match values.get("--render-target-frames").map(String::as_str) {
        None | Some("capacity") => {
            ensure!(
                !values.contains_key("--wake-margin-frames")
                    && !values.contains_key("--callback-budget-frames"),
                "render margins require an explicit numeric target"
            );
            RenderQueuePolicy::FullCapacity
        }
        Some(frames) => RenderQueuePolicy::Target {
            frames: frames.parse()?,
            wake_margin_frames: integer("--wake-margin-frames", 96)?,
            callback_budget_frames: integer("--callback-budget-frames", 48)?,
        },
    };
    render_queue.validate()?;
    render.validate(render_queue)?;
    capture.validate(RenderQueuePolicy::FullCapacity)?;
    let input_budget = if values.contains_key("--input-target-frames") {
        let input_capacity_frames = integer("--input-capacity-frames", 8192)?;
        let input_target_frames = integer("--input-target-frames", 0)?;
        ensure!(
            (4..=262_144).contains(&input_capacity_frames)
                && (2..input_capacity_frames).contains(&input_target_frames),
            "invalid physical input target/capacity"
        );
        let input_jitter_frames = integer("--input-jitter-frames", 96)?;
        // Even the smallest positive native period/demand and FIR lookahead
        // need more than jitter alone; native-dependent admission still runs.
        ensure!(
            input_jitter_frames
                .checked_add(34)
                .is_some_and(|minimum| minimum <= input_target_frames),
            "physical input target cannot cover jitter and minimum transfer/lookahead"
        );
        LatencyBudgetPolicy::Custom {
            input_capacity_frames,
            input_target_frames,
            input_jitter_frames,
            render: render_queue,
        }
    } else {
        ensure!(
            !values.contains_key("--input-capacity-frames")
                && !values.contains_key("--input-jitter-frames"),
            "physical input capacity/jitter requires an explicit target"
        );
        LatencyBudgetPolicy::Compatible
    };
    let validation = ValidationPolicy {
        mode: match values.get("--validation-mode").map(String::as_str) {
            None | Some("latency-sensitive") => ValidationMode::LatencySensitive,
            Some("correctness") => ValidationMode::Correctness,
            _ => bail!("invalid physical validation mode"),
        },
        jitter_tolerance_ms: u64::from(integer("--jitter-tolerance-ms", 3000)?),
    };
    validation.validate()?;
    let seconds = u64::from(integer("--seconds", 0)?);
    ensure!(
        (1..=600).contains(&seconds),
        "physical IO --seconds must be 1..600"
    );
    let engine_block_frames = integer("--engine-block-frames", 256)? as usize;
    ensure!(
        (1..=4096).contains(&engine_block_frames),
        "engine block must be 1..4096"
    );
    let input_endpoint_id = required("--input")?.to_owned();
    let output_endpoint_id = required("--output")?.to_owned();
    endpoint(&input_endpoint_id)?;
    endpoint(&output_endpoint_id)?;
    ensure!(
        !input_endpoint_id.eq_ignore_ascii_case(&output_endpoint_id),
        "physical input and output must differ"
    );
    for key in ["--recorder-config", "--artifact-dir"] {
        ensure!(
            !required(key)?.trim().is_empty() && !required(key)?.contains('\0'),
            "invalid {key}"
        );
    }
    Ok(Options {
        input_endpoint_id,
        output_endpoint_id,
        recorder_config: required("--recorder-config")?.into(),
        artifact_dir: required("--artifact-dir")?.into(),
        seconds,
        capture,
        render,
        input_budget,
        render_queue,
        engine_block_frames,
        validation,
        telemetry: TelemetryMode::Summary,
        dry_run,
    })
}

fn read_config(options: &Options) -> Result<(Vec<u8>, RecorderConfig)> {
    let file = File::open(&options.recorder_config).context("open physical recorder config")?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 64 * 1024,
        "physical recorder config exceeds 64KiB"
    );
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 64 * 1024,
        "physical recorder config grew beyond 64KiB"
    );
    let config: RecorderConfig = serde_json::from_slice(&bytes)?;
    config.validate(options)?;
    Ok((bytes, config))
}

fn new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("reserve {}", path.display()))?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

const HELP: &str = "live-physical --input ID --output ID --recorder-config FILE --artifact-dir NEWDIR --seconds 1..600
Exact physical Input -> Bus -> physical Output with a distinct bounded independent recorder.
External signal and measurement wiring must be supplied separately; no source launch, calibration or latency claim.
Optional --capture-mode shared|exclusive --capture-period-frames N and --render-mode shared|exclusive --render-period-frames N.
N is 64..4096 power of two; omitted settings preserve native default periods. Explicit periods refuse fallback.
Optional --input-target-frames N --input-capacity-frames N --input-jitter-frames N;
--render-target-frames N|capacity --wake-margin-frames N --callback-budget-frames N; --engine-block-frames N.
--validation-mode latency-sensitive|correctness --jitter-tolerance-ms 0..3000; --dry-run.
Correctness records timing/underruns without early quality exit; identity/storage/native/lifecycle errors remain fatal.
Exit0 means operational correctness collection completed, never qualification; exit1 incomplete/unqualified; exit2 invalid input/IO.";

pub(crate) fn run(arguments: Vec<String>) -> Result<i32> {
    run_with(arguments, |options, config, directory| {
        #[cfg(windows)]
        {
            native::run_native(options, config, directory)
        }
        #[cfg(not(windows))]
        {
            let _ = (options, config, directory);
            json!({"collection_complete":false,"cleanup_complete":false,"quality_passed":false,"failure":"physical IO native collection requires Windows"})
        }
    })
}

fn run_with(
    arguments: Vec<String>,
    collector: impl FnOnce(&Options, &RecorderConfig, &Path) -> serde_json::Value,
) -> Result<i32> {
    if arguments.is_empty() || arguments == ["--help"] {
        println!("{HELP}");
        return Ok(0);
    }
    let options = parse(arguments)?;
    let (bytes, config) = read_config(&options)?;
    ensure!(
        !options.artifact_dir.exists(),
        "physical IO artifact directory already exists"
    );
    let root = super::output_path_key(&options.artifact_dir)?;
    let source = super::output_path_key(&options.recorder_config)?;
    ensure!(
        !source.starts_with(&root),
        "recorder config aliases physical IO outputs"
    );
    fs::create_dir(&options.artifact_dir).context("reserve new physical IO artifact directory")?;
    let mut report = json!({"schema_version":1,"mode":"live_physical","qualified":false,"status":"unqualified",
        "external_source":true,"independent_reference_verified":false,"measurement_boundary_verified":false,
        "requested":options,"recorder_config":config,"collection_complete":false,"cleanup_complete":false,
        "quality_passed":false,"native":null,"failure":null,
        "gaps":["External signal identity and waveform are not independently verified.","Recorder is not verified to contain a common pre-route reference and processed output.","No calibrated physical boundary, sample-age provenance or latency distribution is established."]});
    let collected = (|| -> Result<()> {
        new_file(&options.artifact_dir.join("recorder-config.json"), &bytes)?;
        if !options.dry_run {
            {
                let native = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    collector(&options, &config, &options.artifact_dir)
                }))
                .map_err(|_| {
                    anyhow::anyhow!("physical native collector panicked; cleanup unverified")
                })?;
                for key in [
                    "collection_complete",
                    "cleanup_complete",
                    "quality_passed",
                    "failure",
                ] {
                    report[key] = native.get(key).cloned().unwrap_or(serde_json::Value::Null);
                }
                report["native"] = native;
            }
        }
        Ok(())
    })();
    if let Err(error) = collected {
        report["failure"] = format!("{error:#}").into();
    }
    let data = serde_json::to_vec_pretty(&report)?;
    // Attempt both receipts after retirement/preparation failure, not only the
    // first successful write. Existing files are never truncated.
    let manifest = new_file(&options.artifact_dir.join("manifest.json"), &data);
    let receipt = new_file(&options.artifact_dir.join("report.json"), &data);
    manifest?;
    receipt?;
    Ok(
        if options.validation.is_correctness()
            && report["collection_complete"] == true
            && report["cleanup_complete"] == true
            && report["failure"].is_null()
        {
            0
        } else {
            1
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moiren-physical-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn args(root: &Path) -> Vec<String> {
        fs::write(
            root.join("config.json"),
            serde_json::to_vec(&json!({
                "schema_version":1, "input_endpoint_id":"independent recorder", "ring_frames":1024,
                "max_pcm_bytes":1048576, "max_metadata_bytes":100000, "max_packets":1000
            }))
            .unwrap(),
        )
        .unwrap();
        vec![
            "--input".into(),
            "physical capture".into(),
            "--output".into(),
            "physical render".into(),
            "--recorder-config".into(),
            root.join("config.json").to_str().unwrap().into(),
            "--artifact-dir".into(),
            root.join("run").to_str().unwrap().into(),
            "--seconds".into(),
            "2".into(),
            "--dry-run".into(),
        ]
    }

    #[test]
    fn physical_io_runner_uses_actual_collector_and_retains_failure_and_partial_native_reports() {
        for failure in [false, true] {
            let root = root();
            let mut arguments = args(&root);
            arguments.retain(|arg| arg != "--dry-run");
            arguments.extend(["--validation-mode", "correctness"].map(str::to_owned));
            let mut calls = 0;
            let result = run_with(arguments, |options, config, directory| {
                calls += 1;
                assert_eq!(options.input_endpoint_id, "physical capture");
                assert_eq!(config.input_endpoint_id, "independent recorder");
                assert!(directory.join("recorder-config.json").exists());
                json!({"collection_complete":!failure,"cleanup_complete":true,"quality_passed":false,"failure":if failure {Some("native preparation refused")} else {None},"partial":{"timing":"retained"}})
            }).unwrap();
            assert_eq!(calls, 1);
            assert_eq!(result, if failure { 1 } else { 0 });
            let value: Value =
                serde_json::from_slice(&fs::read(root.join("run/report.json")).unwrap()).unwrap();
            assert_eq!(value["native"]["partial"]["timing"], "retained");
            assert_eq!(value["quality_passed"], false);
            assert_eq!(value["qualified"], false);
            if failure {
                assert_eq!(value["failure"], "native preparation refused");
            }
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn physical_io_dry_run_cannot_invoke_native_collector() {
        let root = root();
        let arguments = args(&root);
        assert_eq!(
            run_with(arguments, |_, _, _| panic!("dry-run touched native audio")).unwrap(),
            1
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_collector_panic_retains_failure_receipts_without_claiming_cleanup() {
        let root = root();
        let mut arguments = args(&root);
        arguments.retain(|arg| arg != "--dry-run");
        arguments.extend(["--validation-mode", "correctness"].map(str::to_owned));
        let result = std::panic::catch_unwind(|| {
            run_with(arguments, |_, _, _| panic!("collector escaped supervision"))
        });
        assert!(
            result.is_ok(),
            "collector panic escaped the artifact runner"
        );
        assert_eq!(result.unwrap().unwrap(), 1);
        for name in ["manifest.json", "report.json"] {
            let value: Value =
                serde_json::from_slice(&fs::read(root.join("run").join(name)).unwrap()).unwrap();
            assert_eq!(value["collection_complete"], false);
            assert_eq!(value["cleanup_complete"], false);
            assert_eq!(value["qualified"], false);
            assert!(value["failure"].as_str().unwrap().contains("panicked"));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_topology_is_exact_bus_route_with_separate_direction_requests() {
        use moiren_engine::{
            boundary::ClockRole,
            host::NodeConfig,
            node::{DeviceMode, EndpointSelector, InputSource, OutputTarget},
        };
        let root = root();
        let mut arguments = args(&root);
        arguments.extend(
            [
                "--capture-mode",
                "exclusive",
                "--capture-period-frames",
                "128",
                "--render-period-frames",
                "256",
                "--engine-block-frames",
                "64",
            ]
            .map(str::to_owned),
        );
        let options = parse(arguments).unwrap();
        let prepared =
            topology::prepare(&options).expect("physical graph must prepare without native IO");
        assert_eq!(prepared.project.graph.nodes().len(), 3);
        assert_eq!(prepared.project.nodes.len(), 3);
        assert_eq!(prepared.project.graph.edges().len(), 2);
        assert_eq!(prepared.project.processing.engine.processing_sr, 48_000.0);
        assert!(prepared.project.strips.is_empty());
        assert!(prepared.project.parameters.is_empty());
        assert_eq!(prepared.project.processing.engine.max_block_frames, 64);
        let NodeConfig::Input(input) = &prepared.project.nodes[&prepared.input] else {
            panic!("physical source missing")
        };
        assert_eq!(input.channels, 2);
        assert_eq!(input.device.clock_role, ClockRole::Follower);
        assert_eq!(input.device.mode, DeviceMode::Exclusive);
        assert_eq!(input.device.period_frames, Some(128));
        assert_eq!(
            input.source,
            InputSource::Physical(EndpointSelector::PinnedEndpoint {
                endpoint_id: "physical capture".into(),
                stable_id: None
            })
        );
        let NodeConfig::Output(output) = &prepared.project.nodes[&prepared.output] else {
            panic!("physical sink missing")
        };
        assert_eq!(output.channels, 2);
        assert_eq!(output.device.clock_role, ClockRole::Master);
        assert_eq!(output.device.mode, DeviceMode::Shared);
        assert_eq!(output.device.period_frames, Some(256));
        assert_eq!(
            output.target,
            OutputTarget::Physical(EndpointSelector::PinnedEndpoint {
                endpoint_id: "physical render".into(),
                stable_id: None
            })
        );
        assert_eq!(
            prepared
                .project
                .nodes
                .values()
                .filter(|node| matches!(node, NodeConfig::Bus))
                .count(),
            1
        );
        let bus = *prepared
            .project
            .nodes
            .iter()
            .find(|(_, config)| matches!(config, NodeConfig::Bus))
            .unwrap()
            .0;
        for (from, to) in [(prepared.input, bus), (bus, prepared.output)] {
            let edge = prepared
                .project
                .graph
                .edges()
                .iter()
                .find(|edge| edge.src() == from && edge.dst() == to)
                .expect("exact IO -> Bus edge");
            assert_eq!(*edge.params(), Default::default());
        }
        prepared.project.validate().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_cli_dispatches_and_dry_run_retains_exact_intent_without_native_claim() {
        let root = root();
        let mut arguments = args(&root);
        arguments.extend(
            [
                "--capture-mode",
                "exclusive",
                "--capture-period-frames",
                "128",
                "--render-mode",
                "shared",
                "--render-period-frames",
                "256",
            ]
            .map(str::to_owned),
        );
        arguments.insert(0, "live-physical".into());
        assert_eq!(crate::cli::run(arguments).unwrap(), 1);
        for name in ["manifest.json", "report.json"] {
            let value: Value =
                serde_json::from_slice(&fs::read(root.join("run").join(name)).unwrap()).unwrap();
            assert_eq!(value["mode"], "live_physical");
            assert_eq!(value["qualified"], false);
            assert_eq!(value["requested"]["input_endpoint_id"], "physical capture");
            assert_eq!(value["requested"]["output_endpoint_id"], "physical render");
            assert_eq!(value["requested"]["capture"]["mode"], "exclusive");
            assert_eq!(value["requested"]["capture"]["period_frames"], 128);
            assert_eq!(value["requested"]["render"]["mode"], "shared");
            assert_eq!(value["requested"]["render"]["period_frames"], 256);
            assert_eq!(value["external_source"], true);
            assert_eq!(value["independent_reference_verified"], false);
            assert!(value["native"].is_null());
            assert_eq!(value["collection_complete"], false);
        }
        assert_eq!(
            fs::read(root.join("config.json")).unwrap(),
            fs::read(root.join("run/recorder-config.json")).unwrap()
        );
        assert!(!root.join("run/measurement.f32le").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_invalid_controls_and_conflicts_never_create_artifacts() {
        for extra in [
            vec!["--input", "another"],
            vec!["--output", "another"],
            vec!["--seconds", "1"],
            vec!["--capture-mode", "invalid"],
            vec!["--capture-mode", "shared", "--capture-mode", "exclusive"],
            vec!["--capture-period-frames", "0"],
            vec!["--capture-period-frames", "480"],
            vec!["--render-period-frames", "8192"],
            vec![
                "--render-mode",
                "exclusive",
                "--render-target-frames",
                "256",
            ],
            vec!["--input-capacity-frames", "8192"],
            vec!["--input-target-frames", "8192"],
            vec!["--engine-block-frames", "0"],
            vec!["--validation-mode", "unbounded"],
            vec!["--jitter-tolerance-ms", "3001"],
            vec!["--source-pid", "42"],
        ] {
            let root = root();
            let mut arguments = args(&root);
            arguments.extend(extra.into_iter().map(str::to_owned));
            assert!(run(arguments).is_err());
            assert!(!root.join("run").exists());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn physical_io_requires_distinct_explicit_endpoints_and_bounded_recorder_config() {
        for replacement in [
            "",
            "physical capture",
            "PHYSICAL CAPTURE",
            "physical render",
            "x\0y",
        ] {
            let root = root();
            let arguments = args(&root);
            let mut config: Value =
                serde_json::from_slice(&fs::read(root.join("config.json")).unwrap()).unwrap();
            config["input_endpoint_id"] = replacement.into();
            fs::write(
                root.join("config.json"),
                serde_json::to_vec(&config).unwrap(),
            )
            .unwrap();
            assert!(run(arguments).is_err());
            assert!(!root.join("run").exists());
            let _ = fs::remove_dir_all(root);
        }
        for (field, value) in [
            ("schema_version", json!(2)),
            ("ring_frames", json!(0)),
            ("max_pcm_bytes", json!(0)),
            ("max_metadata_bytes", json!(268435457u64)),
            ("max_packets", json!(1000001)),
            ("unexpected", json!(1)),
        ] {
            let root = root();
            let arguments = args(&root);
            let mut config: Value =
                serde_json::from_slice(&fs::read(root.join("config.json")).unwrap()).unwrap();
            config[field] = value;
            fs::write(
                root.join("config.json"),
                serde_json::to_vec(&config).unwrap(),
            )
            .unwrap();
            assert!(run(arguments).is_err());
            assert!(!root.join("run").exists());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn physical_io_preserves_existing_directories_and_config_inputs() {
        let root = root();
        let arguments = args(&root);
        fs::create_dir(root.join("run")).unwrap();
        fs::write(root.join("run/report.json"), b"do not replace").unwrap();
        assert!(run(arguments).is_err());
        assert_eq!(
            fs::read(root.join("run/report.json")).unwrap(),
            b"do not replace"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_missing_value_cannot_swallow_dry_run_or_another_flag() {
        let root = root();
        for flag in ["--input", "--output", "--artifact-dir", "--recorder-config"] {
            let mut arguments = args(&root);
            let index = arguments.iter().position(|arg| arg == flag).unwrap();
            arguments.drain(index..index + 2);
            arguments.retain(|arg| arg != "--dry-run");
            arguments.extend([flag.to_owned(), "--dry-run".into()]);
            assert!(parse(arguments).is_err(), "{flag} swallowed --dry-run");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_statically_impossible_input_jitter_is_rejected_before_artifacts() {
        let root = root();
        for jitter in ["4294967295", "1024", "991"] {
            let mut arguments = args(&root);
            arguments.extend(
                [
                    "--input-target-frames",
                    "1024",
                    "--input-capacity-frames",
                    "4096",
                    "--input-jitter-frames",
                    jitter,
                ]
                .map(str::to_owned),
            );
            assert!(run(arguments).is_err());
            assert!(!root.join("run").exists());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_custom_input_and_render_budgets_are_separate_and_retained() {
        let root = root();
        let mut arguments = args(&root);
        arguments.extend(
            [
                "--input-target-frames",
                "1024",
                "--input-capacity-frames",
                "4096",
                "--input-jitter-frames",
                "96",
                "--render-target-frames",
                "256",
                "--wake-margin-frames",
                "32",
                "--callback-budget-frames",
                "16",
            ]
            .map(str::to_owned),
        );
        let options = parse(arguments).unwrap();
        let queue = RenderQueuePolicy::Target {
            frames: 256,
            wake_margin_frames: 32,
            callback_budget_frames: 16,
        };
        assert_eq!(options.render_queue, queue);
        assert_eq!(
            options.input_budget,
            LatencyBudgetPolicy::Custom {
                input_capacity_frames: 4096,
                input_target_frames: 1024,
                input_jitter_frames: 96,
                render: queue
            }
        );
        assert_eq!(options.capture, PhysicalRenderRequest::default());
        assert_eq!(options.render, PhysicalRenderRequest::default());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_config_input_cannot_alias_an_output_or_exceed_byte_cap() {
        let root = root();
        let mut arguments = args(&root);
        // The existing config must not become the output directory itself.
        let index = arguments
            .iter()
            .position(|arg| arg == "--artifact-dir")
            .unwrap();
        arguments[index + 1] = root.join("config.json").to_str().unwrap().into();
        let original = fs::read(root.join("config.json")).unwrap();
        assert!(run(arguments).is_err());
        assert_eq!(fs::read(root.join("config.json")).unwrap(), original);
        let arguments = args(&root);
        fs::write(root.join("config.json"), vec![b' '; 64 * 1024 + 1]).unwrap();
        assert!(run(arguments).is_err());
        assert!(!root.join("run").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn physical_io_omission_and_correctness_grace_do_not_change_buffer_defaults() {
        let root = root();
        let mut arguments = args(&root);
        arguments.extend(
            [
                "--validation-mode",
                "correctness",
                "--jitter-tolerance-ms",
                "3000",
            ]
            .map(str::to_owned),
        );
        assert_eq!(run(arguments).unwrap(), 1);
        let value: Value =
            serde_json::from_slice(&fs::read(root.join("run/report.json")).unwrap()).unwrap();
        assert_eq!(
            value["requested"]["capture"],
            json!({"mode":null,"period_frames":null})
        );
        assert_eq!(
            value["requested"]["render"],
            json!({"mode":null,"period_frames":null})
        );
        assert_eq!(
            value["requested"]["validation"]["jitter_tolerance_ms"],
            3000
        );
        assert_eq!(value["requested"]["input_budget"], "compatible");
        assert_eq!(value["collection_complete"], false);
        let _ = fs::remove_dir_all(root);
    }
}
