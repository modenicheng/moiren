use super::{
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
    pub(super) output: Option<AudioReader<f32>>,
    pub(super) sink: NodeId,
    pub(super) stop: StopSignal,
    pub(super) gate: ActivationGate,
    pub(super) generation: u64,
    pub(super) queue: RenderQueuePolicy,
    pub(super) diagnostics: Diagnostics,
    pub(super) resources: Arc<Resources>,
}

impl Drop for WindowsTransport {
    fn drop(&mut self) {
        if self.native.is_some() || !self.captures.is_empty() {
            let _ = self.stop.request_stop();
        }
    }
}

impl WindowsTransport {
    fn cancel_and_report(&mut self, report: Option<RenderReport>, failure: &BackendError) {
        let _ = self.stop.request_stop();
        let render = report.or_else(|| {
            self.native
                .take()
                .and_then(|native| native.cancel_with_report().ok())
        });
        let capture = std::mem::take(&mut self.captures)
            .into_iter()
            .filter_map(|capture| capture.join().ok())
            .collect();
        save(
            &self.diagnostics,
            NativeReport {
                render,
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
        let Some(sink) = bindings
            .node(self.sink)
            .filter(|sink| engine.plan_snapshot().single_sink() == Some(*sink))
        else {
            let error = BackendError::Unsupported(
                "transport does not match the compiled output sink".into(),
            );
            self.cancel_and_report(None, &error);
            return Err(Box::new(AttachFailure { engine, error }));
        };
        let renderer = match DemandRenderer::new_managed_returning(
            engine,
            self.output.take().expect("prepared output consumer"),
            sink,
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
        Ok(Box::new(WindowsOwner {
            render,
            captures: std::mem::take(&mut self.captures),
            stop: self.stop.clone(),
            timeline,
            diagnostics: self.diagnostics.clone(),
            resources: self.resources.clone(),
            reported_failure: AtomicBool::new(false),
            gate: self.gate.clone(),
            cancellation: cancel,
        }))
    }
}

struct WindowsOwner {
    render: RenderSession,
    captures: Vec<CaptureSession>,
    stop: StopSignal,
    timeline: TimelineObserver,
    diagnostics: Diagnostics,
    resources: Arc<Resources>,
    reported_failure: AtomicBool,
    gate: ActivationGate,
    cancellation: CancellationToken,
}

impl BackendOwner<f32> for WindowsOwner {
    fn activate(&self) -> Result<(), BackendError> {
        if self.cancellation.is_cancelled() {
            return Err(BackendError::Cancelled);
        }
        self.gate
            .activate()
            .map_err(|error| BackendError::Failed(error.to_string()))
    }

    fn cancel(&self) {
        let _ = self.stop.request_stop();
    }

    fn status(&self) -> OwnerStatus {
        if self.render.is_finished() || self.captures.iter().any(CaptureSession::is_finished) {
            if !self.reported_failure.swap(true, Ordering::AcqRel) {
                self.resources.retry();
            }
            OwnerStatus::Failed(BackendError::ResourceUnavailable(
                "native audio owner stopped; cleanup retains its report".into(),
            ))
        } else if self.render.has_started() && self.captures.iter().all(CaptureSession::has_started)
        {
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
            stop,
            diagnostics,
            ..
        } = *self;
        let mut failure = stop
            .request_stop()
            .err()
            .map(|error| BackendError::Failed(error.to_string()));
        // Both sides share stop. Always join every capture even if render or a
        // peer fails, and keep the returned Engine for host control retirement.
        let rendered = render.join_with_renderer();
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
struct NativeReport {
    render: Option<RenderReport>,
    capture: Vec<CaptureReport>,
    failure: Option<String>,
}
fn save(diagnostics: &Diagnostics, report: NativeReport) {
    *diagnostics
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = serde_json::to_string(&report).ok();
}

#[cfg(test)]
mod tests;
