//! Native capture waits on its owner while the control side builds the bridge.
use super::{CaptureSession, PreparedCapture, Startup, api, handle, physical};
use crate::{
    ActivationGate, StopSignal,
    capture::{
        CaptureError, CaptureOptions, CaptureReport, CaptureStatus, PcmEvidenceConfig,
        PcmEvidenceReader, PcmEvidenceWriter, pcm_evidence_stream,
    },
    clock_bridge::{CaptureIngress, ClockBridgeConfig, capture_bridge_with_clock},
    latency::clock::QpcClock,
    latency::{NativeStreamTiming, PreparedInputQueue},
    preparation,
};
use std::{
    fmt,
    os::windows::io::OwnedHandle,
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    thread,
};
use windows::Win32::System::Threading::SetEvent;

#[derive(Debug, Clone, Copy)]
pub struct NativeCaptureInfo {
    pub timing: NativeStreamTiming,
    pub channels: usize,
    pub shared_period: crate::shared_stream::SharedPeriodReport,
    pub(crate) age_clock: Option<QpcClock>,
}
pub struct PreparedNativeCapture {
    info: NativeCaptureInfo,
    session: CaptureSession,
    bridge: preparation::Sender<CaptureAttachment>,
    attached: mpsc::Receiver<Result<(), CaptureError>>,
}
type EvidenceHandoff = Arc<Mutex<Option<PcmEvidenceWriter>>>;
type CaptureAttachment = (CaptureIngress, PreparedInputQueue, Option<EvidenceHandoff>);
pub(crate) struct CaptureAttachFailure {
    pub error: CaptureError,
    pub report: Option<CaptureReport>,
    pub cleanup_error: Option<CaptureError>,
}

/// Native preparation can fail after the worker has already assembled useful
/// negotiation evidence. Keep that report available to callers instead of
/// reducing the failure to only its HRESULT and stage text.
#[derive(Debug)]
pub struct CapturePreparationFailure {
    pub error: CaptureError,
    pub report: Option<CaptureReport>,
    pub cleanup_error: Option<CaptureError>,
}

