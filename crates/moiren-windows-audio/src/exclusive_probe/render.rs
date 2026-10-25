//! Independent COM owner: Initialize itself may preempt existing shared streams.
use super::ExclusiveReport;
use crate::{
    StopSignal,
    catalog::{ApiFailure, FormatSnapshot},
    negotiation::{
        endpoint_data_flow, exclusive_format_supported, exclusive_frames_duration,
        initialize_exclusive,
    },
    owner::{Apartment, Mmcss, OwnedHandle, Streaming, TaskMemory},
};
use anyhow::Context;
use std::{
    os::windows::io::AsRawHandle,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Instant,
};
use windows::{
    Win32::{
        Foundation::{
            E_INVALIDARG, E_UNEXPECTED, ERROR_TIMEOUT, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED,
            AUDCLNT_E_UNSUPPORTED_FORMAT, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator,
            MMDeviceEnumerator, WAVEFORMATEX, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateInstance},
            Threading::WaitForMultipleObjects,
        },
    },
    core::{HSTRING, Result},
};

pub(super) struct Owner {
    stop: StopSignal,
    worker: Option<JoinHandle<ExclusiveReport>>,
    finished: Arc<AtomicBool>,
    started: bool,
}

impl Owner {
    pub fn start(endpoint_id: String) -> anyhow::Result<Self> {
        let stop = StopSignal::new()?;
        let worker_stop = stop.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("exclusive-probe".into())
            .spawn(move || {
                let mut report = ExclusiveReport::default();
                if let Err(error) = serve(&endpoint_id, &worker_stop, &sender, &mut report) {
                    report
                        .errors
                        .push(ApiFailure::new(report.last_stage, error));
                }
                // Failed initialization must also unblock the control side.
                if !report.started {
                    let _ = sender.send(false);
                }
                worker_finished.store(true, Ordering::Release);
                report
            })?;
        // Construct the owner first: even a lost handshake is stopped and joined.
        let mut owner = Self {
            stop,
            worker: Some(worker),
            finished,
            started: false,
        };
        owner.started = receiver.recv().context("exclusive Start handshake lost")?;
        Ok(owner)
    }

    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.started,
            "exclusive initialization failed; see exclusive report"
        );
        anyhow::ensure!(
            !self.finished.load(Ordering::Acquire),
            "exclusive owner stopped unexpectedly"
        );
        Ok(())
    }

    pub fn finish(&mut self) -> anyhow::Result<ExclusiveReport> {
        self.stop.request_stop()?;
        self.worker
            .take()
            .context("exclusive owner already joined")?
            .join()
            .map_err(|_| anyhow::anyhow!("exclusive owner panicked"))
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.stop.request_stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn pcm_format(rate: u32, bits: u16, float: bool) -> WAVEFORMATEX {
    let align = 2 * (bits / 8);
    WAVEFORMATEX {
        wFormatTag: if float { 3 } else { 1 },
        nChannels: 2,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * u32::from(align),
        nBlockAlign: align,
        wBitsPerSample: bits,
        cbSize: 0,
    }
}

fn serve(
    endpoint_id: &str,
    stop: &StopSignal,
    ready: &mpsc::SyncSender<bool>,
    report: &mut ExclusiveReport,
) -> Result<()> {
    report.last_stage = "CoInitializeEx(exclusive)";
    let _apartment = Apartment::new()?;
    report.last_stage = "GetDevice / render flow";
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(endpoint_id))? };
    if endpoint_data_flow(&device)? != eRender {
        return Err(windows::core::Error::from_hresult(E_INVALIDARG));
    }
    let event = OwnedHandle::event()?;
    let mut client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    report.last_stage = "GetMixFormat / IsFormatSupported(Exclusive)";
    let mix = TaskMemory(unsafe { client.GetMixFormat()? });
    if mix.0.is_null() {
        return Err(windows::core::Error::from_hresult(E_UNEXPECTED));
    }
    // The CoTaskMem allocation retains the complete extensible format, if any.
    let candidates = [48_000, 44_100].map(|rate| {
        [
            pcm_format(rate, 16, false),
            pcm_format(rate, 24, false),
            pcm_format(rate, 32, false),
            pcm_format(rate, 32, true),
        ]
    });
    let mut selected = if exclusive_format_supported(&client, mix.0)? {
        Some(mix.0.cast_const())
    } else {
        None
    };
    for candidate in candidates.iter().flatten() {
        if selected.is_some() {
            break;
        }
        if exclusive_format_supported(&client, candidate)? {
            selected = Some(candidate as *const WAVEFORMATEX);
        }
    }
    let format =
        selected.ok_or_else(|| windows::core::Error::from_hresult(AUDCLNT_E_UNSUPPORTED_FORMAT))?;
    // Both selected sources remain alive through Initialize and alignment retry.
    let base = unsafe { format.read_unaligned() };
    if base.nSamplesPerSec == 0 || base.nBlockAlign == 0 {
        return Err(windows::core::Error::from_hresult(E_UNEXPECTED));
    }
    report.format = Some(FormatSnapshot::from_base(base));
    let mut period = 0;
    unsafe {
        client.GetDevicePeriod(Some(&mut period), None)?;
    }
    // Favor a relaxed default period over pushing the hardware's minimum.
    period = period.max(100_000);
    report.last_stage = "Initialize(Exclusive)";
    // SAFETY: `format` points to either the live GetMixFormat allocation or a
    // candidate kept alive through this owner call.
    let initial = unsafe { initialize_exclusive(&client, format, period) };
    if initial
        .as_ref()
        .is_err_and(|error| error.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED)
    {
        let frames = unsafe { client.GetBufferSize()? };
        period = exclusive_frames_duration(frames, base.nSamplesPerSec);
        // A failed client cannot be initialized twice; release and re-Activate.
        drop(client);
        client = unsafe { device.Activate(CLSCTX_ALL, None)? };
        report.alignment_retried = true;
        // SAFETY: same live descriptor remains valid across the client retry.
        unsafe { initialize_exclusive(&client, format, period) }?;
    } else {
        initial?;
    }
    report.last_stage = "SetEventHandle / GetBufferSize / GetService";
    unsafe {
        client.SetEventHandle(event.0)?;
    }
    let capacity = unsafe { client.GetBufferSize()? };
    if capacity == 0 {
        return Err(windows::core::Error::from_hresult(E_UNEXPECTED));
    }
    report.buffer_frames = capacity;
    let service: IAudioRenderClient = unsafe { client.GetService()? };
    report.last_stage = "prime exclusive silence";
    submit(&service, capacity)?;
    report.submitted_frames += u64::from(capacity);
    let _mmcss = Mmcss::audio_detailed().ok();
    report.last_stage = "Start(Exclusive)";
    let mut streaming = Streaming::start(&client)?;
    report.started = true;
    let start = Instant::now();
    let _ = ready.send(true);
    let handles = [HANDLE(stop.event.as_raw_handle()), event.0];
    let mut last_callback = Instant::now();
    let result = (|| {
        loop {
            report.last_stage = "exclusive audio / stop wait";
            match unsafe { WaitForMultipleObjects(&handles, false, 100) } {
                result if result == WAIT_OBJECT_0 => break,
                result if result.0 == WAIT_OBJECT_0.0 + 1 => {
                    report.last_stage = "GetBuffer / ReleaseBuffer(exclusive silence)";
                    // Event-driven exclusive mode always writes one FULL buffer.
                    submit(&service, capacity)?;
                    report.submitted_frames += u64::from(capacity);
                    report.callbacks += 1;
                    last_callback = Instant::now();
                }
                WAIT_TIMEOUT if last_callback.elapsed().as_secs() < 2 => continue,
                WAIT_TIMEOUT => {
                    return Err(windows::core::Error::from_hresult(
                        windows::core::HRESULT::from_win32(ERROR_TIMEOUT.0),
                    ));
                }
                _ => return Err(windows::core::Error::from_thread()),
            }
        }
        Ok(())
    })();
    report.elapsed_seconds = start.elapsed().as_secs_f64();
    // Preserve both the streaming error and a separate Stop failure.
    if let Err(error) = streaming.stop() {
        report
            .errors
            .push(ApiFailure::new("Stop(Exclusive)", error));
    } else {
        report.stop_succeeded = true;
    }
    result
}

