use super::{
    NativeObservations, NativeStreamKind, NativeStreamObservation,
    preparation::{Diagnostics, capture_error, render_error},
    resources::Resources,
};
use crate::{
    ActivationGate, StopSignal,
    capture::{CaptureReport, CaptureSession, CaptureStatus},
    latency::RenderQueuePolicy,
    render::{
        DemandRenderer, PreparedNativeRender, RenderReport, RenderSession, RenderStatus,
        TimelineObserver,
    },
    virtual_owner::VirtualSessions,
};
use moiren_core::graph::NodeId;
use moiren_engine::{
    boundary::AudioReader,
    compiler::CompiledBindings,
    host::{
        AttachFailure, BackendError, BackendOwner, CancellationToken, OwnerReturn, OwnerStatus,
        OwnerTimeline, PreparedTransport,
    },
    runtime::Engine,
};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct WindowsTransport {
    pub(super) native: Option<PreparedNativeRender>,
    pub(super) captures: Vec<CaptureSession>,
    pub(super) followers: Vec<RenderSession>,
    pub(super) sinks: Vec<NodeId>,
    pub(super) observations: NativeObservations,
    pub(super) streams: Vec<NativeStreamObservation>,
    pub(super) bridges: Vec<(NodeId, crate::clock_bridge::BridgeObserver)>,
    pub(super) probes: Vec<(
        NodeId,
        NativeStreamKind,
        std::sync::Arc<crate::virtual_owner::BoundaryProbe>,
    )>,
    pub(super) output: Option<AudioReader<f32>>,
    pub(super) sink: NodeId,
    pub(super) stop: StopSignal,
    pub(super) gate: ActivationGate,
    pub(super) generation: u64,
    pub(super) queue: RenderQueuePolicy,
    pub(super) diagnostics: Diagnostics,
    pub(super) resources: Arc<Resources>,
    pub(super) virtual_sessions: VirtualSessions,
}

impl Drop for WindowsTransport {
    fn drop(&mut self) {
        if self.native.is_some()
            || !self.captures.is_empty()
            || !self.followers.is_empty()
            || !self.virtual_sessions.is_empty()
        {
            // A compiler rejection may drop an unattached transport. Reclaim
            // all owners here too, retaining their reports on the control side.
            self.cancel_and_report(None, &BackendError::Cancelled);
        }
    }
}

impl WindowsTransport {
    fn cancel_and_report(&mut self, report: Option<RenderReport>, failure: &BackendError) {
        let _ = self.stop.request_stop();
        let _ = self.virtual_sessions.stop_and_join();
        let render = report.or_else(|| {
            self.native
                .take()
                .and_then(|native| native.cancel_with_report().ok())
        });
        let followers = std::mem::take(&mut self.followers)
            .into_iter()
            .filter_map(|follower| follower.join().ok())
            .collect();
        self.observations.clear(self.generation);
        let capture = std::mem::take(&mut self.captures)
            .into_iter()
            .filter_map(|capture| capture.join().ok())
            .collect();
        save(
            &self.diagnostics,
            NativeReport {
                render,
                prepared: Vec::new(),
                followers,
                capture,
                failure: Some(failure.to_string()),
            },
        );
    }
}

