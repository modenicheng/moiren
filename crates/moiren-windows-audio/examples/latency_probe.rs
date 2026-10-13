use anyhow::{Result, bail};
use moiren_windows_audio::latency::{CallbackObservation, LatencyRecorder, TelemetryMode};
use std::{hint::black_box, time::Instant};

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments == ["--benchmark"] {
        return benchmark();
    }
    if arguments.is_empty() || arguments == ["--help"] {
        println!(
            "latency_probe --list | --benchmark | --input ID --output ID [--seconds 5] [--mode disabled|summary|trace] [--gain 0] [--period-ms default|5] [--render-target-frames capacity|480] [--wake-margin-frames 96] [--callback-budget-frames 48]"
        );
        return Ok(());
    }
    #[cfg(windows)]
    {
        native::run(&arguments)
    }
    #[cfg(not(windows))]
    {
        bail!("Device observations require Windows; --benchmark is portable.")
    }
}

fn benchmark() -> Result<()> {
    #[cfg(windows)]
    let frequency = native::clock_frequency()?;
    for include_clock in [false, true] {
        if include_clock && !cfg!(windows) {
            continue;
        }
        for (mode, trace_full) in [
            (TelemetryMode::Disabled, false),
            (TelemetryMode::Summary, false),
            (TelemetryMode::Trace, false),
            (TelemetryMode::Trace, true),
        ] {
            let callbacks = if mode == TelemetryMode::Trace && !trace_full {
                moiren_windows_audio::latency::TRACE_CAPACITY as u64
            } else {
                100_000
            };
            let mut measurements = Vec::with_capacity(9);
            for _ in 0..9 {
                let mut recorder = LatencyRecorder::new(black_box(mode));
                if trace_full {
                    for _ in 0..moiren_windows_audio::latency::TRACE_CAPACITY {
                        recorder.record(CallbackObservation {
                            wake_qpc_100ns: Some(0),
                            completed_qpc_100ns: Some(0),
                            ..Default::default()
                        });
                    }
                }
                let started = Instant::now();
                for index in 0..callbacks {
                    let mut observation = CallbackObservation {
                        wake_qpc_100ns: Some(index * 100_000),
                        completed_qpc_100ns: Some(index * 100_000 + 1000),
                        demand_frames: 480,
                        padding_before_frames: Some(576),
                        submitted_frames: 480,
                    };
                    if include_clock {
                        observation.wake_qpc_100ns = None;
                        observation.completed_qpc_100ns = None;
                        #[cfg(windows)]
                        if mode != TelemetryMode::Disabled {
                            observation.wake_qpc_100ns = native::clock_now(frequency);
                            observation.completed_qpc_100ns = native::clock_now(frequency);
                        }
                    }
                    recorder.record(black_box(observation));
                }
                measurements.push(started.elapsed().as_nanos() as f64 / callbacks as f64);
                black_box(recorder.snapshot());
            }
            measurements.sort_by(f64::total_cmp);
            println!(
                "{}",
                serde_json::json!({
                    "benchmark": if include_clock { "qpc_and_recorder_excludes_audio" } else { "recorder_only_excludes_qpc_and_audio" },
                    "mode": mode, "callbacks_per_run": callbacks, "trace_full": trace_full, "runs": measurements.len(),
                    "median_nanoseconds_per_callback": measurements[4], "max_nanoseconds_per_callback": measurements[8],
                    "debug_assertions": cfg!(debug_assertions),
                })
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
mod native {
    use super::*;
    use anyhow::Context;
    use moiren_core::graph::{LogicalGraph, NodeKind, SendParams};
    use moiren_engine::{
        boundary::audio_bridge,
        compiler::{CompileConfig, NodeBindings, compile},
        runtime::EngineConfig,
    };
    use moiren_windows_audio::{
        ActivationGate, SessionDuration, StopSignal,
        capture::{CaptureOptions, list_capture_endpoints, prepare_capture_native_with_gate},
        render::{
            DemandRenderer, RenderOptions, list_render_endpoints, prepare_render_native_with_gate,
        },
    };
    use std::time::Duration;

    pub fn clock_frequency() -> Result<u64> {
        let mut frequency = 0;
        unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency) }?;
        u64::try_from(frequency)
            .ok()
            .filter(|value| *value > 0)
            .context("Invalid QPC frequency")
    }
    pub fn clock_now(frequency: u64) -> Option<u64> {
        let mut ticks = 0;
        unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut ticks) }.ok()?;
        moiren_windows_audio::latency::qpc_ticks_to_100ns(u64::try_from(ticks).ok()?, frequency)
    }
    pub fn run(arguments: &[String]) -> Result<()> {
        if arguments == ["--list"] {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "capture": list_capture_endpoints()?, "render": list_render_endpoints()?,
                }))?
            );
            return Ok(());
        }
        let mut input = None;
        let mut output = None;
        let mut seconds = 5u64;
        let mut mode = TelemetryMode::Summary;
        let mut gain = 0.0f64;
        let mut shared_period = moiren_windows_audio::shared_stream::SharedPeriodPolicy::default();
        let mut render_target = None;
        let mut wake_margin_frames = 96;
        let mut callback_budget_frames = 48;
        for pair in arguments.chunks(2) {
            if pair.len() != 2 {
                bail!("Every option requires a value.");
            }
            match pair[0].as_str() {
                "--input" => {
                    if input.replace(pair[1].clone()).is_some() {
                        bail!("Duplicate --input.");
                    }
                }
                "--output" => {
                    if output.replace(pair[1].clone()).is_some() {
                        bail!("Duplicate --output.");
                    }
                }
                "--seconds" => seconds = pair[1].parse().context("Invalid duration")?,
                "--gain" => gain = pair[1].parse().context("Invalid gain")?,
                "--render-target-frames" => {
                    render_target = if pair[1] == "capacity" {
                        None
                    } else {
                        Some(pair[1].parse().context("Invalid render target")?)
                    };
                }
                "--wake-margin-frames" => {
                    wake_margin_frames = pair[1].parse().context("Invalid wake margin")?
                }
                "--callback-budget-frames" => {
                    callback_budget_frames = pair[1].parse().context("Invalid callback budget")?
                }
                "--period-ms" => {
                    shared_period.desired_period_ms = if pair[1] == "default" {
                        None
                    } else {
                        Some(pair[1].parse().context("Invalid shared period")?)
                    };
                }
                "--mode" => {
                    mode = match pair[1].as_str() {
                        "disabled" => TelemetryMode::Disabled,
                        "summary" => TelemetryMode::Summary,
                        "trace" => TelemetryMode::Trace,
                        _ => bail!("Unknown profiling mode."),
                    }
                }
                option => bail!("Unknown option: {option}"),
            }
        }
        if !(1..=600).contains(&seconds) || !gain.is_finite() || !(0.0..=1.0).contains(&gain) {
            bail!("Duration must be 1..600 seconds and gain must be finite in [0,1].");
        }
        let input = input.context("An explicit --input endpoint ID is required")?;
        let output = output.context("An explicit --output endpoint ID is required")?;
        let duration = SessionDuration::For(Duration::from_secs(seconds));
        shared_period.validate()?;
        let queue = render_target.map_or(
            moiren_windows_audio::latency::RenderQueuePolicy::FullCapacity,
            |frames| moiren_windows_audio::latency::RenderQueuePolicy::Target {
                frames,
                wake_margin_frames,
                callback_budget_frames,
            },
        );
        queue.validate()?;
        let stop = StopSignal::new()?;
        let gate = ActivationGate::new(stop.clone())?;
        let capture = prepare_capture_native_with_gate(
            CaptureOptions {
                endpoint_id: input,
                duration,
                latency_telemetry: mode,
                shared_period,
            },
            stop.clone(),
            gate.clone(),
        )?;
        let render_options = RenderOptions {
            queue,
            endpoint_id: output,
            duration,
            latency_telemetry: mode,
            shared_period,
        };
        let render = prepare_render_native_with_gate(render_options.clone(), stop, gate.clone())?;
        let budget_policy = render_target.map_or(
            moiren_windows_audio::latency::LatencyBudgetPolicy::Compatible,
            |_| moiren_windows_audio::latency::LatencyBudgetPolicy::Custom {
                input_capacity_frames: 8192,
                input_target_frames: 1024,
                input_jitter_frames: 96,
                render: queue,
            },
        );
        let budget = moiren_windows_audio::latency::prepare_latency_budget(
            capture.info().timing,
            render.info().timing,
            budget_policy,
        )?;
        let prepared = capture.attach(budget.input)?;
        let observer = prepared.observer;
        let capture = prepared.session;
        let mut graph = LogicalGraph::new();
        let source = graph.create_node(NodeKind::Source, 2)?;
        let gain_node = graph.create_node(NodeKind::Gain, 2)?;
        let pan_node = graph.create_node(NodeKind::Pan, 2)?;
        let sink = graph.create_node(NodeKind::Sink, 2)?;
        for (from, to) in [(source, gain_node), (gain_node, pan_node), (pan_node, sink)] {
            graph.connect(
                graph.get_node(from)?.outputs()[0].id(),
                graph.get_node(to)?.inputs()[0].id(),
                SendParams::default(),
            )?;
        }
        let (writer, reader) = audio_bridge(2, 256, 8 * 1024 * 1024)?;
        let mut bindings = NodeBindings::new();
        bindings.bind_source(source, prepared.source)?;
        bindings.bind_gain(gain_node, gain)?;
        bindings.bind_pan(pan_node, 0.0)?;
        bindings.bind_sink(sink, writer)?;
        let compiled = compile(
            &graph,
            bindings,
            CompileConfig {
                engine: EngineConfig {
                    processing_sr: 48_000.0,
                    max_block_frames: 256,
                    max_events_per_block: 64,
                },
                audio_byte_budget: 8 * 1024 * 1024,
                plan_revision: 1,
                timeline_epoch: 1,
                control_capacity: 64,
                control_horizon_frames: 48_000,
            },
        )?;
        let renderer = DemandRenderer::new(compiled.engine, reader)?;
        let _control = compiled.control;
        let _bindings = compiled.bindings;
        let render = render.attach(renderer, queue)?;
        gate.activate()?;
        let render = render.into_session();
        let capture_report = capture.join();
        let render_report = render.join();
        let capture_report = capture_report?;
        let render_report = render_report?;
        let failed = capture_report.failure.is_some() || render_report.failure.is_some();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 4, "measurement_method": "unknown", "gain": gain,
                "graph": "input_send_gain_send_pan_send_output", "capture": capture_report,
                "render": render_report, "bridge": observer.snapshot(),
            }))?
        );
        if failed {
            bail!("An audio owner failed; see the report.");
        }
        Ok(())
    }
}
