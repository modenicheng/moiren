//! All preparation exits cancel and join every owner on the control worker.
use super::super::NativeLatencyProfile;
use super::Diagnostics;
use crate::{
    StopSignal,
    capture::{
        CaptureReport, CaptureSession, NativeCaptureInfo, PreparedCapture, PreparedNativeCapture,
    },
    latency::{
        LatencyBudgetPolicy, NativeStreamTiming, PreparedInputQueue, PreparedRenderQueue,
        RenderQueuePolicy,
    },
    render::{NativeRenderInfo, PreparedNativeRender},
};
use moiren_core::graph::NodeId;
use moiren_engine::host::BackendError;
pub(super) struct PreparationGroup {
    pub(super) stop: StopSignal,
    pub(super) captures: Vec<CaptureSession>,
    pub(super) followers: Vec<crate::render::RenderSession>,
    pub(super) prepared: Vec<(
        moiren_core::graph::NodeId,
        crate::render::PreparedNativeRender,
        RenderQueuePolicy,
    )>,
    pub(super) failed_renders: Vec<crate::render::RenderReport>,
    pub(super) diagnostics: Diagnostics,
    pending_captures: Vec<PreparedNativeCapture>,
    failed_captures: Vec<CaptureReport>,
    failure: Option<String>,
}

pub(super) fn check_hardware_rate(
    requested: Option<u32>,
    period: crate::shared_stream::SharedPeriodReport,
    client_rate: u32,
) -> Result<(), BackendError> {
    // Hardware intent constrains the endpoint mix, never Windows' converted
    // client packets. Shared mode must not reconfigure the system endpoint.
    let mix_rate = period
        .mix_format
        .map_or(client_rate, |format| format.sample_rate);
    if requested.is_some_and(|rate| rate != mix_rate) {
        return Err(BackendError::Unsupported(
            "endpoint mix rate differs from requested hardware rate".into(),
        ));
    }
    Ok(())
}
impl PreparationGroup {
    pub(super) fn new(stop: StopSignal, diagnostics: Diagnostics) -> Self {
        Self {
            stop,
            diagnostics,
            captures: Vec::new(),
            followers: Vec::new(),
            prepared: Vec::new(),
            failed_renders: Vec::new(),
            pending_captures: Vec::new(),
            failed_captures: Vec::new(),
            failure: None,
        }
    }
    pub(super) fn check<T>(&mut self, result: Result<T, BackendError>) -> Result<T, BackendError> {
        if let Err(error) = &result {
            self.failure = Some(error.to_string());
        }
        result
    }
    /// Registration precedes every admission check: even the very first
    /// negotiated output belongs to report-aware cleanup when its queue fails.
    pub(super) fn admit_output(
        &mut self,
        node: NodeId,
        native: PreparedNativeRender,
        profile: NativeLatencyProfile,
        configured: RenderQueuePolicy,
    ) -> Result<(NativeRenderInfo, PreparedRenderQueue), BackendError> {
        self.prepared.push((node, native, configured));
        let info = self
            .prepared
            .last()
            .expect("registered native output")
            .1
            .info();
        let policy = self.check(
            profile
                .render_policy(info.timing, configured)
                .map_err(|error| BackendError::Unsupported(error.to_string())),
        )?;
        let queue = self.check(
            crate::latency::prepare_render_queue(info.timing, policy)
                .map_err(|error| BackendError::Unsupported(error.to_string())),
        )?;
        self.prepared
            .last_mut()
            .expect("registered native output")
            .2 = policy;
        Ok((info, queue))
    }
    /// A negotiated capture remains in the group through rate and queue
    /// admission. Attachment itself returns its report on every error.
    pub(super) fn admit_capture(
        &mut self,
        native: PreparedNativeCapture,
        requested_rate: Option<u32>,
        engine_rate: u32,
        profile: NativeLatencyProfile,
        output: NativeStreamTiming,
        policy: LatencyBudgetPolicy,
    ) -> Result<(PreparedCapture, NativeCaptureInfo, PreparedInputQueue), BackendError> {
        self.pending_captures.push(native);
        let info = self
            .pending_captures
            .last()
            .expect("registered native capture")
            .info();
        self.check(check_hardware_rate(
            requested_rate,
            info.shared_period,
            info.timing.sample_rate,
        ))?;
        let input = self
            .check(
                profile
                    .input_budget(info.timing, output, policy)
                    .map_err(|error| BackendError::Unsupported(error.to_string())),
            )?
            .input;
        let native = self
            .pending_captures
            .pop()
            .expect("registered native capture");
        match native.attach_returning(input, engine_rate) {
            Ok(capture) => Ok((capture, info, input)),
            Err(failure) => {
                if let Some(report) = failure.report {
                    self.failed_captures.push(report);
                }
                self.check(Err(super::capture_error(failure.error)))
            }
        }
    }
}
impl Drop for PreparationGroup {
    fn drop(&mut self) {
        if !self.captures.is_empty()
            || !self.followers.is_empty()
            || !self.prepared.is_empty()
            || !self.pending_captures.is_empty()
            || !self.failed_captures.is_empty()
            || !self.failed_renders.is_empty()
        {
            let _ = self.stop.request_stop();
            let mut followers = std::mem::take(&mut self.failed_renders);
            let mut prepared = Vec::with_capacity(self.prepared.len());
            for (_, native, _) in std::mem::take(&mut self.prepared) {
                if let Ok(report) = native.cancel_with_report() {
                    prepared.push(report);
                }
            }
            for follower in std::mem::take(&mut self.followers) {
                if let Ok(report) = follower.join() {
                    followers.push(report);
                }
            }
            let mut capture = std::mem::take(&mut self.failed_captures);
            for native in std::mem::take(&mut self.pending_captures) {
                if let Ok(report) = native.cancel_with_report() {
                    capture.push(report);
                }
            }
            for session in std::mem::take(&mut self.captures) {
                if let Ok(report) = session.join() {
                    capture.push(report);
                }
            }
            super::super::owner::save(
                &self.diagnostics,
                super::super::owner::NativeReport {
                    render: None,
                    prepared,
                    followers,
                    capture,
                    failure: Some(
                        self.failure
                            .take()
                            .unwrap_or_else(|| "native preparation did not complete".into()),
                    ),
                },
            );
        }
    }
}
#[cfg(test)]
mod tests;
