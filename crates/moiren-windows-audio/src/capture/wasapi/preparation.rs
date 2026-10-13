//! Native capture waits on its owner while the control side builds the bridge.
use super::{CaptureSession, PreparedCapture, Startup, api, handle, physical};
use crate::{
    ActivationGate, StopSignal,
    capture::{CaptureError, CaptureOptions, CaptureReport, CaptureStatus},
    clock_bridge::{CaptureIngress, ClockBridgeConfig, capture_bridge},
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
}
pub struct PreparedNativeCapture {
    pub info: NativeCaptureInfo,
    session: CaptureSession,
    bridge: preparation::Sender<(CaptureIngress, PreparedInputQueue)>,
    attached: mpsc::Receiver<Result<(), CaptureError>>,
}
impl PreparedNativeCapture {
    pub fn attach(self, input: PreparedInputQueue) -> Result<PreparedCapture, CaptureError> {
        let (ingress, source, observer) = capture_bridge(ClockBridgeConfig::with_prepared_input(
            self.info.timing.sample_rate,
            self.info.channels,
            input,
        ))?;
        api("Send capture bridge", self.bridge.send((ingress, input)))?;
        self.attached
            .recv()
            .map_err(|_| CaptureError::StartupLost)??;
        Ok(PreparedCapture {
            session: self.session,
            source,
            observer,
            sample_rate: self.info.timing.sample_rate,
            channels: self.info.channels,
        })
    }
}
pub(super) enum CapturePreparation {
    Immediate(mpsc::SyncSender<Result<Startup, CaptureError>>),
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
    ) -> Result<CaptureIngress, CaptureError> {
        match self {
            Self::Immediate(sender) => {
                let (ingress, source, observer) = capture_bridge(ClockBridgeConfig {
                    input_sample_rate: timing.sample_rate,
                    input_channels: channels,
                    detect_position_gaps: report.source == crate::capture::CaptureSource::Physical,
                    ..ClockBridgeConfig::default()
                })?;
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
                    Self::Immediate(sender) => {
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
    let (sender, receiver) = api("Create capture handoff", preparation::channel())?;
    let (ready, metadata) = mpsc::sync_channel(1);
    let (attached, attachment) = mpsc::sync_channel(1);
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = started.clone();
    let worker_stop = stop.event.clone();
    let worker = thread::Builder::new()
        .name("moiren-shared-capture".into())
        .spawn(move || {
            physical::run_owner_prepared(
                options,
                worker_stop,
                CapturePreparation::Deferred {
                    ready,
                    bridge: receiver,
                    attached,
                },
                gate,
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
