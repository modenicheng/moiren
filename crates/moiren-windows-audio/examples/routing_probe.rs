//! Finite, muted multi-endpoint validation. No system settings or PCM files change.
use anyhow::{Context, bail};
#[cfg(any(windows, test))]
use moiren_core::graph::{NodeId, NodeKind, SendParams, edit::connect_to_new_bus_input};
#[cfg(any(windows, test))]
use moiren_engine::{
    boundary::ClockRole,
    host::{EngineProject, NodeConfig},
    node::EndpointSelector,
};
#[cfg(any(windows, test))]
use moiren_windows_audio::engine_backend::{physical_input, physical_output, tone_input};
use moiren_windows_audio::{engine_backend::NativeLatencyProfile, latency::TelemetryMode};

#[cfg(any(windows, test))]
#[path = "routing_probe/monitor.rs"]
mod monitor;

#[derive(Debug)]
struct Options {
    outputs: Vec<String>,
    inputs: Vec<String>,
    seconds: u64,
    list: bool,
    help: bool,
    profile: NativeLatencyProfile,
    telemetry: TelemetryMode,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            outputs: Vec::new(),
            inputs: Vec::new(),
            seconds: 5,
            list: false,
            help: false,
            profile: NativeLatencyProfile::default(),
            telemetry: TelemetryMode::Summary,
        }
    }
}

fn options(arguments: impl IntoIterator<Item = String>) -> anyhow::Result<Options> {
    let mut result = Options::default();
    let mut duration_flag = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => result.outputs.push(
                arguments
                    .next()
                    .context("--output requires an endpoint ID")?,
            ),
            "--input" => result.inputs.push(
                arguments
                    .next()
                    .context("--input requires an endpoint ID")?,
            ),
            "--seconds" | "--soak-seconds" => {
                if let Some(previous) = duration_flag {
                    bail!(
                        "duration may be specified only once: {previous} conflicts with {argument}"
                    );
                }
                duration_flag = Some(argument.clone());
                let bounds = if argument == "--seconds" {
                    1..=60
                } else {
                    60..=1800
                };
                result.seconds = arguments
                    .next()
                    .with_context(|| format!("{argument} requires {bounds:?} seconds"))?
                    .parse()
                    .with_context(|| format!("{argument} requires {bounds:?} seconds"))?;
                if !bounds.contains(&result.seconds) {
                    bail!("{argument} requires {bounds:?} seconds");
                }
            }
            "--trace" => result.telemetry = TelemetryMode::Trace,
            "--list" => result.list = true,
            "--profile" => {
                result.profile = match arguments.next().as_deref() {
                    Some("compatible") => NativeLatencyProfile::Compatible,
                    Some("balanced") => NativeLatencyProfile::Balanced,
                    Some("low") => NativeLatencyProfile::Low,
                    _ => bail!("--profile requires compatible, balanced or low"),
                };
            }
            "--help" | "-h" => {
                println!(
                    "routing_probe --list\nrouting_probe --output <master ID> [--output <follower ID> ...] [--input <capture ID> ...] [--seconds 1..60 | --soak-seconds 60..1800] [--profile compatible|balanced|low] [--trace]\nDefault duration is 5 seconds; duration flags cannot be repeated or combined. Telemetry defaults to Summary; --trace selects Trace.\nAll outputs include an independent zero-gain guard. Without --input, mixes two software tones."
                );
                result.help = true;
                return Ok(result);
            }
            _ => bail!("unknown option {argument}"),
        }
    }
    if !result.list && result.outputs.is_empty() {
        bail!("select at least one explicit --output; use --list first");
    }
    if result.outputs.len() > 16
        || result.inputs.len() > 16
        || result
            .inputs
            .iter()
            .chain(&result.outputs)
            .any(|id| id.trim().is_empty() || id.contains('\0'))
    {
        bail!("requires at most 16 inputs/outputs and nonempty endpoint IDs");
    }
    let unique: std::collections::BTreeSet<_> = result.outputs.iter().collect();
    if unique.len() != result.outputs.len() {
        bail!("each output endpoint must be distinct");
    }
    Ok(result)
}

