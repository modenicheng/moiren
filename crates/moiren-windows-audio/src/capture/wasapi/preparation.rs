//! Native capture waits on its owner while the control side builds the bridge.
use super::{CaptureSession, PreparedCapture, Startup, api, handle, physical};
use crate::{
    ActivationGate, StopSignal,
    capture::{CaptureError, CaptureOptions, CaptureReport, CaptureStatus},
    clock_bridge::{CaptureIngress, ClockBridgeConfig, capture_bridge_with_clock},
    latency::clock::QpcClock,
    latency::{NativeStreamTiming, PreparedInputQueue},
    preparation,
};
use std::{
    os::windows::io::OwnedHandle,
    sync::{Arc, atomic::AtomicBool, mpsc},
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
    bridge: preparation::Sender<(CaptureIngress, PreparedInputQueue)>,
    attached: mpsc::Receiver<Result<(), CaptureError>>,
}
pub(crate) struct CaptureAttachFailure {
    pub error: CaptureError,
    pub report: Option<CaptureReport>,
}
impl PreparedNativeCapture {
    pub fn info(&self) -> NativeCaptureInfo {
        self.info
    }
    pub fn attach(self, input: PreparedInputQueue) -> Result<PreparedCapture, CaptureError> {
        self.attach_returning(input, crate::NATIVE_SAMPLE_RATE)
            .map_err(|failure| failure.error)
    }
    pub(crate) fn cancel_with_report(self) -> Result<CaptureReport, CaptureError> {
        let _ = self.session.request_stop();
        self.session.join()
    }
    /// Control-side failures retain the negotiated stream report, including a
    /// rejected queue before handoff. The peer stop precedes every blocking join.
    pub(crate) fn attach_returning(
        self,
        input: PreparedInputQueue,
        engine_rate: u32,
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
            api("Send capture bridge", self.bridge.send((ingress, input)))?;
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
                let _ = self.session.request_stop();
                let report = self.session.join().ok();
                Err(Box::new(CaptureAttachFailure { error, report }))
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
        bridge: preparation::Receiver<(CaptureIngress, PreparedInputQueue)>,
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
                let (ingress, input) = api("Wait for capture bridge", bridge.receive(stop))?
                    .ok_or(CaptureError::Cancelled)?;
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
    options.validate()?;
    gate.validate_stop(&stop)?;
    start_native_owner(stop, move |stop, preparation, started| {
        physical::run_owner_prepared(options, stop, preparation, gate, started)
    })
}
fn start_native_owner(
    stop: StopSignal,
    owner: impl FnOnce(Arc<OwnedHandle>, CapturePreparation, Arc<AtomicBool>) -> CaptureReport
    + Send
    + 'static,
) -> Result<PreparedNativeCapture, CaptureError> {
    let (sender, receiver) = api("Create capture handoff", preparation::channel())?;
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
        .map_err(|error| CaptureError::WorkerSpawn {
            code: error.raw_os_error(),
        })?;
    let session = CaptureSession {
        started,
        stop: stop.event,
        worker: Some(worker),
    };
    let info = match metadata.recv() {
        Ok(result) => result?,
        Err(_) => {
            let _ = session.request_stop();
            session.join()?;
            return Err(CaptureError::StartupLost);
        }
    };
    Ok(PreparedNativeCapture {
        info,
        session,
        bridge: sender,
        attached: attachment,
    })
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
