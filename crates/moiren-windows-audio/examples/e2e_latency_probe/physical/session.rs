//! Portable supervision shared by the physical native adapter and its tests.
use super::super::process_collection::QualityDiagnostics;
use super::Options;
use anyhow::{Result, ensure};
use moiren_core::graph::NodeId;
use moiren_engine::{
    audio_config::SampleFormat,
    host::{ExecutionState, HostState},
    node::DeviceMode,
};
use moiren_windows_audio::{
    latency::{NativeStreamTiming, shared_period::SharedPeriodReport},
    native::{NativeCaptureQuality, NativeStreamInfo},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
pub(super) struct Stream {
    pub node: String,
    pub input: bool,
    pub master: bool,
    pub mode: DeviceMode,
    pub actual: NativeStreamInfoView,
    pub callbacks: u64,
    pub wakes: u64,
    pub written: u64,
    pub timeouts: u64,
    pub dropped: u64,
    pub silent: u64,
    pub native_dropped: u64,
    pub native_silent: u64,
    pub capture_quality: Option<NativeCaptureQuality>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) struct NativeStreamInfoView {
    pub rate: u32,
    pub channels: usize,
    pub format: SampleFormat,
    pub capacity: u32,
}
impl From<NativeStreamInfo> for NativeStreamInfoView {
    fn from(info: NativeStreamInfo) -> Self {
        Self {
            rate: info.sample_rate,
            channels: info.channels,
            format: info.format,
            capacity: info.buffer_frames,
        }
    }
}

/// Only the Windows adapter imports the backend's observation types. Keep the
/// controller's admission data portable and preserve unprojected evidence too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeKind {
    PhysicalInput,
    MasterOutput,
    Unexpected,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NativeStream {
    pub node: NodeId,
    pub kind: NativeKind,
    pub timing: Option<NativeStreamTiming>,
    pub shared_period: Option<SharedPeriodReport>,
    /// Complete immutable stream metadata, including queues and future fields.
    pub metadata: Value,
}

#[derive(Clone, Debug)]
pub(super) struct NativeObservation {
    pub owner_generation: u64,
    pub running: bool,
    pub streams: Vec<NativeStream>,
    /// Full original native snapshot; never reconstruct it from the projection.
    pub raw: Value,
}

#[derive(Clone, Debug)]
pub(super) struct GraphObservation {
    pub host_id: String,
    pub host: HostState,
    pub execution: ExecutionState,
    pub owner: u64,
    pub resource: u64,
    pub desired: u64,
    pub active: Option<u64>,
    pub plan: Option<u64>,
    pub timeline: Option<[u64; 3]>,
    pub fault: Option<String>,
    pub professional_owner: u64,
    pub professional_running: bool,
    pub streams: Vec<Stream>,
    pub native: NativeObservation,
}

pub(super) struct GraphTracker {
    input: NodeId,
    output: NodeId,
    pub admission: Option<Value>,
    pub startup: Option<Value>,
    pub latest: Option<Value>,
    pub final_observation: Option<Value>,
    pinned: Option<GraphObservation>,
    previous: Option<GraphObservation>,
    baseline: Option<GraphObservation>,
    last_capture_quality: Option<NativeCaptureQuality>,
    capture_quality_incomplete: bool,
    progress: [Duration; 3],
}
impl GraphTracker {
    pub fn capture_quality_observed_all_samples(&self) -> bool {
        !self.capture_quality_incomplete
            && self.baseline.is_some()
            && self.final_observation.is_some()
            && self.last_capture_quality.is_some()
    }
    pub fn new(input: NodeId, output: NodeId) -> Self {
        Self {
            input,
            output,
            admission: None,
            startup: None,
            latest: None,
            final_observation: None,
            pinned: None,
            previous: None,
            baseline: None,
            last_capture_quality: None,
            capture_quality_incomplete: false,
            progress: [Duration::ZERO; 3],
        }
    }
    pub fn observe(
        &mut self,
        options: &Options,
        sample: &GraphObservation,
        elapsed: Duration,
        quality: &mut QualityDiagnostics,
    ) -> Result<bool> {
        let raw = graph_value(sample);
        self.latest = Some(raw.clone());
        let result = self.validate(options, sample, false);
        if self.admission.is_none() {
            self.admission = Some(
                json!({"scope":"post_native_activation_validation_not_pre_audio_gate",
                "requested_capture":options.capture,"requested_render":options.render,
                "input_node":format!("{:?}",self.input),"output_node":format!("{:?}",self.output),
                "validation_accepted":result.is_ok(),"failure":result.as_ref().err().map(|e|format!("{e:#}")),"snapshot":raw}),
            );
        }
        result?;
        let input = self.stream(sample, self.input)?;
        let output = self.stream(sample, self.output)?;
        if let Some(capture) = input.capture_quality {
            self.last_capture_quality = Some(capture);
        } else {
            self.capture_quality_incomplete = true;
        }
        let frame = sample.timeline.expect("validated timeline")[2];
        // Validate the interval since the last observed progress before a new
        // counter advance can refresh it. Late recovery cannot erase an expired
        // observation gap, including the terminal collection interval.
        if self.baseline.is_some() {
            for (index, name) in [
                "physical capture native activity",
                "physical output writes",
                "physical engine timeline",
            ]
            .iter()
            .enumerate()
            {
                let stalled = elapsed.saturating_sub(self.progress[index]);
                if stalled >= Duration::from_secs(1) {
                    quality.fault(options.validation, &format!("{name} stalled"))?;
                    ensure!(
                        stalled < Duration::from_secs(1) + options.validation.progress_grace(),
                        "{name} exceeded progress grace"
                    );
                }
            }
        }
        let mut advanced = [
            input.wakes > 0 || input.callbacks > 0,
            output.written > 0,
            frame > 0,
        ];
        if let Some(previous) = &self.previous {
            let old_input = self.stream(previous, self.input)?;
            let old_output = self.stream(previous, self.output)?;
            advanced = [
                input.wakes > old_input.wakes || input.callbacks > old_input.callbacks,
                output.written > old_output.written,
                frame > previous.timeline.expect("validated previous timeline")[2],
            ];
        }
        if self.pinned.is_none() {
            self.pinned = Some(sample.clone());
            self.progress = [elapsed; 3];
        }
        for (index, advanced) in advanced.iter().enumerate() {
            if *advanced {
                self.progress[index] = elapsed;
            }
        }
        let ready = if self.baseline.is_some() {
            true
        } else {
            let stable = self.previous.as_ref().is_none_or(|old| {
                self.stream(old, self.input)
                    .is_ok_and(|old| old.silent == input.silent)
            });
            advanced.iter().all(|v| *v) && stable && (input.silent == 0 || self.previous.is_some())
        };
        if let Some(baseline) = &self.baseline {
            self.quality_delta(options, sample, baseline, false, quality)?;
        } else {
            // Initial follower priming is explicit startup evidence, not a
            // steady underrun. Other known quality faults are not hidden.
            self.quality_delta(options, sample, sample, true, quality)?;
            if ready {
                self.startup = Some(raw);
                self.baseline = Some(sample.clone());
                self.progress = [elapsed; 3];
            }
        }
        self.previous = Some(sample.clone());
        Ok(ready)
    }
    pub fn finish(
        &mut self,
        options: &Options,
        sample: &GraphObservation,
        quality: &mut QualityDiagnostics,
    ) -> Result<()> {
        let mut value = graph_value(sample);
        // Preserve even malformed/regressed final observations before deriving
        // any checked deltas or returning an admission/teardown failure.
        self.final_observation = Some(value.clone());
        if self.stream(sample, self.input)?.capture_quality.is_none() {
            self.capture_quality_incomplete = true;
        }
        if let Some(baseline) = &self.baseline {
            value["steady_deltas"] = Value::Array([self.input,self.output].iter().map(|node| {
                let current = self.stream(sample,*node)?;
                let old = self.stream(baseline,*node)?;
                let delta = |a:u64,b:u64|a.checked_sub(b).ok_or_else(||anyhow::anyhow!("physical steady counter regressed"));
                Ok(json!({"node":current.node,"callbacks":delta(current.callbacks,old.callbacks)?,"wakes":delta(current.wakes,old.wakes)?,
                    "written":delta(current.written,old.written)?,"timeouts":delta(current.timeouts,old.timeouts)?,
                    "dropped":delta(current.dropped,old.dropped)?,"silent":delta(current.silent,old.silent)?,
                    "native_dropped":delta(current.native_dropped,old.native_dropped)?,"native_silent":delta(current.native_silent,old.native_silent)?,
                    "capture_quality":capture_delta(current.capture_quality,old.capture_quality)?}))
            }).collect::<Result<Vec<_>>>()?);
        }
        self.final_observation = Some(value);
        self.validate(options, sample, true)?;
        if let Some(baseline) = &self.baseline {
            self.quality_delta(options, sample, baseline, false, quality)?;
        }
        Ok(())
    }
    fn stream<'a>(&self, sample: &'a GraphObservation, node: NodeId) -> Result<&'a Stream> {
        sample
            .streams
            .iter()
            .find(|stream| stream.node == format!("{node:?}"))
            .ok_or_else(|| anyhow::anyhow!("physical stream node missing"))
    }
    fn validate(
        &self,
        options: &Options,
        sample: &GraphObservation,
        final_joined: bool,
    ) -> Result<()> {
        ensure!(
            sample.fault.is_none(),
            "physical host native/cleanup/update fault: {:?}",
            sample.fault
        );
        ensure!(
            sample.owner > 0
                && sample.resource > 0
                && sample.desired == 1
                && sample.professional_owner == sample.owner
                && sample.native.owner_generation == sample.owner,
            "physical observation owner/revision mismatch"
        );
        if final_joined {
            ensure!(
                sample.host == HostState::Closed
                    && !sample.professional_running
                    && !sample.native.running,
                "physical host not closed after join"
            );
        } else {
            ensure!(
                sample.host == HostState::Open
                    && sample.execution == ExecutionState::Running
                    && sample.active == Some(1)
                    && sample.plan.is_some()
                    && sample.professional_running
                    && sample.native.running,
                "physical graph no longer Running/current"
            );
            // The fixed project has graph/plan revision one. Timeline order is
            // [epoch, plan revision, frame], matching Host supervisor identity.
            ensure!(
                sample.plan == Some(1)
                    && sample
                        .timeline
                        .is_some_and(|t| t[0] > 0 && t[1] == 1 && t[2] > 0),
                "physical graph plan/timeline unavailable or mismatched"
            );
        }
        ensure!(
            sample.streams.len() == 2 && sample.native.streams.len() == 2,
            "physical route requires exactly two native streams"
        );
        for (node, kind, request, is_input) in [
            (self.input, NativeKind::PhysicalInput, options.capture, true),
            (self.output, NativeKind::MasterOutput, options.render, false),
        ] {
            let professional = self.stream(sample, node)?;
            ensure!(
                professional.input == is_input
                    && professional.master != is_input
                    && professional.mode == request.mode.unwrap_or_default(),
                "physical stream direction/master/mode mismatch"
            );
            ensure!(
                (8_000..=384_000).contains(&professional.actual.rate)
                    && (1..=32).contains(&professional.actual.channels)
                    && professional.actual.capacity > 0
                    && professional.actual.format != SampleFormat::Native,
                "physical native format invalid"
            );
            let mut matches = sample
                .native
                .streams
                .iter()
                .filter(|s| s.node == node && s.kind == kind);
            let native = matches
                .next()
                .ok_or_else(|| anyhow::anyhow!("physical native node/kind missing"))?;
            ensure!(matches.next().is_none(), "ambiguous physical native node");
            let timing = native
                .timing
                .ok_or_else(|| anyhow::anyhow!("physical timing unavailable"))?;
            ensure!(
                timing.sample_rate == professional.actual.rate
                    && timing.capacity_frames == professional.actual.capacity
                    && timing.period_frames.is_none_or(|v| v > 0),
                "physical native format/timing mismatch"
            );
            request.validate_observed(timing, native.shared_period)?;
        }
        if let Some(pinned) = &self.pinned {
            ensure!(
                sample.host_id == pinned.host_id
                    && sample.owner == pinned.owner
                    && sample.resource == pinned.resource,
                "physical pinned host/owner/resource changed"
            );
            if !final_joined {
                ensure!(
                    sample.plan == pinned.plan
                        && sample.timeline.map(|t| [t[0], t[1]])
                            == pinned.timeline.map(|t| [t[0], t[1]]),
                    "physical pinned plan/timeline changed"
                );
            }
            ensure!(
                sample.native.streams == pinned.native.streams,
                "physical prepared native metadata changed"
            );
            for node in [self.input, self.output] {
                let current = self.stream(sample, node)?;
                let old = self.stream(pinned, node)?;
                ensure!(
                    current.actual == old.actual && current.mode == old.mode,
                    "physical native format changed"
                );
            }
        }
        for current in &sample.streams {
            ensure!(
                stream_counts(current).iter().all(|v| *v != u64::MAX),
                "physical counter saturated"
            );
        }
        if let Some(current) = self.stream(sample, self.input)?.capture_quality {
            ensure!(
                capture_counts(current).iter().all(|v| *v != u64::MAX),
                "physical capture quality counter saturated"
            );
            if let Some(previous) = self.last_capture_quality {
                capture_delta(Some(current), Some(previous))?;
            }
        }
        if let Some(previous) = &self.previous {
            for node in [self.input, self.output] {
                ensure!(
                    stream_counts(self.stream(sample, node)?)
                        .iter()
                        .zip(stream_counts(self.stream(previous, node)?))
                        .all(|(current, old)| *current >= old),
                    "physical counter regressed"
                );
            }
            if !final_joined {
                ensure!(
                    sample.timeline.unwrap()[2] >= previous.timeline.unwrap()[2],
                    "physical timeline regressed"
                );
            }
        }
        Ok(())
    }
    fn quality_delta(
        &self,
        options: &Options,
        sample: &GraphObservation,
        baseline: &GraphObservation,
        startup: bool,
        quality: &mut QualityDiagnostics,
    ) -> Result<()> {
        for node in [self.input, self.output] {
            let stream = self.stream(sample, node)?;
            let old = self.stream(baseline, node)?;
            let baseline_capture = old.capture_quality;
            let current = [
                stream.timeouts,
                stream.dropped,
                stream.silent,
                stream.native_dropped,
                stream.native_silent,
            ];
            let old = [
                old.timeouts,
                old.dropped,
                old.silent,
                old.native_dropped,
                old.native_silent,
            ];
            for (index, name) in [
                "native_timeouts",
                "transport_dropped",
                "transport_silent",
                "native_dropped",
                "native_render_silent",
            ]
            .iter()
            .enumerate()
            {
                if startup && node == self.input && index == 2 {
                    continue;
                }
                let delta = if startup {
                    current[index]
                } else {
                    current[index]
                        .checked_sub(old[index])
                        .ok_or_else(|| anyhow::anyhow!("physical quality counter regressed"))?
                };
                if delta != 0 {
                    quality.fault(
                        options.validation,
                        &format!(
                            "physical {node:?} {name} {}={delta}",
                            if startup { "startup" } else { "steady_delta" }
                        ),
                    )?;
                }
            }
            if node == self.input {
                // Unknown baseline cannot establish a steady delta. Retain it
                // as unknown, but known cumulative faults still fail quality.
                let observed = capture_delta(stream.capture_quality, baseline_capture)?;
                let faults = if startup {
                    stream.capture_quality
                } else {
                    observed.or(stream.capture_quality)
                };
                if let Some(faults) = faults {
                    for (name, count) in [
                        "capture_discontinuities",
                        "capture_timestamp_errors",
                        "capture_silent_frames",
                    ]
                    .into_iter()
                    .zip(capture_counts(faults))
                    {
                        if count != 0 {
                            quality.fault(
                                options.validation,
                                &format!(
                                    "physical {node:?} {name} {}={count}",
                                    if startup {
                                        "startup"
                                    } else if observed.is_some() {
                                        "steady_delta"
                                    } else {
                                        "cumulative_without_known_baseline"
                                    }
                                ),
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn capture_counts(quality: NativeCaptureQuality) -> [u64; 3] {
    [
        quality.discontinuities,
        quality.timestamp_errors,
        quality.silent_frames,
    ]
}

fn capture_delta(
    current: Option<NativeCaptureQuality>,
    baseline: Option<NativeCaptureQuality>,
) -> Result<Option<NativeCaptureQuality>> {
    current
        .zip(baseline)
        .map(|(current, baseline)| {
            let delta = |current: u64, baseline: u64| {
                current
                    .checked_sub(baseline)
                    .ok_or_else(|| anyhow::anyhow!("physical capture quality counter regressed"))
            };
            Ok(NativeCaptureQuality {
                discontinuities: delta(current.discontinuities, baseline.discontinuities)?,
                timestamp_errors: delta(current.timestamp_errors, baseline.timestamp_errors)?,
                silent_frames: delta(current.silent_frames, baseline.silent_frames)?,
            })
        })
        .transpose()
}

fn stream_counts(s: &Stream) -> [u64; 8] {
    [
        s.callbacks,
        s.wakes,
        s.written,
        s.timeouts,
        s.dropped,
        s.silent,
        s.native_dropped,
        s.native_silent,
    ]
}

fn graph_value(s: &GraphObservation) -> Value {
    let capture = s
        .streams
        .iter()
        .find(|stream| stream.input)
        .and_then(|stream| stream.capture_quality);
    json!({"host_id":s.host_id,"host":format!("{:?}",s.host),"execution":format!("{:?}",s.execution),
        "owner_generation":s.owner,"resource_generation":s.resource,"desired_revision":s.desired,"active_revision":s.active,"plan_revision":s.plan,
        "timeline":s.timeline,"fault":s.fault,"professional_owner_generation":s.professional_owner,"professional_running":s.professional_running,
        "streams":s.streams,"native":s.native.raw,
        "graph_capture_discontinuities":capture.map(|v|v.discontinuities),
        "graph_capture_timestamp_errors":capture.map(|v|v.timestamp_errors),
        "graph_capture_silent_frames":capture.map(|v|v.silent_frames)})
}

pub(super) trait Operations {
    fn elapsed(&self) -> Duration;
    fn wait(&mut self);
    fn prepare_recorder(&mut self) -> Result<()>;
    fn activate_recorder(&mut self) -> Result<()>;
    fn recorder_running(&mut self) -> Result<bool>;
    fn open_graph(&mut self) -> Result<()>;
    fn graph_running(&mut self, quality: &mut QualityDiagnostics) -> Result<bool>;
    fn stop_recorder(&mut self) -> Result<()>;
    fn finish_graph(&mut self, quality: &mut QualityDiagnostics) -> Result<Value>;
    fn finish_recorder(&mut self, quality: &mut QualityDiagnostics) -> Result<Value>;
    fn evidence(&self) -> Value {
        Value::Null
    }
}

fn attempt<T>(name: &str, action: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(action))
        .map_err(|_| anyhow::anyhow!("{name} panicked"))?
}

fn latch_quality(
    operations: &impl Operations,
    quality: &QualityDiagnostics,
    first: &mut Option<Value>,
) -> Result<()> {
    if first.is_none() && quality.failure.is_some() {
        *first = Some(operations.evidence());
    }
    Ok(())
}

pub(super) fn collect(options: &Options, operations: &mut impl Operations) -> Value {
    let startup_base = Duration::from_secs(10);
    let startup_limit = startup_base + options.validation.progress_grace();
    let total_limit = startup_limit + Duration::from_secs(options.seconds);
    let mut quality = QualityDiagnostics::default();
    let mut complete = false;
    let mut failure = None;
    let mut first_failure = None;
    let mut startup_elapsed = None;
    let result = attempt("physical collection", || {
        operations.prepare_recorder()?;
        ensure!(
            operations.elapsed() < startup_limit,
            "physical startup deadline after recorder prepare"
        );
        operations.activate_recorder()?;
        loop {
            ensure!(
                operations.elapsed() < startup_limit,
                "physical startup deadline awaiting recorder"
            );
            if operations.recorder_running()? {
                break;
            }
            operations.wait();
        }
        operations.open_graph()?;
        loop {
            ensure!(
                operations.elapsed() < startup_limit,
                "physical startup deadline awaiting graph"
            );
            ensure!(
                operations.recorder_running()?,
                "recorder lost Running during graph startup"
            );
            let running = operations.graph_running(&mut quality);
            latch_quality(operations, &quality, &mut first_failure)?;
            if running? {
                break;
            }
            operations.wait();
        }
        let start = operations.elapsed();
        startup_elapsed = Some(start.as_secs_f64());
        if start >= startup_base {
            quality.fault(
                options.validation,
                "physical startup used correctness progress grace",
            )?;
            latch_quality(operations, &quality, &mut first_failure)?;
        }
        let steady_end = start + Duration::from_secs(options.seconds);
        ensure!(
            steady_end <= total_limit,
            "physical steady duration exceeds fixed total ceiling"
        );
        while operations.elapsed() < steady_end {
            ensure!(
                operations.recorder_running()?,
                "recorder lost Running during steady collection"
            );
            let running = operations.graph_running(&mut quality);
            latch_quality(operations, &quality, &mut first_failure)?;
            ensure!(running?, "graph lost steady readiness");
            operations.wait();
        }
        ensure!(
            operations.recorder_running()?,
            "recorder lost Running at terminal collection observation"
        );
        let terminal = operations.graph_running(&mut quality);
        latch_quality(operations, &quality, &mut first_failure)?;
        ensure!(
            terminal?,
            "graph lost readiness at terminal collection observation"
        );
        ensure!(
            operations.elapsed() <= total_limit,
            "physical total collection deadline exceeded"
        );
        complete = true;
        Ok(())
    });
    if let Err(error) = result {
        failure = Some(format!("{error:#}"));
        if first_failure.is_none() {
            first_failure = attempt("first failure evidence", || Ok(operations.evidence())).ok();
        }
    }
    let mut cleanup_complete = true;
    // EngineHost has no public asynchronous stop. Signal the recorder first,
    // then synchronously drop/join the host, then join/drain the recorder sink.
    if let Err(error) = attempt("stop recorder", || operations.stop_recorder()) {
        cleanup_complete = false;
        append(&mut failure, &format!("{error:#}"));
    }
    let graph = match attempt("finish graph", || operations.finish_graph(&mut quality)) {
        Ok(value) => Some(value),
        Err(error) => {
            cleanup_complete = false;
            append(&mut failure, &format!("{error:#}"));
            None
        }
    };
    if let Err(error) = attempt("final graph quality evidence", || {
        latch_quality(operations, &quality, &mut first_failure)
    }) {
        cleanup_complete = false;
        append(&mut failure, &format!("{error:#}"));
    }
    let recorder = match attempt("finish recorder", || {
        operations.finish_recorder(&mut quality)
    }) {
        Ok(value) => Some(value),
        Err(error) => {
            cleanup_complete = false;
            append(&mut failure, &format!("{error:#}"));
            None
        }
    };
    if let Err(error) = attempt("final recorder quality evidence", || {
        latch_quality(operations, &quality, &mut first_failure)
    }) {
        cleanup_complete = false;
        append(&mut failure, &format!("{error:#}"));
    }
    let evidence = match attempt("final evidence", || Ok(operations.evidence())) {
        Ok(value) => value,
        Err(error) => {
            cleanup_complete = false;
            append(&mut failure, &format!("{error:#}"));
            Value::Null
        }
    };
    let capture_quality_observed = evidence["capture_quality_observed_all_samples"] == true;
    let unavailable_graph_counters: &[&str] = if capture_quality_observed {
        &[]
    } else {
        &[
            "capture_discontinuities",
            "capture_timestamp_errors",
            "capture_silent_frames",
        ]
    };
    json!({"collection_complete":complete,"cleanup_complete":cleanup_complete,
        "quality_passed":complete && cleanup_complete && failure.is_none() && quality.failure.is_none(),"quality_diagnostics":quality,
        "quality_scope":"available graph counters and independent recorder only; native capture flags are not signal-loss proof; no complete duplicated/missing PCM or calibrated physical continuity proof",
        "quality_complete":false,"qualified":false,"physical_boundary_verified":false,
        "graph_capture_quality_observed_all_samples":capture_quality_observed,
        "unavailable_graph_counters":unavailable_graph_counters,
        "failure":failure,"first_failure":first_failure,"startup_elapsed_seconds":startup_elapsed,
        "startup_ceiling_seconds":startup_limit.as_secs_f64(),"total_collection_ceiling_seconds":total_limit.as_secs_f64(),
        "graph_final":graph,"recorder_final":recorder,"observations":evidence})
}

fn append(failure: &mut Option<String>, message: &str) {
    if let Some(existing) = failure {
        existing.push_str("; ");
        existing.push_str(message);
    } else {
        *failure = Some(message.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::validation_policy::{ValidationMode, ValidationPolicy};
    use super::*;
    use moiren_windows_audio::latency::{LatencyBudgetPolicy, RenderQueuePolicy, TelemetryMode};

    fn options() -> Options {
        Options {
            input_endpoint_id: "source".into(),
            output_endpoint_id: "output".into(),
            recorder_config: "config.json".into(),
            artifact_dir: "unused".into(),
            seconds: 2,
            capture: Default::default(),
            render: Default::default(),
            input_budget: LatencyBudgetPolicy::Compatible,
            render_queue: RenderQueuePolicy::FullCapacity,
            engine_block_frames: 64,
            validation: ValidationPolicy::default(),
            telemetry: TelemetryMode::Summary,
            dry_run: false,
        }
    }

    #[derive(Default)]
    struct Fake {
        now: Duration,
        log: Vec<&'static str>,
        recorder_ready: Duration,
        graph_ready: Duration,
        prepare_cost: Duration,
        fail: Option<&'static str>,
        panic_at: Option<&'static str>,
        soft_fault: bool,
        capture_quality_observed: bool,
    }
    impl Fake {
        fn step(&mut self, name: &'static str) -> Result<()> {
            self.log.push(name);
            assert_ne!(self.panic_at, Some(name), "injected {name} panic");
            anyhow::ensure!(self.fail != Some(name), "injected {name} failure");
            Ok(())
        }
    }
    impl Operations for Fake {
        fn elapsed(&self) -> Duration {
            self.now
        }
        fn wait(&mut self) {
            self.now += Duration::from_millis(100);
        }
        fn prepare_recorder(&mut self) -> Result<()> {
            self.now += self.prepare_cost;
            self.step("prepare")
        }
        fn activate_recorder(&mut self) -> Result<()> {
            self.step("activate")
        }
        fn recorder_running(&mut self) -> Result<bool> {
            self.step("recorder_check")?;
            Ok(self.now >= self.recorder_ready)
        }
        fn open_graph(&mut self) -> Result<()> {
            self.step("open_graph")
        }
        fn graph_running(&mut self, quality: &mut QualityDiagnostics) -> Result<bool> {
            self.step("graph_check")?;
            if self.soft_fault {
                quality.fault(
                    ValidationPolicy {
                        mode: ValidationMode::Correctness,
                        ..Default::default()
                    },
                    "injected quality",
                )?;
            }
            Ok(self.now >= self.graph_ready)
        }
        fn stop_recorder(&mut self) -> Result<()> {
            self.step("stop_recorder")
        }
        fn finish_graph(&mut self, _: &mut QualityDiagnostics) -> Result<Value> {
            self.step("join_graph")?;
            Ok(json!({"closed":true}))
        }
        fn finish_recorder(&mut self, _: &mut QualityDiagnostics) -> Result<Value> {
            self.step("join_recorder")?;
            Ok(json!({"joined":true}))
        }
        fn evidence(&self) -> Value {
            json!({"elapsed":self.now.as_secs_f64(),"capture_quality_observed_all_samples":self.capture_quality_observed})
        }
    }

    #[test]
    fn physical_session_recorder_running_precedes_graph_open_and_steady_has_full_duration() {
        let mut io = Fake {
            recorder_ready: Duration::from_secs(1),
            graph_ready: Duration::from_secs(2),
            ..Default::default()
        };
        let report = collect(&options(), &mut io);
        assert_eq!(report["collection_complete"], true, "{report}");
        assert_eq!(report["cleanup_complete"], true, "{report}");
        let open = io
            .log
            .iter()
            .position(|name| *name == "open_graph")
            .unwrap();
        assert!(
            io.log[..open]
                .iter()
                .filter(|name| **name == "recorder_check")
                .count()
                >= 10
        );
        assert_eq!(io.now, Duration::from_secs(4));
        assert_eq!(
            &io.log[io.log.len() - 3..],
            &["stop_recorder", "join_graph", "join_recorder"]
        );
    }

    #[test]
    fn physical_session_startup_budget_is_not_restarted_after_prepare_or_between_owners() {
        let mut io = Fake {
            prepare_cost: Duration::from_secs(9),
            recorder_ready: Duration::from_millis(9500),
            graph_ready: Duration::from_secs(11),
            ..Default::default()
        };
        let report = collect(&options(), &mut io);
        assert_eq!(report["collection_complete"], false);
        assert!(
            report["failure"].as_str().unwrap().contains("startup"),
            "{report}"
        );
        assert!(io.now <= Duration::from_secs(10));
    }

    #[test]
    fn physical_session_correctness_grace_does_not_expand_steady_duration() {
        let mut options = options();
        options.validation.mode = ValidationMode::Correctness;
        options.validation.jitter_tolerance_ms = 3000;
        let mut io = Fake {
            recorder_ready: Duration::from_secs(11),
            graph_ready: Duration::from_secs(12),
            ..Default::default()
        };
        let report = collect(&options, &mut io);
        assert_eq!(report["collection_complete"], true, "{report}");
        assert_eq!(io.now, Duration::from_secs(14));
        assert_eq!(report["quality_passed"], false);
    }

    #[test]
    fn physical_session_every_failure_and_panic_still_stops_recorder_before_both_joins() {
        for stage in [
            "prepare",
            "activate",
            "recorder_check",
            "open_graph",
            "graph_check",
            "stop_recorder",
            "join_graph",
            "join_recorder",
        ] {
            for panic in [false, true] {
                let mut io = Fake {
                    fail: (!panic).then_some(stage),
                    panic_at: panic.then_some(stage),
                    ..Default::default()
                };
                let report = collect(&options(), &mut io);
                assert!(report["failure"].is_string(), "{stage}: {report}");
                let stop = io
                    .log
                    .iter()
                    .position(|name| *name == "stop_recorder")
                    .unwrap();
                let graph = io
                    .log
                    .iter()
                    .position(|name| *name == "join_graph")
                    .unwrap();
                let recorder = io
                    .log
                    .iter()
                    .position(|name| *name == "join_recorder")
                    .unwrap();
                assert!(stop < graph && graph < recorder, "{stage}: {:?}", io.log);
            }
        }
    }

    fn graph() -> (GraphTracker, GraphObservation) {
        use moiren_core::graph::{LogicalGraph, NodeKind};
        let mut graph = LogicalGraph::new();
        let input = graph.create_node(NodeKind::Source, 2).unwrap();
        let output = graph.create_node(NodeKind::Sink, 2).unwrap();
        let stream = |node, input| Stream {
            node: format!("{node:?}"),
            input,
            master: !input,
            mode: DeviceMode::Shared,
            actual: NativeStreamInfoView {
                rate: 48_000,
                channels: 2,
                format: SampleFormat::Float32,
                capacity: 256,
            },
            callbacks: 0,
            wakes: 10,
            written: if input { 0 } else { 512 },
            timeouts: 0,
            dropped: 0,
            silent: 0,
            native_dropped: 0,
            native_silent: 0,
            capture_quality: None,
        };
        let native = |node, kind| {
            let timing = Some(NativeStreamTiming {
                sample_rate: 48_000,
                capacity_frames: 256,
                period_frames: Some(256),
            });
            NativeStream {
                node,
                kind,
                timing,
                shared_period: None,
                metadata: json!({
                    "node":format!("{node:?}"),
                    "kind":if kind == NativeKind::PhysicalInput {"physical_input"} else {"master_output"},
                    "timing":timing,"shared_period":null,
                    "render_queue":(kind == NativeKind::MasterOutput).then(||json!({
                        "capacity_frames":256,"target_frames":256,"deadline_budget_frames":null
                    })),"input_queue":null,"virtual_budget":null,
                }),
            }
        };
        let native_streams = vec![
            native(input, NativeKind::PhysicalInput),
            native(output, NativeKind::MasterOutput),
        ];
        let raw = json!({"owner_generation":1,"running":true,
            "streams":native_streams.iter().map(|s|&s.metadata).collect::<Vec<_>>(),
            "bridges":[],"boundary_probes":[],"render_padding":[],"process_delivery":[]});
        (
            GraphTracker::new(input, output),
            GraphObservation {
                host_id: "host1".into(),
                host: HostState::Open,
                execution: ExecutionState::Running,
                owner: 1,
                resource: 1,
                desired: 1,
                active: Some(1),
                plan: Some(1),
                timeline: Some([1, 1, 512]),
                fault: None,
                professional_owner: 1,
                professional_running: true,
                streams: vec![stream(input, true), stream(output, false)],
                native: NativeObservation {
                    owner_generation: 1,
                    running: true,
                    streams: native_streams,
                    raw,
                },
            },
        )
    }

    fn capture_quality(
        discontinuities: u64,
        timestamp_errors: u64,
        silent_frames: u64,
    ) -> NativeCaptureQuality {
        NativeCaptureQuality {
            discontinuities,
            timestamp_errors,
            silent_frames,
        }
    }

    #[test]
    fn physical_capture_quality_known_zero_and_unknown_are_distinct() {
        let (mut tracker, mut sample) = graph();
        let mut quality = QualityDiagnostics::default();
        tracker
            .observe(&options(), &sample, Duration::ZERO, &mut quality)
            .unwrap();
        assert_eq!(
            tracker.startup.as_ref().unwrap()["graph_capture_timestamp_errors"],
            Value::Null
        );
        sample.streams[0].capture_quality = Some(NativeCaptureQuality::default());
        let raw = graph_value(&sample);
        assert_eq!(raw["graph_capture_discontinuities"], 0);
        assert_eq!(raw["graph_capture_timestamp_errors"], 0);
        assert_eq!(raw["graph_capture_silent_frames"], 0);
        sample.streams[0].capture_quality = Some(capture_quality(7, 11, (1 << 53) + 13));
        assert_eq!(
            graph_value(&sample)["graph_capture_silent_frames"].as_u64(),
            Some((1 << 53) + 13)
        );
    }

    #[test]
    fn physical_capture_quality_startup_and_steady_faults_follow_validation_policy() {
        for correctness in [false, true] {
            for startup in [false, true] {
                for counter in 0..3 {
                    let (mut tracker, mut sample) = graph();
                    let mut options = options();
                    if correctness {
                        options.validation.mode = ValidationMode::Correctness;
                    }
                    let mut quality = QualityDiagnostics::default();
                    sample.streams[0].capture_quality = Some(NativeCaptureQuality::default());
                    if !startup {
                        tracker
                            .observe(&options, &sample, Duration::ZERO, &mut quality)
                            .unwrap();
                    }
                    sample.streams[0].capture_quality = Some(match counter {
                        0 => capture_quality(1, 0, 0),
                        1 => capture_quality(0, 1, 0),
                        _ => capture_quality(0, 0, 1),
                    });
                    assert_eq!(
                        tracker
                            .observe(&options, &sample, Duration::from_millis(100), &mut quality)
                            .is_ok(),
                        correctness,
                        "correctness={correctness}, startup={startup}, counter={counter}"
                    );
                    assert!(quality.failure.is_some());
                    assert!(quality.failure.as_ref().unwrap().contains(if startup {
                        "startup"
                    } else {
                        "steady_delta"
                    }));
                }
            }
        }
    }

    #[test]
    fn physical_capture_quality_final_join_preserves_startup_and_checked_deltas() {
        for correctness in [false, true] {
            let (mut tracker, mut sample) = graph();
            let mut options = options();
            options.validation.mode = ValidationMode::Correctness;
            let mut quality = QualityDiagnostics::default();
            sample.streams[0].capture_quality = Some(capture_quality(2, 3, 5));
            tracker
                .observe(&options, &sample, Duration::ZERO, &mut quality)
                .unwrap();
            assert_eq!(
                tracker.startup.as_ref().unwrap()["graph_capture_discontinuities"],
                2
            );
            let mut final_quality = QualityDiagnostics::default();
            sample.host = HostState::Closed;
            sample.execution = ExecutionState::Dormant;
            sample.active = None;
            sample.plan = None;
            sample.timeline = None;
            sample.professional_running = false;
            sample.native.running = false;
            sample.streams[0].capture_quality = Some(capture_quality(7, 11, 13));
            options.validation.mode = if correctness {
                ValidationMode::Correctness
            } else {
                ValidationMode::LatencySensitive
            };
            assert_eq!(
                tracker
                    .finish(&options, &sample, &mut final_quality)
                    .is_ok(),
                correctness
            );
            let final_value = tracker.final_observation.as_ref().unwrap();
            assert_eq!(final_value["graph_capture_discontinuities"], 7);
            assert_eq!(
                final_value["steady_deltas"][0]["capture_quality"],
                json!({"discontinuities":5,"timestamp_errors":8,"silent_frames":8})
            );
            assert!(final_quality.failure.is_some());
            assert!(tracker.capture_quality_observed_all_samples());
        }
    }

    #[test]
    fn physical_capture_quality_regression_and_saturation_are_hard_even_across_unknown() {
        for counter in 0..3 {
            for unknown_between in [false, true] {
                let (mut tracker, mut sample) = graph();
                let mut options = options();
                options.validation.mode = ValidationMode::Correctness;
                let mut quality = QualityDiagnostics::default();
                sample.streams[0].capture_quality = Some(capture_quality(7, 11, 13));
                tracker
                    .observe(&options, &sample, Duration::ZERO, &mut quality)
                    .unwrap();
                if unknown_between {
                    sample.streams[0].capture_quality = None;
                    tracker
                        .observe(&options, &sample, Duration::from_millis(10), &mut quality)
                        .unwrap();
                }
                sample.streams[0].capture_quality = Some(match counter {
                    0 => capture_quality(6, 11, 13),
                    1 => capture_quality(7, 10, 13),
                    _ => capture_quality(7, 11, 12),
                });
                let error = tracker
                    .observe(&options, &sample, Duration::from_millis(20), &mut quality)
                    .unwrap_err();
                assert!(format!("{error:#}").contains("capture quality counter regressed"));
            }
            let (mut tracker, mut sample) = graph();
            sample.streams[0].capture_quality = Some(match counter {
                0 => capture_quality(u64::MAX, 0, 0),
                1 => capture_quality(0, u64::MAX, 0),
                _ => capture_quality(0, 0, u64::MAX),
            });
            assert!(
                tracker
                    .observe(
                        &options(),
                        &sample,
                        Duration::ZERO,
                        &mut QualityDiagnostics::default()
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn physical_capture_quality_unknown_baseline_does_not_invent_steady_zero() {
        let (mut tracker, mut sample) = graph();
        let mut options = options();
        options.validation.mode = ValidationMode::Correctness;
        let mut quality = QualityDiagnostics::default();
        tracker
            .observe(&options, &sample, Duration::ZERO, &mut quality)
            .unwrap();
        sample.streams[0].capture_quality = Some(capture_quality(1, 2, 3));
        sample.host = HostState::Closed;
        sample.execution = ExecutionState::Dormant;
        sample.active = None;
        sample.plan = None;
        sample.timeline = None;
        sample.professional_running = false;
        sample.native.running = false;
        tracker.finish(&options, &sample, &mut quality).unwrap();
        let final_value = tracker.final_observation.as_ref().unwrap();
        assert_eq!(
            final_value["steady_deltas"][0]["capture_quality"],
            Value::Null
        );
        assert_eq!(final_value["graph_capture_timestamp_errors"], 2);
        assert!(
            quality.failure.is_some(),
            "known cumulative faults still remain visible"
        );
        assert!(!tracker.capture_quality_observed_all_samples());
    }

    #[test]
    fn physical_capture_quality_unknown_poll_or_final_stays_incomplete() {
        for unknown_final in [false, true] {
            let (mut tracker, mut sample) = graph();
            let mut quality = QualityDiagnostics::default();
            sample.streams[0].capture_quality = Some(NativeCaptureQuality::default());
            tracker
                .observe(&options(), &sample, Duration::ZERO, &mut quality)
                .unwrap();
            sample.streams[0].capture_quality = None;
            if !unknown_final {
                tracker
                    .observe(&options(), &sample, Duration::from_millis(10), &mut quality)
                    .unwrap();
                sample.streams[0].capture_quality = Some(NativeCaptureQuality::default());
            }
            sample.host = HostState::Closed;
            sample.execution = ExecutionState::Dormant;
            sample.active = None;
            sample.plan = None;
            sample.timeline = None;
            sample.professional_running = false;
            sample.native.running = false;
            tracker.finish(&options(), &sample, &mut quality).unwrap();
            assert!(!tracker.capture_quality_observed_all_samples());
            if unknown_final {
                assert_eq!(
                    tracker.final_observation.as_ref().unwrap()["steady_deltas"][0]["capture_quality"],
                    Value::Null
                );
            }
        }
    }

    #[test]
    fn physical_capture_quality_report_distinguishes_available_flags_from_full_quality() {
        for known in [false, true] {
            let mut io = Fake {
                capture_quality_observed: known,
                ..Default::default()
            };
            let report = collect(&options(), &mut io);
            assert_eq!(report["graph_capture_quality_observed_all_samples"], known);
            assert_eq!(
                report["unavailable_graph_counters"]
                    .as_array()
                    .unwrap()
                    .len(),
                if known { 0 } else { 3 }
            );
            assert_eq!(report["quality_complete"], false);
            assert_eq!(report["qualified"], false);
            assert!(
                report["quality_scope"]
                    .as_str()
                    .unwrap()
                    .contains("not signal-loss proof")
            );
        }
    }

    #[test]
    fn physical_session_admission_refuses_wrong_nodes_generation_mode_and_period() {
        for variant in 0..8 {
            let (mut tracker, mut sample) = graph();
            let mut options = options();
            match variant {
                0 => sample.native.owner_generation = 2,
                1 => sample.professional_owner = 2,
                2 => sample.streams[0].node = "wrong".into(),
                3 => sample.native.streams.swap(0, 1),
                4 => sample.streams[0].mode = DeviceMode::Exclusive,
                5 => options.capture.period_frames = Some(128),
                6 => sample.native.streams[0].timing = None,
                _ => sample.native.streams[0].kind = NativeKind::Unexpected,
            }
            if variant == 3 {
                sample.native.streams[0].node = sample.native.streams[1].node;
            }
            assert!(
                tracker
                    .observe(
                        &options,
                        &sample,
                        Duration::ZERO,
                        &mut QualityDiagnostics::default()
                    )
                    .is_err(),
                "variant {variant}"
            );
        }
    }

    #[test]
    fn physical_session_priming_silence_is_startup_not_steady_and_unknowns_stay_unknown() {
        let (mut tracker, mut sample) = graph();
        let mut quality = QualityDiagnostics::default();
        sample.streams[0].silent = 512;
        assert!(
            !tracker
                .observe(&options(), &sample, Duration::ZERO, &mut quality)
                .unwrap()
        );
        sample.timeline = Some([1, 1, 1024]);
        sample.streams[0].wakes += 1;
        sample.streams[1].written += 512;
        assert!(
            tracker
                .observe(
                    &options(),
                    &sample,
                    Duration::from_millis(100),
                    &mut quality
                )
                .unwrap()
        );
        assert!(quality.failure.is_none());
        assert_eq!(
            tracker.startup.as_ref().unwrap()["graph_capture_timestamp_errors"],
            Value::Null
        );
        sample.streams[0].silent += 1;
        assert!(
            tracker
                .observe(
                    &options(),
                    &sample,
                    Duration::from_millis(200),
                    &mut quality
                )
                .is_err()
        );
    }

    #[test]
    fn physical_session_quality_can_continue_but_identity_format_counter_regression_cannot() {
        for variant in 0..5 {
            let (mut tracker, mut sample) = graph();
            let mut options = options();
            options.validation.mode = ValidationMode::Correctness;
            let mut quality = QualityDiagnostics::default();
            tracker
                .observe(&options, &sample, Duration::ZERO, &mut quality)
                .unwrap();
            match variant {
                0 => sample.owner += 1,
                1 => sample.streams[0].actual.rate = 44_100,
                2 => sample.streams[0].wakes = 0,
                3 => sample.timeline = Some([2, 1, 1024]),
                _ => sample.streams[0].dropped += 1,
            }
            let result =
                tracker.observe(&options, &sample, Duration::from_millis(100), &mut quality);
            assert_eq!(
                result.is_ok(),
                variant == 4,
                "variant {variant}: {result:?}"
            );
            if variant == 4 {
                assert!(quality.failure.is_some());
            }
        }
    }

    #[test]
    fn physical_session_progress_requires_input_activity_output_and_timeline_with_bounded_grace() {
        for correctness in [false, true] {
            let (mut tracker, sample) = graph();
            let mut options = options();
            if correctness {
                options.validation.mode = ValidationMode::Correctness;
            }
            let mut quality = QualityDiagnostics::default();
            tracker
                .observe(&options, &sample, Duration::ZERO, &mut quality)
                .unwrap();
            let mut progressed = sample.clone();
            progressed.timeline = Some([1, 1, 1024]);
            progressed.streams[1].written += 512;
            assert_eq!(
                tracker
                    .observe(&options, &progressed, Duration::from_secs(1), &mut quality)
                    .is_ok(),
                correctness
            );
            assert!(
                tracker
                    .observe(&options, &progressed, Duration::from_secs(4), &mut quality)
                    .is_err()
            );
        }
    }

    #[test]
    fn physical_session_final_joined_observation_keeps_identity_and_trailing_quality() {
        for variant in 0..4 {
            let (mut tracker, mut sample) = graph();
            let mut quality = QualityDiagnostics::default();
            tracker
                .observe(&options(), &sample, Duration::ZERO, &mut quality)
                .unwrap();
            sample.host = HostState::Closed;
            sample.execution = ExecutionState::Dormant;
            sample.active = None;
            sample.plan = None;
            sample.timeline = None;
            sample.professional_running = false;
            sample.native.running = false;
            match variant {
                0 => {}
                1 => sample.owner = 2,
                2 => sample.fault = Some("stop failed".into()),
                _ => sample.streams[1].native_silent += 1,
            }
            assert_eq!(
                tracker.finish(&options(), &sample, &mut quality).is_ok(),
                variant == 0,
                "variant {variant}"
            );
            assert!(tracker.final_observation.is_some());
        }
    }

    #[test]
    fn physical_session_initial_plan_must_match_first_timeline_revision() {
        let (mut tracker, mut sample) = graph();
        sample.plan = Some(2);
        assert!(
            tracker
                .observe(
                    &options(),
                    &sample,
                    Duration::ZERO,
                    &mut QualityDiagnostics::default()
                )
                .is_err()
        );
    }

    #[test]
    fn physical_session_final_evidence_exports_startup_and_steady_delta_separately() {
        let (mut tracker, mut sample) = graph();
        let mut options = options();
        options.validation.mode = ValidationMode::Correctness;
        let mut quality = QualityDiagnostics::default();
        sample.streams[0].silent = 512;
        tracker
            .observe(&options, &sample, Duration::ZERO, &mut quality)
            .unwrap();
        sample.timeline = Some([1, 1, 1024]);
        sample.streams[0].wakes += 1;
        sample.streams[1].written += 512;
        tracker
            .observe(&options, &sample, Duration::from_millis(100), &mut quality)
            .unwrap();
        sample.streams[0].silent += 3;
        sample.host = HostState::Closed;
        sample.professional_running = false;
        sample.native.running = false;
        tracker.finish(&options, &sample, &mut quality).unwrap();
        assert_eq!(
            tracker.startup.as_ref().unwrap()["streams"][0]["silent"],
            512
        );
        assert_eq!(
            tracker.final_observation.as_ref().unwrap()["steady_deltas"][0]["silent"],
            3
        );
    }

    #[test]
    fn physical_session_failed_collection_never_reports_available_quality_passed() {
        let mut io = Fake {
            fail: Some("prepare"),
            ..Default::default()
        };
        let report = collect(&options(), &mut io);
        assert_eq!(report["quality_passed"], false);
    }

    #[test]
    fn physical_session_correctness_retains_first_quality_observation_not_later_final() {
        let mut options = options();
        options.validation.mode = ValidationMode::Correctness;
        let mut io = Fake {
            soft_fault: true,
            ..Default::default()
        };
        let report = collect(&options, &mut io);
        assert_eq!(report["collection_complete"], true);
        assert_eq!(report["first_failure"]["elapsed"], 0.0);
        assert_eq!(report["observations"]["elapsed"], 2.0);
    }

    struct TrackedIo {
        io: Fake,
        tracker: GraphTracker,
        sample: GraphObservation,
        options: Options,
        recover: bool,
        terminal: Duration,
    }
    impl Operations for TrackedIo {
        fn elapsed(&self) -> Duration {
            self.io.now
        }
        fn wait(&mut self) {
            self.io.now = if self.io.now.is_zero() {
                Duration::from_millis(900)
            } else {
                self.terminal
            };
        }
        fn prepare_recorder(&mut self) -> Result<()> {
            self.io.prepare_recorder()
        }
        fn activate_recorder(&mut self) -> Result<()> {
            self.io.activate_recorder()
        }
        fn recorder_running(&mut self) -> Result<bool> {
            self.io.recorder_running()
        }
        fn open_graph(&mut self) -> Result<()> {
            self.io.open_graph()
        }
        fn graph_running(&mut self, quality: &mut QualityDiagnostics) -> Result<bool> {
            if self.recover && self.io.now == self.terminal {
                self.sample.streams[0].wakes += 1;
                self.sample.streams[1].written += 512;
                self.sample.timeline.as_mut().unwrap()[2] += 512;
            }
            self.tracker
                .observe(&self.options, &self.sample, self.io.now, quality)
        }
        fn stop_recorder(&mut self) -> Result<()> {
            self.io.stop_recorder()
        }
        fn finish_graph(&mut self, quality: &mut QualityDiagnostics) -> Result<Value> {
            self.io.step("join_graph")?;
            self.sample.host = HostState::Closed;
            self.sample.native.running = false;
            self.sample.professional_running = false;
            self.tracker.finish(&self.options, &self.sample, quality)?;
            Ok(json!({"joined":true}))
        }
        fn finish_recorder(&mut self, quality: &mut QualityDiagnostics) -> Result<Value> {
            self.io.finish_recorder(quality)
        }
        fn evidence(&self) -> Value {
            json!({"latest":self.tracker.latest,"elapsed":self.io.now.as_secs_f64()})
        }
    }

    #[test]
    fn physical_session_terminal_stall_and_late_recovery_cannot_skip_expired_interval() {
        for correctness in [false, true] {
            for recover in [false, true] {
                let mut options = options();
                if correctness {
                    options.validation.mode = ValidationMode::Correctness;
                }
                options.seconds = if correctness { 5 } else { 2 };
                let (tracker, sample) = graph();
                let terminal = Duration::from_millis(if correctness { 5100 } else { 2100 });
                let mut io = TrackedIo {
                    io: Fake::default(),
                    tracker,
                    sample,
                    options: options.clone(),
                    recover,
                    terminal,
                };
                let report = collect(&options, &mut io);
                assert_eq!(
                    report["collection_complete"], false,
                    "correctness={correctness} recovered={recover}: {report}"
                );
                assert!(
                    report["quality_diagnostics"]["failure"].is_string(),
                    "{report}"
                );
                assert_eq!(
                    &io.io.log[io.io.log.len() - 3..],
                    &["stop_recorder", "join_graph", "join_recorder"]
                );
            }
        }
    }

    #[test]
    fn physical_session_raw_native_evidence_survives_projection_without_dropping_metadata() {
        let (mut tracker, mut sample) = graph();
        let expected = sample.native.raw.clone();
        tracker
            .observe(
                &options(),
                &sample,
                Duration::ZERO,
                &mut QualityDiagnostics::default(),
            )
            .unwrap();
        assert_eq!(tracker.latest.as_ref().unwrap()["native"], expected);
        sample.native.streams[1].metadata["render_queue"]["target_frames"] = json!(255);
        assert!(
            tracker
                .observe(
                    &options(),
                    &sample,
                    Duration::from_millis(100),
                    &mut QualityDiagnostics::default(),
                )
                .is_err()
        );
    }
}