#[cfg(any(windows, test))]
fn build(options: &Options) -> anyhow::Result<(EngineProject, NodeId)> {
    let mut project = EngineProject::default();
    let bus = project.graph.create_node(NodeKind::Bus, 2)?;
    project.nodes.insert(bus, NodeConfig::Bus);
    let sources = if options.inputs.is_empty() {
        vec![tone_input(440.0)?, tone_input(880.0)?]
    } else {
        options
            .inputs
            .iter()
            .map(|id| physical_input(endpoint(id)))
            .collect()
    };
    for configuration in sources {
        let input = project.graph.create_node(NodeKind::Source, 2)?;
        project
            .nodes
            .insert(input, NodeConfig::Input(configuration));
        let port = project.graph.get_node(input)?.outputs()[0].id();
        connect_to_new_bus_input(&mut project.graph, port, bus, SendParams::default())?;
    }
    let edited = project.graph.create_node(NodeKind::Gain, 2)?;
    project
        .nodes
        .insert(edited, NodeConfig::Gain { initial: 1.0 });
    connect(&mut project, bus, edited)?;
    for (index, endpoint) in options.outputs.iter().enumerate() {
        let guard = project.graph.create_node(NodeKind::Gain, 2)?;
        project
            .nodes
            .insert(guard, NodeConfig::Gain { initial: 0.0 });
        connect(&mut project, edited, guard)?;
        let sink = project.graph.create_node(NodeKind::Sink, 2)?;
        let mut output = physical_output(self::endpoint(endpoint));
        if index > 0 {
            output.device.clock_role = ClockRole::Follower;
        }
        project.nodes.insert(sink, NodeConfig::Output(output));
        connect(&mut project, guard, sink)?;
    }
    project.validate()?;
    Ok((project, edited))
}

#[cfg(any(windows, test))]
fn connect(project: &mut EngineProject, from: NodeId, to: NodeId) -> anyhow::Result<()> {
    project.graph.connect(
        project.graph.get_node(from)?.outputs()[0].id(),
        project.graph.get_node(to)?.inputs()[0].id(),
        SendParams::default(),
    )?;
    Ok(())
}

#[cfg(any(windows, test))]
fn endpoint(id: &str) -> EndpointSelector {
    EndpointSelector::PinnedEndpoint {
        endpoint_id: id.into(),
        stable_id: None,
    }
}

#[cfg(windows)]
struct SharedBackend(std::sync::Arc<moiren_windows_audio::engine_backend::WindowsBackend>);

#[cfg(windows)]
impl moiren_engine::host::EngineBackend<f32> for SharedBackend {
    fn prepare(
        &self,
        request: moiren_engine::host::PrepareRequest<'_>,
        cancellation: &moiren_engine::host::CancellationToken,
    ) -> Result<moiren_engine::host::PreparedBackend<f32>, moiren_engine::host::BackendError> {
        moiren_engine::host::EngineBackend::prepare(self.0.as_ref(), request, cancellation)
    }
    fn poll_resource(&self) -> Option<moiren_engine::host::ResourceEvent> {
        moiren_engine::host::EngineBackend::poll_resource(self.0.as_ref())
    }
}