impl PreparedTransport<f32> for WindowsTransport {
    fn attach(
        mut self: Box<Self>,
        engine: Engine<f32>,
        bindings: &CompiledBindings,
        cancel: CancellationToken,
    ) -> Result<Box<dyn BackendOwner<f32>>, Box<AttachFailure<f32>>> {
        if cancel.is_cancelled() {
            self.cancel_and_report(None, &BackendError::Cancelled);
            return Err(Box::new(AttachFailure {
                engine,
                error: BackendError::Cancelled,
            }));
        }
        let Some(sink) = bindings.node(self.sink) else {
            let error = BackendError::Unsupported(
                "transport does not match the compiled output sink".into(),
            );
            self.cancel_and_report(None, &error);
            return Err(Box::new(AttachFailure { engine, error }));
        };
        let protected: Option<Vec<_>> =
            self.sinks.iter().map(|&node| bindings.node(node)).collect();
        let Some(protected) = protected else {
            let error = BackendError::Unsupported(
                "transport sink set does not match compiled outputs".into(),
            );
            self.cancel_and_report(None, &error);
            return Err(Box::new(AttachFailure { engine, error }));
        };
        if !protected.contains(&sink) {
            let error =
                BackendError::Unsupported("master sink missing from transport sink set".into());
            self.cancel_and_report(None, &error);
            return Err(Box::new(AttachFailure { engine, error }));
        }
        let renderer = match DemandRenderer::new_managed_outputs_returning(
            engine,
            self.output.take().expect("prepared output consumer"),
            &protected,
            self.generation,
        ) {
            Ok(renderer) => renderer,
            Err(failure) => {
                let error = render_error(failure.error);
                self.cancel_and_report(None, &error);
                return Err(Box::new(AttachFailure {
                    engine: failure.engine,
                    error,
                }));
            }
        };
        let timeline = renderer.timeline_observer();
        let prepared = match self
            .native
            .take()
            .expect("prepared native transport")
            .attach_returning(renderer, self.queue)
        {
            Ok(prepared) => prepared,
            Err(failure) => {
                let error = render_error(failure.error);
                self.cancel_and_report(failure.report, &error);
                let (engine, _) = failure.renderer.into_parts();
                return Err(Box::new(AttachFailure { engine, error }));
            }
        };
        let render = prepared.into_session();
        let stop = self.stop.clone();
        if let Err(error) = cancel.on_cancel(move || {
            let _ = stop.request_stop();
        }) {
            let _ = self.stop.request_stop();
            let exit = render
                .join_with_renderer()
                .expect("native loop catches panic while retaining renderer");
            self.cancel_and_report(Some(exit.report), &error);
            let (engine, _) = exit.renderer.into_parts();
            return Err(Box::new(AttachFailure { engine, error }));
        }
        self.observations.publish(
            self.generation,
            std::mem::take(&mut self.streams),
            std::mem::take(&mut self.bridges),
            std::mem::take(&mut self.probes),
        );
        Ok(Box::new(WindowsOwner {
            render,
            followers: std::mem::take(&mut self.followers),
            observations: self.observations.clone(),
            generation: self.generation,
            _observation_lease: self.observations.lease(self.generation),
            captures: std::mem::take(&mut self.captures),
            stop: self.stop.clone(),
            timeline,
            diagnostics: self.diagnostics.clone(),
            resources: self.resources.clone(),
            reported_failure: AtomicBool::new(false),
            gate: self.gate.clone(),
            cancellation: cancel,
            virtual_sessions: std::mem::take(&mut self.virtual_sessions),
        }))
    }
}

struct WindowsOwner {
    _observation_lease: super::observations::ObservationLease,
    render: RenderSession,
    captures: Vec<CaptureSession>,
    followers: Vec<RenderSession>,
    observations: NativeObservations,
    generation: u64,
    stop: StopSignal,
    timeline: TimelineObserver,
    diagnostics: Diagnostics,
    resources: Arc<Resources>,
    reported_failure: AtomicBool,
    gate: ActivationGate,
    cancellation: CancellationToken,
    virtual_sessions: VirtualSessions,
}

impl BackendOwner<f32> for WindowsOwner {
    fn activate(&self) -> Result<(), BackendError> {
        if self.cancellation.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        if let Err(error) = self.virtual_sessions.activate() {
            let _ = self.stop.request_stop();
            return Err(error);
        }
        if let Err(error) = self.gate.activate() {
            self.virtual_sessions.cancel();
            let _ = self.stop.request_stop();
            return Err(BackendError::Failed(error.to_string()));
        }
        Ok(())
    }

    fn cancel(&self) {
        self.observations.clear(self.generation);
        self.virtual_sessions.cancel();
        let _ = self.stop.request_stop();
    }

