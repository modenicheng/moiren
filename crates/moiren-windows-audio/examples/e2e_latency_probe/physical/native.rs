//! Production physical IO ownership; no native execution is used by tests.
use super::super::process_collection::{QualityDiagnostics, validate_measurement_outcome};
use super::{
    Options, RecorderConfig,
    session::{
        self, GraphObservation, GraphTracker, NativeKind, NativeObservation, NativeStream,
        NativeStreamInfoView, Operations, Stream,
    },
};
use crate::measurement::session::{MeasurementPaths, MeasurementProgress, MeasurementSession};
use anyhow::{Context, Result, ensure};
use moiren_engine::{
    host::{EngineHost, EngineObserver, ExecutionState, HostConfig, UpdateState},
    node::{EndpointSelector, InputSource, OutputTarget},
};
use moiren_windows_audio::engine_backend::{
    NativeObservationReadHandle, NativeObservationSnapshot, NativeObservations, NativeStreamKind,
    ProfessionalBackend, ProfessionalObservationReadHandle, ProfessionalObservations,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn run_native(options: &Options, config: &RecorderConfig, directory: &Path) -> Value {
    let mut adapter = Adapter {
        options,
        config,
        directory,
        beginning: Instant::now(),
        recorder: None,
        host: None,
        observer: None,
        native: None,
        professional: None,
        retained_native: None,
        retained_professional: None,
        expected_owner: None,
        expected_host: None,
        tracker: None,
        recorder_info: None,
        recorder_final: None,
        graph_final: None,
        preparation_failure: None,
        graph_opened: false,
    };
    session::collect(options, &mut adapter)
}

struct Adapter<'a> {
    options: &'a Options,
    config: &'a RecorderConfig,
    directory: &'a Path,
    beginning: Instant,
    recorder: Option<MeasurementSession>,
    host: Option<EngineHost<f32>>,
    observer: Option<EngineObserver>,
    native: Option<NativeObservations>,
    professional: Option<ProfessionalObservations>,
    retained_native: Option<NativeObservationReadHandle>,
    retained_professional: Option<ProfessionalObservationReadHandle>,
    expected_owner: Option<u64>,
    expected_host: Option<moiren_engine::host::HostId>,
    tracker: Option<GraphTracker>,
    recorder_info: Option<Value>,
    recorder_final: Option<Value>,
    graph_final: Option<Value>,
    preparation_failure: Option<Value>,
    graph_opened: bool,
}