fn submit(service: &IAudioRenderClient, capacity: u32) -> Result<()> {
    // Flagged SILENT the endpoint never reads these bytes; the contract is
    // documented on `capture_packet::borrow_capture_packet`.
    let _buffer = unsafe { service.GetBuffer(capacity)? };
    unsafe { service.ReleaseBuffer(capacity, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::WaitForSingleObject;

    fn waiting_owner() -> Owner {
        let stop = StopSignal::new().unwrap();
        let worker_stop = stop.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let worker = std::thread::spawn(move || {
            // Kernel-only owner: no COM activation and no audio hardware access.
            assert_eq!(
                unsafe { WaitForSingleObject(HANDLE(worker_stop.event.as_raw_handle()), 2_000) },
                WAIT_OBJECT_0
            );
            worker_finished.store(true, Ordering::Release);
            ExclusiveReport {
                started: true,
                stop_succeeded: true,
                ..Default::default()
            }
        });
        Owner {
            stop,
            worker: Some(worker),
            finished,
            started: true,
        }
    }

    #[test]
    fn abandoning_an_experiment_stops_and_joins_its_exclusive_owner() {
        let owner = waiting_owner();
        let finished = Arc::clone(&owner.finished);
        drop(owner);
        assert!(finished.load(Ordering::Acquire));
    }

    #[test]
    fn explicit_finish_waits_for_the_owner_report_and_releases_the_join_handle() {
        let mut owner = waiting_owner();
        assert!(owner.check().is_ok());
        assert!(owner.finish().unwrap().stop_succeeded);
        assert!(owner.worker.is_none());
        assert!(owner.check().is_err());
    }
}