    fn status(&self) -> OwnerStatus {
        if let OwnerStatus::Failed(error) = self.virtual_sessions.status() {
            self.cancel();
            return OwnerStatus::Failed(error);
        }
        if self.render.is_finished()
            || self.captures.iter().any(CaptureSession::is_finished)
            || self.followers.iter().any(RenderSession::is_finished)
        {
            self.observations.clear(self.generation);
            let _ = self.stop.request_stop();
            if !self.reported_failure.swap(true, Ordering::AcqRel) {
                self.resources.retry();
            }
            OwnerStatus::Failed(BackendError::ResourceUnavailable(
                "native audio owner stopped; cleanup retains its report".into(),
            ))
        } else if self.render.has_started()
            && self.captures.iter().all(CaptureSession::has_started)
            && self.followers.iter().all(RenderSession::has_started)
            && self.virtual_sessions.status() == OwnerStatus::Running
        {
            self.observations.running(self.generation);
            OwnerStatus::Running
        } else {
            OwnerStatus::Starting
        }
    }

    fn timeline(&self) -> Option<OwnerTimeline> {
        self.timeline.try_snapshot().map(|timeline| OwnerTimeline {
            epoch: timeline.epoch,
            revision: timeline.revision,
            frame: timeline.frame,
        })
    }

    fn stop_and_join(self: Box<Self>) -> OwnerReturn<f32> {
        let Self {
            render,
            captures,
            followers,
            observations,
            generation,
            stop,
            diagnostics,
            mut virtual_sessions,
            ..
        } = *self;
        observations.clear(generation);
        let mut failure = stop
            .request_stop()
            .err()
            .map(|error| BackendError::Failed(error.to_string()));
        if let Err(error) = virtual_sessions.stop_and_join() {
            failure.get_or_insert(error);
        }
        // Both sides share stop. Always join every capture even if render or a
        // peer fails, and keep the returned Engine for host control retirement.
        let rendered = render.join_with_renderer();
        let mut follower_reports = Vec::with_capacity(followers.len());
        for follower in followers {
            match follower.join() {
                Ok(report) => {
                    if report.status == RenderStatus::Failed {
                        failure.get_or_insert_with(|| {
                            BackendError::ResourceUnavailable(
                                report
                                    .failure
                                    .clone()
                                    .unwrap_or_else(|| "follower render stopped".into()),
                            )
                        });
                    }
                    follower_reports.push(report);
                }
                Err(error) => {
                    failure.get_or_insert_with(|| render_error(error));
                }
            }
        }
        let mut reports = Vec::with_capacity(captures.len());
        for capture in captures {
            match capture.join() {
                Ok(report) => {
                    if matches!(
                        report.status,
                        CaptureStatus::Failed | CaptureStatus::TargetExited
                    ) {
                        failure.get_or_insert_with(|| {
                            BackendError::ResourceUnavailable(
                                report
                                    .failure
                                    .clone()
                                    .unwrap_or_else(|| "capture source stopped".into()),
                            )
                        });
                    }
                    reports.push(report);
                }
                Err(error) => {
                    failure.get_or_insert_with(|| capture_error(error));
                }
            }
        }
        let rendered = rendered.expect("native callback unwind retains renderer for its joiner");
        if rendered.report.status == RenderStatus::Failed {
            failure.get_or_insert_with(|| {
                let message = rendered
                    .report
                    .failure
                    .clone()
                    .unwrap_or_else(|| "render failed".into());
                if message == crate::render::RenderError::WorkerPanicked.to_string() {
                    BackendError::WorkerPanicked
                } else {
                    BackendError::ResourceUnavailable(message)
                }
            });
        }
        save(
            &diagnostics,
            NativeReport {
                render: Some(rendered.report),
                prepared: Vec::new(),
                followers: follower_reports,
                capture: reports,
                failure: failure.as_ref().map(ToString::to_string),
            },
        );
        let (engine, _) = rendered.renderer.into_parts();
        OwnerReturn {
            engine,
            error: failure,
        }
    }
}

#[derive(Serialize)]
pub(super) struct NativeReport {
    pub(super) render: Option<RenderReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) prepared: Vec<RenderReport>,
    pub(super) followers: Vec<RenderReport>,
    pub(super) capture: Vec<CaptureReport>,
    pub(super) failure: Option<String>,
}
pub(super) fn save(diagnostics: &Diagnostics, report: NativeReport) {
    *diagnostics
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = serde_json::to_string(&report).ok();
}

#[cfg(test)]
mod tests;
