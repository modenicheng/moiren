//! Existing Process graph assembly and its control-side native ownership.
use super::super::render::{
    DemandRenderer, PreparedNativeRender, RenderOptions, RenderReport, RenderSession,
    TimelineObserver, prepare_render_native_with_gate_returning_with_abort,
};
use super::super::{ArtifactPaths, LiveOptions};
use super::append_failure;
use anyhow::{Context, Result, ensure};
use moiren_core::graph::{LogicalGraph, NodeKind, SendParams, edit::connect_to_new_bus_input};
use moiren_engine::{
    boundary::audio_bridge,
    compiler::{CompileConfig, NodeBindings, compile},
    runtime::EngineConfig,
};
use moiren_windows_audio::{
    ActivationGate, SessionDuration, StopSignal,
    capture::{
        CapturePreparationAbortFailure, CaptureReport, CaptureSession, PcmArtifact,
        PcmArtifactConfig, PreparedCapture,
    },
    clock_bridge::{BridgeObserver, ClockSource},
    latency::RenderQueuePolicy,
    process_loopback::{
        ProcessBridgeOptions, ProcessIdentity, ProcessLoopbackOptions, ProcessTraceCollector,
        prepare_process_capture_with_gate_returning_with_abort,
    },
};
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::{fs::OpenOptions, io::Write};

pub(in crate::live) fn build_renderer(
    options: &LiveOptions,
    source: ClockSource,
) -> Result<DemandRenderer> {
    let mut graph = LogicalGraph::new();
    let input = graph.create_node(NodeKind::Source, 2)?;
    let bus = graph.create_node(NodeKind::Bus, 2)?;
    let output = graph.create_node(NodeKind::Sink, 2)?;
    let source_port = graph.get_node(input)?.outputs()[0].id();
    let bus_port = graph.get_node(bus)?.outputs()[0].id();
    let sink_port = graph.get_node(output)?.inputs()[0].id();
    connect_to_new_bus_input(&mut graph, source_port, bus, SendParams::default())?;
    graph.connect(bus_port, sink_port, SendParams::default())?;
    let (writer, reader) = audio_bridge(2, 8192, 8 * 1024 * 1024)?;
    let mut bindings = NodeBindings::new();
    bindings.bind_source(input, source)?;
    bindings.bind_sink(output, writer)?;
    let compiled = compile(
        &graph,
        bindings,
        CompileConfig {
            engine: EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: options.engine_block_frames as usize,
                max_events_per_block: 64,
            },
            audio_byte_budget: 8 * 1024 * 1024,
            plan_revision: 1,
            timeline_epoch: 1,
            control_capacity: 64,
            control_horizon_frames: 48_000,
        },
    )
    .context("compile Input -> Bus -> Output graph")?;
    DemandRenderer::new(compiled.engine, reader).context("prepare Engine renderer")
}

pub(super) struct Graph {
    stop: StopSignal,
    gate: ActivationGate,
    capture: Option<CaptureSession>,
    prepared_render: Option<PreparedNativeRender>,
    render: Option<RenderSession>,
    recovered_renderer: Option<DemandRenderer>,
    bridge: Option<BridgeObserver>,
    timeline: Option<TimelineObserver>,
    trace: Option<ProcessTraceCollector>,
    process_admission: Option<super::super::process_admission::ProcessAdmissionEvidence>,
    pub(super) capture_report: Option<CaptureReport>,
    pub(super) render_report: Option<RenderReport>,
    pub(super) aborts: Vec<Value>,
    pub(super) failures: Option<String>,
}

impl Graph {
    pub(super) fn new() -> Result<Self> {
        let stop = StopSignal::new()?;
        let gate = ActivationGate::new(stop.clone())?;
        Ok(Self {
            stop,
            gate,
            capture: None,
            prepared_render: None,
            render: None,
            recovered_renderer: None,
            bridge: None,
            timeline: None,
            trace: None,
            process_admission: None,
            capture_report: None,
            render_report: None,
            aborts: Vec::with_capacity(3),
            failures: None,
        })
    }

