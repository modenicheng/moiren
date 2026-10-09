//! Native preparation, streaming and destruction run on one COM owner thread.
use super::{Startup, api, format::mix_format, stream};
use crate::{
    capture::{CaptureError, CaptureOptions, CaptureReport, CaptureSource, CaptureStatus},
    owner::{Apartment, OwnedHandle},
};
use std::{
    os::windows::io::OwnedHandle as StdHandle,
    sync::{Arc, mpsc::SyncSender},
};
use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_NOPERSIST, AudioCategory_Other, AudioClientProperties,
            IAudioClient, IAudioClient2, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator,
            eCapture,
        },
        System::Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance},
    },
    core::{HSTRING, Interface},
};

fn owner(
    options: &CaptureOptions,
    stop: &StdHandle,
    sender: &SyncSender<Result<Startup, CaptureError>>,
    report: &mut CaptureReport,
) -> Result<CaptureStatus, CaptureError> {
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
    // SAFETY: mix_format validated the owned packed native structure.
    let base = unsafe { memory.0.read_unaligned() };
    report.sample_rate = Some(base.nSamplesPerSec);
    report.channels = Some(usize::from(base.nChannels));
    let guid = api("CoCreateGuid(capture)", unsafe { CoCreateGuid() })?;
    api("Initialize(capture Shared)", unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_NOPERSIST,
            0,
            0,
            memory.0,
            Some(&guid),
        )
    })?;
    api("SetEventHandle(capture)", unsafe {
        client.SetEventHandle(event.0)
    })?;
    stream::run(
        stream::StreamInput {
            client: &client,
            audio: event.0,
            target: None,
            stop,
            duration: options.duration,
            sample_rate: base.nSamplesPerSec,
            channels: usize::from(base.nChannels),
        },
        sender,
        report,
    )
}
pub(super) fn run_owner(
    options: CaptureOptions,
    stop: Arc<StdHandle>,
    sender: SyncSender<Result<Startup, CaptureError>>,
) -> CaptureReport {
    let mut report = CaptureReport::new(CaptureSource::Physical, options.duration);
    report.endpoint_id = Some(options.endpoint_id.clone());
    let result = owner(&options, &stop, &sender, &mut report);
    stream::finish(result, &stop, &sender, report)
}