#[cfg(windows)]
fn run(options: Options) -> anyhow::Result<()> {
    use moiren_engine::host::{EditOutcome, EngineHost, HostConfig};
    use moiren_windows_audio::engine_backend::{WindowsBackend, WindowsBackendConfig};
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    if options.help {
        return Ok(());
    }
    if options.list {
        println!(
            "{}",
            serde_json::to_string_pretty(&moiren_windows_audio::catalog_snapshot()?)?
        );
        return Ok(());
    }
    let (project, edited) = build(&options)?;
    let mut samples = monitor::Samples::new(options.seconds)?;
    let backend = Arc::new(WindowsBackend::new(WindowsBackendConfig {
        latency_profile: options.profile,
        telemetry: options.telemetry,
        ..WindowsBackendConfig::default()
    })?);
    let host = EngineHost::<f32>::open(
        HostConfig::default(),
        project,
        SharedBackend(backend.clone()),
    )?;
    let observer = host.observer();
    let observations = backend.observations();
    let mut initial = None;
    let mut active = observer.snapshot();
    let mut started = None;
    let mut compatible_edit_activated = false;
    let mut health_polls = 0_u64;

    // Keep every post-open failure inside this result so shutdown and JSON
    // reporting still happen for startup, edit, and mid-run failures.
    let run_result = (|| -> anyhow::Result<()> {
        let startup_deadline = Instant::now() + Duration::from_secs(20);
        loop {
            active = observer.snapshot();
            health_polls += 1;
            if monitor::startup_ready(&active)? {
                break;
            }
            if Instant::now() >= startup_deadline {
                bail!("startup timed out: {active:?}");
            }
            thread::sleep(monitor::POLL_INTERVAL);
        }
        initial = Some(active.clone());
        let clock = Instant::now();
        started = Some(clock);
        let mut health = monitor::Health::new(&active)?;
        samples.push(Duration::ZERO, observations.snapshot())?;
        monitor::check_native(
            samples
                .values
                .last()
                .and_then(|sample| sample.observation.as_ref()),
            active.owner_generation,
        )?;
        let receipt = host.control().edit(move |project| {
            project
                .nodes
                .insert(edited, NodeConfig::Gain { initial: 0.5 });
            Ok(())
        })?;
        let requested = Duration::from_secs(options.seconds);
        let mut next_progress = Duration::from_secs(10);
        loop {
            active = observer.snapshot();
            health_polls += 1;
            let elapsed = clock.elapsed();
            health.check(&active, elapsed)?;
            if !compatible_edit_activated {
                let outcome = receipt.outcome();
                match outcome {
                    EditOutcome::Activated { .. } => compatible_edit_activated = true,
                    outcome if outcome.is_terminal() => {
                        bail!("compatible edit did not activate: {outcome:?}");
                    }
                    _ if elapsed >= Duration::from_secs(10) => {
                        bail!("compatible edit activation timed out: {outcome:?}");
                    }
                    _ => {}
                }
            }
            if samples.due(elapsed) {
                samples.push(elapsed, observations.snapshot())?;
                monitor::check_native(
                    samples
                        .values
                        .last()
                        .and_then(|sample| sample.observation.as_ref()),
                    active.owner_generation,
                )?;
            }
            if elapsed >= next_progress {
                eprintln!(
                    "routing_probe: {:.1}/{} active seconds, {} samples, owner {}, plan {:?}",
                    elapsed.as_secs_f64(),
                    options.seconds,
                    samples.values.len(),
                    active.owner_generation,
                    active.plan_revision,
                );
                next_progress = Duration::from_secs((elapsed.as_secs() / 10 + 1) * 10);
            }
            if elapsed >= requested {
                if !compatible_edit_activated {
                    bail!("compatible edit did not activate within the requested duration");
                }
                break;
            }
            thread::sleep(monitor::POLL_INTERVAL.min(requested.saturating_sub(elapsed)));
        }
        Ok(())
    })();

    // Freeze active duration before synchronous native teardown and reporting.
    let elapsed = started.map_or(Duration::ZERO, |clock| clock.elapsed());
    let native_observation = observations.snapshot();
    let terminal_result = samples
        .push(elapsed, native_observation.clone())
        .and_then(|()| {
            if let Some(initial) = &initial {
                monitor::check_native(native_observation.as_ref(), initial.owner_generation)?;
            }
            Ok(())
        });
    let same_owner = initial
        .as_ref()
        .is_some_and(|initial| active.owner_generation == initial.owner_generation);
    let plan_replaced = initial.as_ref().is_some_and(|initial| {
        active
            .plan_revision
            .zip(initial.plan_revision)
            .is_some_and(|(current, previous)| current > previous)
    });
    let same_epoch = initial.as_ref().is_some_and(|initial| {
        active
            .timeline
            .zip(initial.timeline)
            .is_some_and(|(a, b)| a.epoch == b.epoch && a.frame > b.frame)
    });
    let run_result = monitor::finish(run_result, terminal_result).and_then(|()| {
        if !same_owner || !same_epoch || !plan_replaced {
            bail!("routing or compatible-update validation failed: {active:?}");
        }
        Ok(())
    });
    drop(host);
    let closed = observer.snapshot();
    let closure_result = monitor::check_closed(&closed);
    let native_raw = backend.last_native_report();
    let native = native_raw
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>);
    let report = serde_json::json!({
        "outputs": options.outputs,
        "inputs": options.inputs,
        "muted": true,
        "profile": options.profile,
        "telemetry": options.telemetry,
        "requested_seconds": options.seconds,
        "elapsed_seconds": elapsed.as_secs_f64(),
        "active_elapsed_seconds": elapsed.as_secs_f64(),
        "samples": samples.values,
        "sample_count": samples.values.len(),
        "sample_count_limit": samples.limit,
        "max_sample_count": monitor::MAX_SAMPLES,
        "sample_count_within_bound": samples.values.len() <= samples.limit
            && samples.values.len() <= monitor::MAX_SAMPLES,
        "health_polls": health_polls,
        "health_poll_interval_ms": monitor::POLL_INTERVAL.as_millis(),
        "native_sample_interval_seconds": 1,
        "observation": native_observation,
        "compatible_edit_activated": compatible_edit_activated,
        "plan_revision_initial": initial.as_ref().and_then(|initial| initial.plan_revision),
        "plan_revision_final": active.plan_revision,
        "plan_replaced": plan_replaced,
        "same_owner": same_owner,
        "same_epoch_and_advancing": same_epoch,
        "healthy": run_result.is_ok(),
        "closed": format!("{:?}", closed.host),
        "clean_shutdown": closure_result.is_ok(),
        "fault": active.fault.as_ref().or(closed.fault.as_ref())
            .map(|error| format!("{error:?}")),
        "cleanup_error": closed.cleanup_error.as_ref().map(ToString::to_string),
        "error": run_result.as_ref().err().map(|error| format!("{error:#}")),
        "closure_error": closure_result.as_ref().err().map(|error| format!("{error:#}")),
        "host_snapshot_initial": initial.as_ref().map(monitor::snapshot_json),
        "host_snapshot_final": monitor::snapshot_json(&active),
        "host_snapshot_closed": monitor::snapshot_json(&closed),
        "native": native.as_ref().and_then(|parsed| parsed.as_ref().ok()),
        "native_report_parse_error": native.as_ref()
            .and_then(|parsed| parsed.as_ref().err()).map(ToString::to_string),
        "native_report_raw": native.as_ref().filter(|parsed| parsed.is_err())
            .and(native_raw.as_ref()),
        "physical_end_to_end_latency_ms": null
    });
    let report_result = (|| -> anyhow::Result<()> {
        use std::io::Write;
        let mut output = std::io::stdout().lock();
        serde_json::to_writer_pretty(&mut output, &report)?;
        writeln!(output)?;
        Ok(())
    })();
    monitor::finish(monitor::finish(run_result, closure_result), report_result)
}

