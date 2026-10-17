//! Native preparation, streaming and destruction run on one COM owner thread.
use super::{
    Startup, Wake, api, format::mix_format, handle, preparation::CapturePreparation, stream,
    wait_target,
};
use crate::ActivationGate;
use crate::{
    capture::{CaptureError, CaptureOptions, CaptureReport, CaptureSource, CaptureStatus},
    owner::{Apartment, OwnedHandle},
};
use std::{
    os::windows::io::OwnedHandle as StdHandle,
    sync::{Arc, atomic::AtomicBool, mpsc::SyncSender},
};
use windows::{
    Win32::{
        Media::Audio::{
            AudioCategory_Other, AudioClientProperties, IAudioClient, IAudioClient2,
            IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, eCapture,
        },
        System::Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
    },
    core::{HSTRING, Interface},
};

fn owner(
    options: &CaptureOptions,
    stop: &StdHandle,
    preparation: &CapturePreparation,
    report: &mut CaptureReport,
    gate: &ActivationGate,
    started: &AtomicBool,
) -> Result<CaptureStatus, CaptureError> {
    if wait_target(handle(stop), handle(stop), None, 0)? == Wake::Stop {
        return Err(CaptureError::Cancelled);
    }
    // Reverse declaration order releases stream/services/client before apartment.
    let _apartment = api("CoInitializeEx(capture)", Apartment::new())?;
    let enumerator: IMMDeviceEnumerator = api("CoCreateInstance(capture)", unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    })?;
    let device = api("GetDevice(capture pinned)", unsafe {
        enumerator.GetDevice(&HSTRING::from(&options.endpoint_id))
    })?;
    let endpoint: IMMEndpoint = api("IMMEndpoint(capture)", device.cast())?;
    if api("GetDataFlow(capture)", unsafe { endpoint.GetDataFlow() })? != eCapture {
        return Err(CaptureError::InvalidEndpoint);
    }
    let event = api("CreateEvent(capture audio)", OwnedHandle::event())?;
    let client: IAudioClient = api("Activate(capture)", unsafe {
        device.Activate(CLSCTX_ALL, None)
    })?;
    let client2: IAudioClient2 = api("IAudioClient2(capture)", client.cast())?;
    api("SetClientProperties(capture)", unsafe {
        client2.SetClientProperties(&AudioClientProperties {
            cbSize: size_of::<AudioClientProperties>() as u32,
            eCategory: AudioCategory_Other,
            ..Default::default()
        })
    })?;
    let memory = mix_format(&client)?;
    // Packet lengths and bridge timing use the selected client descriptor,
    // which can differ from the endpoint mix rate and microphone-array layout.
    let base = memory.client();
    report.sample_rate = Some(base.sample_rate);
    report.channels = Some(usize::from(base.channels));
    let guid = api("CoCreateGuid(capture)", unsafe { CoCreateGuid() })?;
    crate::shared_stream::initialize_shared(
        &client,
        &memory,
        options.shared_period,
        &guid,
        &mut report.shared_period,
    )
    .map_err(|error| CaptureError::Api {
        stage: error.stage,
        hresult: error.error.code().0,
    })?;
    report.windows_auto_conversion = memory.windows_auto_conversion;
    api("SetEventHandle(capture)", unsafe {
        client.SetEventHandle(event.0)
    })?;
    stream::run_prepared(
        stream::StreamInput {
            client: &client,
            gate,
            started,
            audio: event.0,
            target: None,
            stop,
            duration: options.duration,
            latency_telemetry: options.latency_telemetry,
            sample_rate: base.sample_rate,
            channels: usize::from(base.channels),
        },
        preparation,
        report,
    )
}
pub(super) fn run_owner(
    options: CaptureOptions,
    stop: Arc<StdHandle>,
    sender: SyncSender<Result<Startup, CaptureError>>,
    gate: ActivationGate,
    started: Arc<AtomicBool>,
) -> CaptureReport {
    run_owner_prepared(
        options,
        stop,
        CapturePreparation::Immediate(sender),
        gate,
        started,
    )
}
pub(super) fn run_owner_prepared(
    options: CaptureOptions,
    stop: Arc<StdHandle>,
    preparation: CapturePreparation,
    gate: ActivationGate,
    started: Arc<AtomicBool>,
) -> CaptureReport {
    let mut report = CaptureReport::new(CaptureSource::Physical, options.duration);
    report.latency = crate::latency::LatencyRecorder::new(options.latency_telemetry).snapshot();
    report.endpoint_id = Some(options.endpoint_id.clone());
    let result = owner(&options, &stop, &preparation, &mut report, &gate, &started);
    preparation.finish(result, &stop, report)
}
