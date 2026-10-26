//! Owner-local virtual-client activation and Shared format preparation.
use super::{ProcessLoopbackOptions, activation, identity};
use crate::{
    ActivationGate, StopSignal,
    capture::{
        CaptureError, CaptureReport, CaptureSource, CaptureStatus, PreparedCapture,
        wasapi::{Startup, api, handle, start_owner, stream},
    },
    owner::{Apartment, OwnedHandle},
};
use std::{
    os::windows::io::OwnedHandle as StdHandle,
    sync::{atomic::AtomicBool, mpsc::SyncSender},
    time::Instant,
};
use windows::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    Media::{
        Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
            AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
        },
        Multimedia::WAVE_FORMAT_IEEE_FLOAT,
    },
    System::Threading::WaitForSingleObject,
};

pub fn start_process_capture(
    options: ProcessLoopbackOptions,
) -> Result<PreparedCapture, CaptureError> {
    options.validate()?;
    let stop = api("CreateEvent(process stop)", StopSignal::new())?;
    start_process_capture_with_stop(options, stop)
}
pub fn start_process_capture_with_stop(
    options: ProcessLoopbackOptions,
    stop: StopSignal,
) -> Result<PreparedCapture, CaptureError> {
    options.validate()?;
    let gate = ActivationGate::new(stop.clone())?;
    let prepared = prepare_process_capture_with_gate(options, stop, gate.clone())?;
    gate.activate()?;
    Ok(prepared)
}
pub fn prepare_process_capture_with_gate(
    options: ProcessLoopbackOptions,
    stop: StopSignal,
    gate: ActivationGate,
) -> Result<PreparedCapture, CaptureError> {
    options.validate()?;
    gate.validate_stop(&stop)?;
    start_owner(stop, move |stop, sender, started| {
        let mut report = CaptureReport::new(CaptureSource::ProcessLoopback, options.duration);
        report.latency = crate::latency::LatencyRecorder::new(options.latency_telemetry).snapshot();
        report.process = Some(options.target.clone());
        report.delivery_bound = options.bridge.delivery;
        let result = owner(&options, &stop, &sender, &mut report, &gate, &started);
        stream::finish(result, &stop, &sender, report)
    })
}
fn cancelled(stop: &StdHandle) -> Result<(), CaptureError> {
    match unsafe { WaitForSingleObject(handle(stop), 0) } {
        WAIT_OBJECT_0 => Err(CaptureError::Cancelled),
        WAIT_TIMEOUT => Ok(()),
        _ => api(
            "WaitForSingleObject(process stop)",
            Err(windows::core::Error::from_thread()),
        ),
    }
}

/// Shared Process Loopback flags: event delivery plus Windows conversion into
/// the fixed bridge format. Every owner waits on the event after initialization.
pub(crate) fn process_loopback_stream_flags() -> u32 {
    AUDCLNT_STREAMFLAGS_LOOPBACK
        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
}

fn owner(
    options: &ProcessLoopbackOptions,
    stop: &StdHandle,
    sender: &SyncSender<Result<Startup, CaptureError>>,
    report: &mut CaptureReport,
    gate: &ActivationGate,
    started: &AtomicBool,
) -> Result<CaptureStatus, CaptureError> {
    cancelled(stop)?;
    let _apartment = api("CoInitializeEx(process capture)", Apartment::new())?;
    let process = identity::open_selected(&options.target)?;
    report.process = Some(process.identity.clone());
    let check = || {
        cancelled(stop)?;
        if api("Process liveness(activation)", process.exited())? {
            return Err(CaptureError::TargetExited);
        }
        Ok(())
    };
    check()?;
    let activation_started = Instant::now();
    let activation = activation::activate(options.target.pid, check);
    report.activation_seconds = Some(activation_started.elapsed().as_secs_f64());
    let client = activation?;
    check()?;
    let event = api("CreateEvent(process audio)", OwnedHandle::event())?;
    let format = capture_format();
    // Virtual process loopback is not an endpoint-native capture. Request a
    // fixed bridge format and allow Windows to convert the target's streams.
    api("Initialize(process Shared)", unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            process_loopback_stream_flags(),
            0,
            0,
            &format,
            None,
        )
    })?;
    api("SetEventHandle(process capture)", unsafe {
        client.SetEventHandle(event.0)
    })?;
    stream::run(
        stream::StreamInput {
            client: &client,
            gate,
            started,
            audio: event.0,
            target: Some(process.handle.0),
            stop,
            duration: options.duration,
            latency_telemetry: options.latency_telemetry,
            sample_rate: crate::NATIVE_SAMPLE_RATE,
            channels: 2,
            artifact: options.artifact,
        },
        sender,
        report,
        options.bridge.clock_config()?,
    )
}
pub(crate) fn capture_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: 2,
        nSamplesPerSec: crate::NATIVE_SAMPLE_RATE,
        nAvgBytesPerSec: 384000,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_loopback_stream_flags_request_conversion_and_event_delivery() {
        let flags = process_loopback_stream_flags();
        assert_eq!(
            flags & AUDCLNT_STREAMFLAGS_LOOPBACK,
            AUDCLNT_STREAMFLAGS_LOOPBACK
        );
        assert_eq!(
            flags & AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        );
        assert_eq!(
            flags & AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        );
        assert_eq!(
            flags & AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
        );
    }

    #[test]
    fn pre_cancelled_preparation_never_activates_or_starts_client() {
        let stop = StopSignal::new().unwrap();
        stop.request_stop().unwrap();
        let options = ProcessLoopbackOptions {
            artifact: None,
            bridge: Default::default(),
            latency_telemetry: crate::latency::TelemetryMode::Summary,
            target: super::super::ProcessIdentity {
                pid: u32::MAX,
                creation_time_100ns: 1,
                executable_name: "missing.exe".into(),
            },
            duration: crate::SessionDuration::For(std::time::Duration::from_secs(10)),
        };
        assert!(matches!(
            start_process_capture_with_stop(options, stop),
            Err(CaptureError::Cancelled)
        ));
    }
}
