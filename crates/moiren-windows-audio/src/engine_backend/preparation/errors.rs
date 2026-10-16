use crate::{capture::CaptureError, render::RenderError};
use moiren_engine::host::BackendError;
pub(in crate::engine_backend) fn render_error(error: RenderError) -> BackendError {
    match error {
        RenderError::Cancelled => BackendError::Cancelled,
        RenderError::Api { .. } => BackendError::ResourceUnavailable(error.to_string()),
        RenderError::WorkerPanicked => BackendError::WorkerPanicked,
        _ => BackendError::Unsupported(error.to_string()),
    }
}
pub(in crate::engine_backend) fn capture_error(error: CaptureError) -> BackendError {
    match error {
        CaptureError::Cancelled => BackendError::Cancelled,
        CaptureError::Api { .. }
        | CaptureError::TargetExited
        | CaptureError::ProcessIdentityChanged
        | CaptureError::ActivationTimeout => BackendError::ResourceUnavailable(error.to_string()),
        CaptureError::WorkerPanicked => BackendError::WorkerPanicked,
        _ => BackendError::Unsupported(error.to_string()),
    }
}