impl fmt::Display for CapturePreparationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)?;
        if let Some(error) = self.cleanup_error {
            write!(formatter, "; native cleanup failed: {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CapturePreparationFailure {}
impl PreparedNativeCapture {
    pub fn info(&self) -> NativeCaptureInfo {
        self.info
    }
    pub fn attach(self, input: PreparedInputQueue) -> Result<PreparedCapture, CaptureError> {
        self.attach_returning(input, crate::NATIVE_SAMPLE_RATE)
            .map_err(|failure| failure.error)
    }
    /// Prepare bounded evidence storage on control and copy native packets
    /// before lease release. This is an observation tap, not a graph input.
    pub fn attach_with_evidence(
        self,
        input: PreparedInputQueue,
        config: PcmEvidenceConfig,
    ) -> Result<(PreparedCapture, PcmEvidenceReader), CaptureError> {
        self.attach_with_evidence_returning(input, config)
            .map_err(|failure| failure.error)
    }
    /// Like [`Self::attach_with_evidence`], but preserves the native report
    /// when control-side attachment or evidence setup fails.
    pub fn attach_with_evidence_returning(
        self,
        input: PreparedInputQueue,
        mut config: PcmEvidenceConfig,
    ) -> Result<(PreparedCapture, PcmEvidenceReader), Box<CapturePreparationFailure>> {
        config.detect_position_gaps &=
            client_position_clock(self.info.timing, self.info.shared_period);
        let (writer, reader) =
            match pcm_evidence_stream(config, self.info.timing.sample_rate, self.info.channels) {
                Ok(stream) => stream,
                Err(error) => {
                    let (report, cleanup_error) = stop_and_join(self.session);
                    return Err(Box::new(CapturePreparationFailure {
                        error: CaptureError::PcmEvidence(error),
                        report,
                        cleanup_error,
                    }));
                }
            };
        // Both sides retain the prepared writer until attachment is resolved.
        // A failed/closed handoff cannot free its last allocation on the owner.
        let handoff = Arc::new(Mutex::new(Some(writer)));
        let capture = self
            .attach_with_handoff(input, crate::NATIVE_SAMPLE_RATE, Some(handoff.clone()))
            .map_err(|failure| {
                Box::new(CapturePreparationFailure {
                    error: failure.error,
                    report: failure.report,
                    cleanup_error: failure.cleanup_error,
                })
            })?;
        Ok((capture, reader))
    }
    /// Stop the prepared native owner and return its final report without
    /// attaching a graph bridge. This is used when a control-side validation
    /// fails after native negotiation but before attachment.
    pub fn cancel_with_report(self) -> Result<CaptureReport, CaptureError> {
        let (report, cleanup_error) = self.cancel_with_report_returning();
        match (report, cleanup_error) {
            (Some(report), None) => Ok(report),
            (_, Some(error)) => Err(error),
            (None, None) => Err(CaptureError::WorkerPanicked),
        }
    }
    /// Stop the prepared owner while retaining both a partial report and a
    /// control-side stop/join error for callers that need failure evidence.
    pub fn cancel_with_report_returning(self) -> (Option<CaptureReport>, Option<CaptureError>) {
        stop_and_join(self.session)
    }
    /// Control-side failures retain the negotiated stream report, including a
    /// rejected queue before handoff. The peer stop precedes every blocking join.
    pub(crate) fn attach_returning(
        self,
        input: PreparedInputQueue,
        engine_rate: u32,
    ) -> Result<PreparedCapture, Box<CaptureAttachFailure>> {
        self.attach_with_handoff(input, engine_rate, None)
    }
    fn attach_with_handoff(
        self,
        input: PreparedInputQueue,
        engine_rate: u32,
        evidence: Option<EvidenceHandoff>,
    ) -> Result<PreparedCapture, Box<CaptureAttachFailure>> {
        let attachment = (|| {
            let mut config = ClockBridgeConfig::with_engine_input(
                self.info.timing.sample_rate,
                engine_rate,
                self.info.channels,
                input,
            );
            config.detect_position_gaps =
                client_position_clock(self.info.timing, self.info.shared_period);
            let (ingress, source, observer) =
                capture_bridge_with_clock(config, self.info.age_clock)?;
            api(
                "Send capture bridge",
                self.bridge.send((ingress, input, evidence)),
            )?;
            self.attached
                .recv()
                .map_err(|_| CaptureError::StartupLost)??;
            Ok::<_, CaptureError>((source, observer))
        })();
        match attachment {
            Ok((source, observer)) => Ok(PreparedCapture {
                session: self.session,
                source,
                observer,
                sample_rate: self.info.timing.sample_rate,
                channels: self.info.channels,
            }),
            Err(error) => {
                let (report, cleanup_error) = stop_and_join(self.session);
                Err(Box::new(CaptureAttachFailure {
                    error,
                    report,
                    cleanup_error,
                }))
            }
        }
    }
}

fn client_position_clock(
    timing: NativeStreamTiming,
    period: crate::shared_stream::SharedPeriodReport,
) -> bool {
    // GetBuffer positions remain in endpoint/device frames through Windows SRC,
    // while packet lengths are client frames. Do not infer gaps, age or rate
    // from incompatible units; driver flags and the inventory servo still work.
    period
        .mix_format
        .is_none_or(|mix| mix.sample_rate == timing.sample_rate)
}
pub(super) enum CapturePreparation {
    Immediate(mpsc::SyncSender<Result<Startup, CaptureError>>),
    Configured {
        sender: mpsc::SyncSender<Result<Startup, CaptureError>>,
        config: ClockBridgeConfig,
    },
    Deferred {
        ready: mpsc::SyncSender<Result<NativeCaptureInfo, CaptureError>>,
        bridge: preparation::Receiver<CaptureAttachment>,
        attached: mpsc::SyncSender<Result<(), CaptureError>>,
    },
}
impl CapturePreparation {
    pub(super) fn prepare(
        &self,
        timing: NativeStreamTiming,
        channels: usize,
        stop: &OwnedHandle,
        report: &mut CaptureReport,
        age_clock: Option<QpcClock>,
    ) -> Result<CaptureIngress, CaptureError> {
        match self {
            Self::Immediate(sender) | Self::Configured { sender, .. } => {
                let config = if let Self::Configured { config, .. } = self {
                    *config
                } else {
                    ClockBridgeConfig {
                        input_sample_rate: timing.sample_rate,
                        input_channels: channels,
                        detect_position_gaps: report.source
                            == crate::capture::CaptureSource::Physical
                            && client_position_clock(timing, report.shared_period),
                        ..ClockBridgeConfig::default()
                    }
                };
                let (ingress, source, observer) = capture_bridge_with_clock(config, age_clock)?;
                report.input_queue = config.prepared_input;
                sender
                    .send(Ok(Startup {
                        source,
                        observer,
                        sample_rate: timing.sample_rate,
                        channels,
                    }))
                    .map_err(|_| CaptureError::StartupLost)?;
                Ok(ingress)
            }
            Self::Deferred {
                ready,
                bridge,
                attached,
            } => {
                ready
                    .send(Ok(NativeCaptureInfo {
                        timing,
                        channels,
                        shared_period: report.shared_period,
                        age_clock,
                    }))
                    .map_err(|_| CaptureError::StartupLost)?;
                let (ingress, input, evidence) =
                    api("Wait for capture bridge", bridge.receive(stop))?
                        .ok_or(CaptureError::Cancelled)?;
                if let Some(evidence) = evidence {
                    report.evidence_recorder = evidence
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take();
                }
                report.input_queue = Some(input);
                attached
                    .send(Ok(()))
                    .map_err(|_| CaptureError::StartupLost)?;
                Ok(ingress)
            }
        }
    }
    pub(super) fn finish(
        &self,
        result: Result<CaptureStatus, CaptureError>,
        stop: &OwnedHandle,
        mut report: CaptureReport,
    ) -> CaptureReport {
        let _ = unsafe { SetEvent(handle(stop)) };
        if let Self::Deferred { bridge, .. } = self
            && let Some((_, _, Some(evidence))) = bridge.close_and_take()
        {
            report.evidence_recorder = evidence
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take();
        }
        match result {
            Ok(status) => report.status = status,
            Err(error) => {
                report.failure = Some(error.to_string());
                match self {
                    Self::Immediate(sender) | Self::Configured { sender, .. } => {
                        let _ = sender.try_send(Err(error));
                    }
                    Self::Deferred {
                        ready, attached, ..
                    } => {
                        let _ = ready.try_send(Err(error));
                        let _ = attached.try_send(Err(error));
                    }
                }
            }
        }
        report.finish_artifact();
        report
    }
}
pub fn prepare_capture_native_with_gate(
    options: CaptureOptions,
    stop: StopSignal,
    gate: ActivationGate,
) -> Result<PreparedNativeCapture, CaptureError> {
    prepare_capture_native_with_gate_returning(options, stop, gate).map_err(|failure| failure.error)
}
pub fn prepare_capture_native_with_gate_returning(
    options: CaptureOptions,
    stop: StopSignal,
    gate: ActivationGate,
) -> Result<PreparedNativeCapture, Box<CapturePreparationFailure>> {
    options.validate().map_err(|error| {
        Box::new(CapturePreparationFailure {
            error,
            report: None,
            cleanup_error: None,
        })
    })?;
    gate.validate_stop(&stop).map_err(|error| {
        Box::new(CapturePreparationFailure {
            error: CaptureError::Gate(error),
            report: None,
            cleanup_error: None,
        })
    })?;
    start_native_owner_returning(stop, move |stop, preparation, started| {
        physical::run_owner_prepared(options, stop, preparation, gate, started)
    })
}
#[cfg(test)]
fn start_native_owner(
    stop: StopSignal,
    owner: impl FnOnce(Arc<OwnedHandle>, CapturePreparation, Arc<AtomicBool>) -> CaptureReport
    + Send
    + 'static,
) -> Result<PreparedNativeCapture, CaptureError> {
    start_native_owner_returning(stop, owner).map_err(|failure| failure.error)
}
fn start_native_owner_returning(
    stop: StopSignal,
    owner: impl FnOnce(Arc<OwnedHandle>, CapturePreparation, Arc<AtomicBool>) -> CaptureReport
    + Send
    + 'static,
) -> Result<PreparedNativeCapture, Box<CapturePreparationFailure>> {
    let (sender, receiver) =
        api("Create capture handoff", preparation::channel()).map_err(|error| {
            Box::new(CapturePreparationFailure {
                error,
                report: None,
                cleanup_error: None,
            })
        })?;
    let (ready, metadata) = mpsc::sync_channel(1);
    let (attached, attachment) = mpsc::sync_channel(1);
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_stop = stop.event.clone();
    let worker = thread::Builder::new()
        .name("moiren-shared-capture".into())
        .spawn(move || {
            let _stop_on_exit = preparation::OwnerStop(worker_stop.clone());
            owner(
                worker_stop,
                CapturePreparation::Deferred {
                    ready,
                    bridge: receiver,
                    attached,
                },
                worker_started,
            )
        })
        .map_err(|error| {
            Box::new(CapturePreparationFailure {
                error: CaptureError::WorkerSpawn {
                    code: error.raw_os_error(),
                },
                report: None,
                cleanup_error: None,
            })
        })?;
    let session = CaptureSession {
        started,
        stop: stop.event,
        worker: Some(worker),
    };
    let info = match metadata.recv() {
        Ok(Ok(info)) => info,
        Ok(Err(error)) => {
            let (report, cleanup_error) = stop_and_join(session);
            return Err(Box::new(CapturePreparationFailure {
                error,
                report,
                cleanup_error,
            }));
        }
        Err(_) => {
            let (report, cleanup_error) = stop_and_join(session);
            let error = cleanup_error.unwrap_or(CaptureError::StartupLost);
            return Err(Box::new(CapturePreparationFailure {
                error,
                report,
                cleanup_error: None,
            }));
        }
    };
    Ok(PreparedNativeCapture {
        info,
        session,
        bridge: sender,
        attached: attachment,
    })
}

fn stop_and_join(session: CaptureSession) -> (Option<CaptureReport>, Option<CaptureError>) {
    let stop_error = session.request_stop().err();
    match session.join() {
        Ok(report) => (Some(report), stop_error),
        Err(error) => (None, Some(error)),
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) fn prepare_test_native(
    stop: StopSignal,
    gate: ActivationGate,
    exited: Arc<AtomicBool>,
) -> Result<PreparedNativeCapture, CaptureError> {
    tests::fake_prepared(stop, gate, tests::Outcome::Run, exited)
}