fn project_native(snapshot: NativeObservationSnapshot) -> Result<NativeObservation> {
    let raw = serde_json::to_value(&snapshot)?;
    let streams = snapshot
        .streams
        .iter()
        .map(|stream| {
            Ok(NativeStream {
                node: stream.node,
                kind: match stream.kind {
                    NativeStreamKind::PhysicalInput => NativeKind::PhysicalInput,
                    NativeStreamKind::MasterOutput => NativeKind::MasterOutput,
                    _ => NativeKind::Unexpected,
                },
                timing: stream.timing,
                shared_period: stream.shared_period,
                metadata: serde_json::to_value(stream)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(NativeObservation {
        owner_generation: snapshot.owner_generation,
        running: snapshot.running,
        streams,
        raw,
    })
}

fn project_professional(
    s: &moiren_windows_audio::engine_backend::ProfessionalStreamObservation,
) -> Stream {
    Stream {
        node: format!("{:?}", s.node),
        input: s.input,
        master: s.master,
        mode: s.requested.mode,
        actual: NativeStreamInfoView::from(s.actual),
        callbacks: s.native_callbacks,
        wakes: s.native_wait_wakes,
        written: s.native_written_frames,
        timeouts: s.native_wait_timeouts,
        dropped: s.dropped_frames,
        silent: s.silent_frames,
        native_dropped: s.native_dropped_frames,
        native_silent: s.native_silent_frames,
        capture_quality: s.native_capture_quality,
    }
}

impl Adapter<'_> {
    fn pin_and_observe(&mut self) -> Result<GraphObservation> {
        let project = self
            .observer
            .as_ref()
            .context("physical host observer missing")?
            .read();
        if let Some(expected) = self.expected_host {
            ensure!(
                project.host_id == expected,
                "physical host identity changed"
            );
        }
        let state = project.state;
        if state.owner_generation > 0 {
            if let Some(expected) = self.expected_owner {
                ensure!(
                    state.owner_generation == expected,
                    "physical owner generation changed; automatic adoption prohibited"
                );
            } else {
                self.expected_owner = Some(state.owner_generation);
            }
        }
        if self.retained_native.is_none() {
            self.retained_native = self.native.as_ref().and_then(|v| v.retain_session());
        }
        if self.retained_professional.is_none() {
            self.retained_professional =
                self.professional.as_ref().and_then(|v| v.retain_session());
        }
        let native = self
            .retained_native
            .as_ref()
            .context("physical native session unavailable")?
            .snapshot();
        let professional = self
            .retained_professional
            .as_ref()
            .context("physical professional session unavailable")?
            .snapshot();
        let fault = state
            .fault
            .as_ref()
            .map(|e| format!("{e:?}"))
            .or_else(|| state.cleanup_error.as_ref().map(|e| format!("{e:?}")))
            .or_else(|| match &state.update {
                UpdateState::Blocked { reason, .. } => Some(format!("{reason:?}")),
                _ => None,
            });
        Ok(GraphObservation {
            host_id: format!("{:?}", project.host_id),
            host: state.host,
            execution: state.execution,
            owner: state.owner_generation,
            resource: state.resource_generation,
            desired: state.desired_revision,
            active: state.active_revision,
            plan: state.plan_revision,
            timeline: state.timeline.map(|t| [t.epoch, t.revision, t.frame]),
            fault,
            professional_owner: professional.owner_generation,
            professional_running: professional.running,
            streams: professional
                .streams
                .iter()
                .map(project_professional)
                .collect(),
            native: project_native(native)?,
        })
    }
}

impl Operations for Adapter<'_> {
    fn elapsed(&self) -> Duration {
        self.beginning.elapsed()
    }
    fn wait(&mut self) {
        std::thread::sleep(Duration::from_millis(10));
    }
    fn prepare_recorder(&mut self) -> Result<()> {
        match MeasurementSession::prepare(
            self.config.measurement_config(self.options),
            MeasurementPaths {
                pcm: self.directory.join("measurement.f32le"),
                packets: self.directory.join("measurement-packets.jsonl"),
            },
        ) {
            Ok(owner) => {
                // Install ownership before any fallible metadata/grace setup.
                self.recorder = Some(owner);
                let owner = self.recorder.as_ref().expect("installed measurement owner");
                let info = owner.info();
                self.recorder_info = Some(
                    json!({"input_endpoint_id":info.input_endpoint_id,"sample_rate":info.sample_rate,"channels":info.channels,"ring_frames":info.ring_frames,"max_frames":info.max_frames}),
                );
                ensure!(
                    info.input_endpoint_id == self.config.input_endpoint_id,
                    "independent recorder prepared endpoint mismatch"
                );
                let owner = self.recorder.take().expect("installed measurement owner");
                // This method validates before mutating; its failure still drops
                // and joins the independently owned measurement session.
                self.recorder =
                    Some(owner.with_start_grace(self.options.validation.progress_grace())?);
                Ok(())
            }
            Err(failure) => {
                self.preparation_failure = Some(
                    json!({"error":format!("{:#}",failure.error),"native_capture":failure.native_capture}),
                );
                Err(failure.error)
            }
        }
    }
    fn activate_recorder(&mut self) -> Result<()> {
        self.recorder
            .as_mut()
            .context("recorder not prepared")?
            .activate()
    }
    fn recorder_running(&mut self) -> Result<bool> {
        match self
            .recorder
            .as_ref()
            .context("recorder not prepared")?
            .check()?
        {
            MeasurementProgress::Running { .. } => Ok(true),
            MeasurementProgress::Prepared | MeasurementProgress::Starting => Ok(false),
            other => anyhow::bail!("physical recorder stopped/deadline reached: {other:?}"),
        }
    }
    fn open_graph(&mut self) -> Result<()> {
        let prepared = super::topology::prepare(self.options)?;
        // The exact pinned selectors are also kept in the report. Observation
        // DTOs identify nodes, not independently authenticated hardware IDs.
        use moiren_engine::host::NodeConfig;
        ensure!(
            matches!(prepared.project.nodes.get(&prepared.input),Some(NodeConfig::Input(input)) if matches!(&input.source,InputSource::Physical(EndpointSelector::PinnedEndpoint{endpoint_id,..}) if endpoint_id == &self.options.input_endpoint_id)),
            "physical source selector changed"
        );
        ensure!(
            matches!(prepared.project.nodes.get(&prepared.output),Some(NodeConfig::Output(output)) if matches!(&output.target,OutputTarget::Physical(EndpointSelector::PinnedEndpoint{endpoint_id,..}) if endpoint_id == &self.options.output_endpoint_id)),
            "physical output selector changed"
        );
        self.tracker = Some(GraphTracker::new(prepared.input, prepared.output));
        let backend = ProfessionalBackend::<f32>::new()?
            .with_render_queue(self.options.render_queue)?
            .with_latency_budget(self.options.input_budget)?
            .with_telemetry(self.options.telemetry);
        self.native = Some(backend.native_observations());
        self.professional = Some(backend.observations());
        let host = EngineHost::open(HostConfig::default(), prepared.project, backend)?;
        self.observer = Some(host.observer());
        self.host = Some(host);
        self.graph_opened = true;
        self.expected_host = self.observer.as_ref().map(|o| o.read().host_id);
        Ok(())
    }
    fn graph_running(&mut self, quality: &mut QualityDiagnostics) -> Result<bool> {
        let state = self
            .observer
            .as_ref()
            .context("physical observer missing")?
            .snapshot();
        ensure!(
            state.fault.is_none()
                && state.cleanup_error.is_none()
                && !matches!(state.update, UpdateState::Blocked { .. }),
            "physical host failed: {state:?}"
        );
        if state.owner_generation > 0 {
            if let Some(owner) = self.expected_owner {
                ensure!(
                    state.owner_generation == owner,
                    "physical owner replaced during startup"
                );
            } else {
                self.expected_owner = Some(state.owner_generation);
            }
        }
        if state.execution != ExecutionState::Running
            || state.active_revision != Some(1)
            || !state.timeline.is_some_and(|t| t.frame > 0)
        {
            ensure!(
                self.tracker.as_ref().is_none_or(|t| t.startup.is_none()),
                "physical graph lost Running after readiness"
            );
            return Ok(false);
        }
        let sample = self.pin_and_observe()?;
        let elapsed = self.elapsed();
        self.tracker
            .as_mut()
            .context("physical tracker missing")?
            .observe(self.options, &sample, elapsed, quality)
    }
    fn stop_recorder(&mut self) -> Result<()> {
        if let Some(recorder) = &mut self.recorder {
            recorder.request_stop()?;
        }
        Ok(())
    }
    fn finish_graph(&mut self, quality: &mut QualityDiagnostics) -> Result<Value> {
        // Retain any published owner even on startup/refusal. Never redirect an
        // already pinned handle to a replacement registry generation.
        if self.retained_native.is_none() {
            self.retained_native = self.native.as_ref().and_then(|v| v.retain_session());
        }
        if self.retained_professional.is_none() {
            self.retained_professional =
                self.professional.as_ref().and_then(|v| v.retain_session());
        }
        drop(self.host.take()); // Synchronous EngineHost stop and all-worker join.
        if !self.graph_opened {
            return Ok(json!({"opened":false,"host_drop_returned":true}));
        }
        let sample = self.pin_and_observe()?;
        self.graph_final = Some(
            json!({"host_drop_returned":true,"final_closed":sample.host == moiren_engine::host::HostState::Closed,
            "expected_owner_generation":self.expected_owner,"actual_owner_generation":sample.owner,
            "fault":sample.fault,"native":sample.native.raw,"streams":sample.streams}),
        );
        let tracker = self
            .tracker
            .as_mut()
            .context("physical tracker missing at join")?;
        tracker.finish(self.options, &sample, quality)?;
        ensure!(
            self.expected_owner == Some(sample.owner),
            "physical final owner mismatch"
        );
        Ok(self.graph_final.clone().expect("recorded final evidence"))
    }
    fn finish_recorder(&mut self, quality: &mut QualityDiagnostics) -> Result<Value> {
        let Some(mut recorder) = self.recorder.take() else {
            return Ok(json!({"prepared":false}));
        };
        let outcome = recorder.finish()?;
        self.recorder_final = Some(
            json!({"info":{"input_endpoint_id":outcome.info.input_endpoint_id,"sample_rate":outcome.info.sample_rate,
            "channels":outcome.info.channels,"ring_frames":outcome.info.ring_frames,"max_frames":outcome.info.max_frames},
            "native_capture":outcome.native_capture,"sink":outcome.sink,"activation_elapsed_seconds":outcome.activation_elapsed.map(|v|v.as_secs_f64()),"failure":outcome.failure}),
        );
        ensure!(
            outcome.info.input_endpoint_id == self.config.input_endpoint_id,
            "recorder final endpoint mismatch"
        );
        let native = outcome
            .native_capture
            .as_ref()
            .context("recorder final native report missing")?;
        ensure!(
            native.endpoint_id.as_deref() == Some(self.config.input_endpoint_id.as_str())
                && native.sample_rate == Some(outcome.info.sample_rate)
                && native.channels == Some(outcome.info.channels),
            "recorder final native endpoint/format mismatch"
        );
        validate_measurement_outcome(&outcome, self.options.validation, quality)?;
        Ok(self
            .recorder_final
            .clone()
            .expect("recorded recorder outcome"))
    }
    fn evidence(&self) -> Value {
        json!({"identity_scope":"requested pinned endpoint selectors mapped to graph nodes; not independent hardware provenance",
            "input_endpoint_id":self.options.input_endpoint_id,"output_endpoint_id":self.options.output_endpoint_id,
            "recorder_endpoint_id":self.config.input_endpoint_id,"expected_host":self.expected_host.map(|v|format!("{v:?}")),
            "expected_owner_generation":self.expected_owner,"physical_admission":self.tracker.as_ref().and_then(|t|t.admission.as_ref()),
            "startup_counters":self.tracker.as_ref().and_then(|t|t.startup.as_ref()),
            "capture_quality_observed_all_samples":self.tracker.as_ref().is_some_and(GraphTracker::capture_quality_observed_all_samples),
            "latest":self.tracker.as_ref().and_then(|t|t.latest.as_ref()),
            "final_after_host_drop":self.tracker.as_ref().and_then(|t|t.final_observation.as_ref()),
            "graph_final":self.graph_final,"recorder_info":self.recorder_info,"recorder_final":self.recorder_final,"preparation_failure":self.preparation_failure})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeKind};
    use moiren_windows_audio::{
        engine_backend::NativeStreamObservation,
        latency::{NativeStreamTiming, PreparedRenderQueue},
    };

    #[test]
    fn physical_capture_quality_projection_preserves_unknown_and_exact_native_flags() {
        use moiren_windows_audio::{
            engine_backend::ProfessionalStreamObservation, native::NativeCaptureQuality,
        };
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::Source, 2).unwrap();
        let mut stream = ProfessionalStreamObservation {
            node,
            input: true,
            master: false,
            requested: Default::default(),
            actual: moiren_windows_audio::native::NativeStreamInfo {
                sample_rate: 48_000,
                format: moiren_engine::audio_config::SampleFormat::Float32,
                buffer_frames: 256,
                channels: 2,
            },
            dropped_frames: 3,
            silent_frames: 5,
            native_callbacks: 0,
            native_wait_wakes: 7,
            native_written_frames: 0,
            native_wait_timeouts: 11,
            native_dropped_frames: 13,
            native_silent_frames: 17,
            native_capture_quality: None,
            driver_latency: None,
        };
        assert_eq!(project_professional(&stream).capture_quality, None);
        stream.native_capture_quality = Some(NativeCaptureQuality {
            discontinuities: 19,
            timestamp_errors: 23,
            silent_frames: (1 << 53) + 29,
        });
        let projected = project_professional(&stream);
        assert_eq!(projected.capture_quality, stream.native_capture_quality);
        assert_eq!(
            projected.native_silent, 17,
            "render silence stays separate from capture flags"
        );
        assert_eq!(
            serde_json::to_value(projected).unwrap()["capture_quality"]["silent_frames"].as_u64(),
            Some((1 << 53) + 29)
        );
    }

    #[test]
    fn physical_native_projection_preserves_raw_snapshot_and_all_prepared_metadata() {
        let mut graph = LogicalGraph::new();
        let node = graph.create_node(NodeKind::Sink, 2).unwrap();
        let snapshot = NativeObservationSnapshot {
            virtual_clock: None,
            owner_generation: 7,
            running: true,
            streams: vec![NativeStreamObservation {
                node,
                kind: NativeStreamKind::MasterOutput,
                timing: Some(NativeStreamTiming {
                    sample_rate: 48_000,
                    period_frames: Some(128),
                    capacity_frames: 512,
                }),
                shared_period: None,
                render_queue: Some(PreparedRenderQueue {
                    capacity_frames: 512,
                    target_frames: 256,
                    deadline_budget_frames: Some(128),
                }),
                input_queue: None,
                virtual_budget: None,
            }],
            bridges: vec![],
            boundary_probes: vec![],
            render_padding: vec![],
            process_delivery: vec![],
        };
        let expected = serde_json::to_value(&snapshot).unwrap();
        let projected = project_native(snapshot).unwrap();
        assert_eq!(projected.raw, expected);
        assert_eq!(projected.owner_generation, 7);
        assert!(projected.running);
        assert_eq!(projected.streams[0].node, node);
        assert_eq!(projected.streams[0].kind, NativeKind::MasterOutput);
        assert_eq!(
            projected.streams[0].timing.unwrap().period_frames,
            Some(128)
        );
        assert_eq!(projected.streams[0].metadata, expected["streams"][0]);
    }
}