    pub(super) fn prepare(
        &mut self,
        options: &LiveOptions,
        bridge: &ProcessBridgeOptions,
        queue: RenderQueuePolicy,
        identity: ProcessIdentity,
        mut stop_peers: impl FnMut() -> Option<String>,
    ) -> Result<()> {
        // Prepare while gated, then budget the entire empty-padding wake.
        // Rendering that wake in Engine-sized segments cannot reduce reserve.
        let render = match prepare_render_native_with_gate_returning_with_abort(
            RenderOptions::from_live(options, queue, SessionDuration::UntilStopped),
            self.stop.clone(),
            self.gate.clone(),
            &mut stop_peers,
        ) {
            Ok(render) => render,
            Err(mut error) => {
                self.aborts.push(error.evidence("render_prepare"));
                self.render_report = error.report.take();
                anyhow::bail!("prepare owned render: {}", error.error);
            }
        };
        let capture = self.prepare_capture_after_render(
            render,
            options,
            *bridge,
            identity,
            &mut stop_peers,
            |capture, stop, gate, before_join| {
                prepare_process_capture_with_gate_returning_with_abort(
                    capture,
                    stop,
                    gate,
                    before_join,
                )
            },
        )?;
        let PreparedCapture {
            session,
            source,
            observer,
            sample_rate,
            channels,
        } = capture;
        self.capture = Some(session);
        self.bridge = Some(observer);
        self.poll_trace();
        ensure!(
            sample_rate == 48_000 && channels == 2,
            "Process graph negotiation is not 48k stereo"
        );
        let renderer = build_renderer(options, source)?.with_bridge_observer(
            self.bridge
                .as_ref()
                .expect("prepared capture observer")
                .clone(),
        );
        self.timeline = Some(renderer.timeline_observer());
        let render = self
            .prepared_render
            .take()
            .context("prepared render missing")?;
        match render.attach_returning_with_abort(renderer, queue, stop_peers) {
            Ok(prepared) => self.render = Some(prepared.into_session()),
            Err(mut error) => {
                self.aborts.push(error.evidence("render_attach"));
                self.render_report = error.report.take();
                self.recovered_renderer = Some(error.renderer);
                anyhow::bail!("attach owned render: {}", error.error);
            }
        }
        Ok(())
    }

    fn prepare_capture_after_render(
        &mut self,
        render: PreparedNativeRender,
        options: &LiveOptions,
        bridge: ProcessBridgeOptions,
        identity: ProcessIdentity,
        mut stop_peers: impl FnMut() -> Option<String>,
        prepare_capture: impl FnOnce(
            ProcessLoopbackOptions,
            StopSignal,
            ActivationGate,
            &mut dyn FnMut() -> Option<String>,
        ) -> std::result::Result<
            PreparedCapture,
            Box<CapturePreparationAbortFailure<Option<String>>>,
        >,
    ) -> Result<PreparedCapture> {
        let info = render.info();
        // Retain the parked owner before any fallible admission/capture work,
        // so the control-side finish path can preserve its final report.
        self.prepared_render = Some(render);
        let (_, capture) = super::super::process_admission::prepare_capture_for_render_recording(
            bridge,
            info.timing,
            info.queue,
            &mut self.process_admission,
            |bridge| {
                self.trace = Some(ProcessTraceCollector::new(
                    identity.clone(),
                    1,
                    48_000,
                    48_000,
                    bridge.max_demand_frames,
                ));
                let mut before_join = || {
                    // Broadcast peer cancellation before joining either owner.
                    // Even a panicking peer hook must release the parked render;
                    // then let capture's abort guard retain the panic evidence.
                    let peers = catch_unwind(AssertUnwindSafe(&mut stop_peers));
                    let cancelled =
                        cancel_prepared_render(&mut self.prepared_render, &mut self.render_report);
                    if let Some(failure) = &cancelled {
                        append_failure(&mut self.failures, failure);
                    }
                    match peers {
                        Ok(mut failures) => {
                            if let Some(failure) = cancelled {
                                append_failure(&mut failures, &failure);
                            }
                            failures
                        }
                        Err(panic) => std::panic::resume_unwind(panic),
                    }
                };
                match prepare_capture(
                    ProcessLoopbackOptions {
                        artifact: Some(PcmArtifactConfig::new(
                            usize::try_from(options.seconds)?
                                .checked_mul(48_000)
                                .context("capture artifact cap overflow")?,
                        )),
                        bridge,
                        latency_telemetry: options.telemetry,
                        target: identity,
                        duration: SessionDuration::UntilStopped,
                    },
                    self.stop.clone(),
                    self.gate.clone(),
                    &mut before_join,
                ) {
                    Ok(prepared) => Ok(prepared),
                    Err(mut error) => {
                        self.capture_report = error.report.take();
                        self.aborts.push(json!({"stage":"capture_prepare", "error":error.error.to_string(),
                            "stop_error":error.stop_error.map(|e|e.to_string()), "join_error":error.join_error.map(|e|e.to_string()),
                            "abort":error.abort, "abort_panicked":error.abort_panicked}));
                        Err(error.error).context("prepare owned Process capture")
                    }
                }
            },
        )?;
        Ok(capture)
    }

    pub(super) fn activate(&mut self) -> Result<()> {
        self.poll_trace();
        self.gate.activate().context("activate Process graph")
    }
    pub(super) fn request_stop(&self) -> Result<()> {
        self.stop.request_stop().context("stop Process graph")
    }

    pub(super) fn poll_trace(&mut self) {
        if let (Some(trace), Some(observer)) = (&mut self.trace, &self.bridge) {
            trace.poll(observer);
        }
    }