fn main() -> anyhow::Result<()> {
    let selected = options(std::env::args().skip(1))?;
    #[cfg(windows)]
    return run(selected);
    #[cfg(not(windows))]
    {
        let _ = selected;
        bail!("native routing_probe requires Windows");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_explicit_duration_bounds_are_finite() {
        assert_eq!(
            options(["--output", "a"].map(String::from))
                .unwrap()
                .seconds,
            5
        );
        for (flag, seconds) in [
            ("--seconds", "1"),
            ("--seconds", "60"),
            ("--soak-seconds", "60"),
            ("--soak-seconds", "1800"),
        ] {
            let selected = options(["--output", "a", flag, seconds].map(String::from)).unwrap();
            assert_eq!(selected.seconds, seconds.parse::<u64>().unwrap());
        }
    }

    #[test]
    fn rejects_conflicting_and_duplicate_duration_flags_in_either_order() {
        for (first, second) in [
            ("--seconds", "--soak-seconds"),
            ("--soak-seconds", "--seconds"),
            ("--seconds", "--seconds"),
            ("--soak-seconds", "--soak-seconds"),
        ] {
            let error = options(["--output", "a", first, "60", second, "60"].map(String::from))
                .unwrap_err()
                .to_string();
            assert!(error.contains("duration"), "{error}");
        }
    }

    #[test]
    fn rejects_invalid_missing_and_out_of_range_durations() {
        for (flag, values) in [
            (
                "--seconds",
                ["0", "61", "-1", "later", "18446744073709551616"],
            ),
            ("--soak-seconds", ["0", "59", "1801", "-1", "later"]),
        ] {
            for value in values {
                assert!(
                    options(["--output", "a", flag, value].map(String::from)).is_err(),
                    "{flag} {value}"
                );
            }
            assert!(options(["--output", "a", flag].map(String::from)).is_err());
        }
    }

    #[test]
    fn trace_is_opt_in_and_can_precede_or_follow_duration() {
        assert_eq!(
            options(["--output", "a"].map(String::from))
                .unwrap()
                .telemetry,
            TelemetryMode::Summary
        );
        for args in [
            vec!["--output", "a", "--trace"],
            vec!["--output", "a", "--trace", "--soak-seconds", "60"],
            vec!["--output", "a", "--seconds", "1", "--trace"],
        ] {
            assert_eq!(
                options(args.into_iter().map(String::from))
                    .unwrap()
                    .telemetry,
                TelemetryMode::Trace
            );
        }
    }

    #[test]
    fn list_help_and_explicit_endpoint_selection_are_preserved() {
        assert!(options(["--list"].map(String::from)).unwrap().list);
        assert!(options(["--help"].map(String::from)).unwrap().help);
        assert!(options(["-h"].map(String::from)).unwrap().help);
        assert!(options(std::iter::empty()).is_err());
        assert!(options(["--input", "capture"].map(String::from)).is_err());
    }

    #[test]
    fn rejects_duplicate_output_and_unbounded_duration_before_opening_hardware() {
        for args in [
            vec!["--output", "a", "--output", "a"],
            vec!["--output", "a", "--seconds", "0"],
            vec!["--output", "a", "--seconds", "61"],
        ] {
            assert!(options(args.into_iter().map(String::from)).is_err());
        }
    }

    #[test]
    fn multi_output_project_has_distinct_muted_guards_and_one_master() {
        let selected = options(["--output", "a", "--output", "b"].map(String::from)).unwrap();
        let (project, edited) = build(&selected).unwrap();
        assert!(matches!(
            project.nodes[&edited],
            NodeConfig::Gain { initial: 1.0 }
        ));
        let mut masters = 0;
        let mut sinks = 0;
        let mut guards = std::collections::BTreeSet::new();
        for (&id, config) in &project.nodes {
            if let NodeConfig::Output(output) = config {
                sinks += 1;
                masters += usize::from(output.device.clock_role == ClockRole::Master);
                let input = project.graph.get_node(id).unwrap().inputs()[0].id();
                let edge = project
                    .graph
                    .edges()
                    .iter()
                    .find(|edge| edge.dst_port() == input)
                    .expect("every output sink input is connected");
                let guard = edge.src();
                assert!(guards.insert(guard));
                assert!(matches!(
                    project.nodes[&guard],
                    NodeConfig::Gain { initial: 0.0 }
                ));
            }
        }
        assert_eq!((sinks, masters), (2, 1));
    }
}
