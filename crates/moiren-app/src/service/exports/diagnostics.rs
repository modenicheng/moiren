//! Detached Windows provider evidence, read only on the application control side.
use moiren_windows_audio::vad::VadPcmTelemetrySnapshot;

/// Diagnostic evidence for one accepted lifecycle observation. These values
/// never authorize gate work, transport admission, or an active Engine revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDiagnostics {
    /// The accepted control-adapter lifecycle sequence, assigned by Runtime.
    pub generation: u64,
    /// Native [pdo, session, endpoint, stream, gate], including a closed stream.
    pub native_stamp: Option<[u64; 5]>,
    pub transport_prepared: bool,
    pub telemetry: Option<VadPcmTelemetrySnapshot>,
    pub error: Option<String>,
}