    pub(super) fn observation(&mut self) -> Value {
        self.poll_trace();
        json!({"capture_started":self.capture.as_ref().map(CaptureSession::has_started),
            "render_started":self.render.as_ref().map(RenderSession::has_started),
            "capture_finished":self.capture.as_ref().map(CaptureSession::is_finished),
            "render_finished":self.render.as_ref().map(RenderSession::is_finished),
            "bridge":self.bridge.as_ref().map(BridgeObserver::snapshot),
            "timeline":self.timeline.as_ref().and_then(TimelineObserver::try_snapshot).map(|s|json!({"frame":s.frame,"epoch":s.epoch,"revision":s.revision}))})
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        self.poll_trace();
        if let Some(capture) = self.capture.take() {
            match catch_unwind(AssertUnwindSafe(|| capture.join_retaining_report())) {
                Ok((report, error)) => {
                    self.capture_report = report;
                    if let Some(error) = error {
                        append_failure(&mut self.failures, &format!("join capture: {error}"));
                    }
                    if self.capture_report.is_none() {
                        append_failure(&mut self.failures, "capture report missing after join");
                    }
                }
                Err(_) => append_failure(&mut self.failures, "join capture panic"),
            }
        }
        self.poll_trace();
        if let Some(failure) =
            cancel_prepared_render(&mut self.prepared_render, &mut self.render_report)
        {
            append_failure(&mut self.failures, &failure);
        }
        if let Some(render) = self.render.take() {
            match catch_unwind(AssertUnwindSafe(|| render.join_with_renderer())) {
                Ok(Ok(exit)) => {
                    self.render_report = Some(exit.report);
                    self.recovered_renderer = Some(exit.renderer);
                }
                Ok(Err(error)) => {
                    append_failure(&mut self.failures, &format!("join render: {error}"))
                }
                Err(_) => append_failure(&mut self.failures, "join render panic"),
            }
        }
        self.recovered_renderer.take();
        self.poll_trace();
        ensure!(
            self.failures.is_none(),
            "{}",
            self.failures.as_deref().unwrap_or_default()
        );
        Ok(())
    }

    pub(super) fn persist(&self, paths: &ArtifactPaths) -> Result<()> {
        if let Some(artifact) = self
            .capture_report
            .as_ref()
            .and_then(|report| report.artifact.as_ref())
        {
            persist_artifact_with(paths, artifact, |path| {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .with_context(|| format!("create new capture PCM artifact {path}"))
            })?;
        }
        Ok(())
    }

    pub(super) fn export_trace(
        &self,
        paths: &ArtifactPaths,
        report: &mut super::super::NativeReport,
    ) -> Result<()> {
        report.process_admission = self.process_admission.clone();
        match (&self.bridge, &self.trace) {
            (Some(_), Some(trace)) => {
                super::super::process_trace::export_collector_into(report, paths, trace)
            }
            _ => {
                report.process_trace = Some(
                    json!({"schema_version":1,"exported":false,"qualified":false,
                    "failure":"Process bridge was not prepared"}),
                );
                Ok(())
            }
        }
    }
}

fn cancel_prepared_render(
    prepared: &mut Option<PreparedNativeRender>,
    retained: &mut Option<RenderReport>,
) -> Option<String> {
    let render = prepared.take()?;
    match catch_unwind(AssertUnwindSafe(|| render.cancel_retaining_report())) {
        Ok((report, result)) => {
            *retained = report;
            result
                .err()
                .map(|error| format!("cancel prepared render: {error:#}"))
        }
        Err(_) => Some(String::from("cancel prepared render panic")),
    }
}

pub(super) fn persist_artifact_with<W: Write>(
    paths: &ArtifactPaths,
    artifact: &PcmArtifact,
    open: impl FnOnce(&str) -> Result<W>,
) -> Result<()> {
    let pcm_result = (|| -> Result<()> {
        let pcm_path = paths
            .capture_pcm
            .as_deref()
            .context("capture PCM artifact path missing")?;
        let mut file = open(pcm_path)?;
        file.write_all(artifact.pcm_bytes())
            .context("write capture PCM artifact")?;
        file.flush().context("flush capture PCM artifact")
    })();
    // Independent artifact attempts: a failed PCM open/write/flush must not
    // suppress metadata, and neither path consumes the retained typed report.
    let metadata_result = (|| -> Result<()> {
        let metadata_path = paths
            .capture_pcm_metadata
            .as_deref()
            .context("capture PCM metadata path missing")?;
        super::super::write_json(metadata_path, &artifact.metadata())
            .context("write capture PCM metadata")
    })();
    let mut failures = None;
    for result in [pcm_result, metadata_result] {
        if let Err(error) = result {
            append_failure(&mut failures, &format!("{error:#}"));
        }
    }
    ensure!(
        failures.is_none(),
        "{}",
        failures.as_deref().unwrap_or_default()
    );
    Ok(())
}

#[cfg(test)]
#[path = "graph/tests.rs"]
mod tests;
