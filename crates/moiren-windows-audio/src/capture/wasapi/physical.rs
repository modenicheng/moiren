//! Native preparation, streaming and destruction run on one COM owner thread.
use super::{
    Startup, Wake, api, format::mix_format, handle, preparation::CapturePreparation, stream,
    wait_target,
};
use crate::ActivationGate;
use crate::negotiation::{OpenedSharedClient, SharedOpenStages, open_shared_client};
use crate::{
    capture::{CaptureError, CaptureOptions, CaptureReport, CaptureSource, CaptureStatus},
    owner::Apartment,
};
use std::{
    os::windows::io::OwnedHandle as StdHandle,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, atomic::AtomicBool, mpsc::SyncSender},
};

const OPEN_STAGES: SharedOpenStages = SharedOpenStages::capture();

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
    let (opened, client_format) = open_shared_client(
        OPEN_STAGES,
        &options.endpoint_id,
        true,
        report,
        |report, memory| -> Result<_, CaptureError> {
            let format = mix_format(memory)?;
            // Packet lengths and bridge timing use the selected client descriptor,
            // which can differ from the endpoint mix rate and microphone-array layout.
            let base = format.client();
            report.sample_rate = Some(base.sample_rate);
            report.channels = Some(usize::from(base.channels));
            Ok((format, (base.sample_rate, usize::from(base.channels))))
        },
        |report, client, format, guid| -> Result<(), CaptureError> {
            crate::shared_stream::initialize_shared(
                client,
                format,
                options.shared_period,
                guid,
                &mut report.shared_period,
            )
            .map_err(|error| CaptureError::Api {
                stage: error.stage,
                hresult: error.error.code().0,
            })?;
            report.windows_auto_conversion = format.windows_auto_conversion;
            Ok(())
        },
    )?;
    let OpenedSharedClient { client, event, .. } = opened;
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
            sample_rate: client_format.0,
            channels: client_format.1,
            artifact: options.artifact,
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
    run_owner_with(&stop, &preparation, report, |report| {
        owner(&options, &stop, &preparation, report, &gate, &started)
    })
}

fn run_owner_with(
    stop: &StdHandle,
    preparation: &CapturePreparation,
    mut report: CaptureReport,
    run: impl FnOnce(&mut CaptureReport) -> Result<CaptureStatus, CaptureError>,
) -> CaptureReport {
    // Prepared evidence stays in the report outside the unwind scope. Native
    // COM can unwind here, but the control joiner reclaims the bounded ring.
    let result = catch_unwind(AssertUnwindSafe(|| run(&mut report)))
        .unwrap_or(Err(CaptureError::WorkerPanicked));
    preparation.finish(result, stop, report)
}

#[cfg(test)]
mod tests;
