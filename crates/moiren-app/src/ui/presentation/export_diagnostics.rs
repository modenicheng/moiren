//! Detached provider evidence can outlive an audio session or endpoint. Keep
//! its last observation separate from the service's Engine authority fields.

use crate::service::{ExportDirection, ExportResourceState, ExportStatus};
use rust_i18n::t;

/// Pure control-side formatting; no diagnostic value authorizes a gate or
/// supplies an active revision. Do not apply the live-session timing filter.
pub fn export_diagnostic_rows(status: &ExportStatus) -> Vec<(String, String)> {
    let windows_name = status.windows_name.as_ref();

    let windows_name_text = windows_name
        .and_then(|observation| observation.name.clone())
        .unwrap_or_else(|| t!("Unknown / not observed").into());
    let mut rows = vec![
        (
            t!("Stable endpoint ID").into(),
            status.intent.endpoint_id.to_string(),
        ),
        (
            t!("Export direction").into(),
            match status.intent.direction {
                ExportDirection::Playback => t!("Playback").into(),
                ExportDirection::Microphone => t!("Microphone").into(),
            },
        ),
        (
            t!("Requested endpoint name").into(),
            status.intent.name.clone(),
        ),
        (
            t!("Provider-created name").into(),
            status
                .actual_name
                .clone()
                .unwrap_or_else(|| t!("Unknown / not present").into()),
        ),
        (t!("Windows endpoint name").into(), windows_name_text),
        (
            t!("Windows name match").into(),
            windows_name_match_text(status),
        ),
    ];
    if let Some(observation) = windows_name {
        if let Some(error) = &observation.error {
            rows.push((t!("Windows name readback error").into(), error.clone()));
        }
        if let Some(mmdevice_id) = &observation.mmdevice_id {
            rows.push((t!("Windows name MMDevice ID").into(), mmdevice_id.clone()));
        }
        if let Some(qpc) = observation.observed_qpc_100ns {
            rows.push((
                t!("Windows name observation QPC (100 ns)").into(),
                qpc.to_string(),
            ));
        }
    }
    if status.rename_pending {
        rows.push((
            t!("Rename").into(),
            t!("Name pending; waiting for native recreation").into(),
        ));
    }
    rows.extend([
        (
            t!("Endpoint").into(),
            match status.resource {
                ExportResourceState::Unavailable => t!("Unavailable").into(),
                ExportResourceState::Absent => t!("Absent").into(),
                ExportResourceState::Creating => t!("Creating").into(),
                ExportResourceState::Present => t!("Present (not a flow signal)").into(),
                ExportResourceState::Removing => t!("Removing / awaiting confirmation").into(),
            },
        ),
        (
            t!("Gate admission").into(),
            if status.gate_open {
                t!("Open (not a flow signal)").into()
            } else {
                t!("Closed").into()
            },
        ),
        (
            t!("Engine-confirmed binding").into(),
            status
                .active_revision
                .map(|revision| t!("Active revision %{revision}", revision = revision).into())
                .unwrap_or_else(|| t!("Inactive").into()),
        ),
        (
            t!("Diagnostic scope").into(),
            t!("Latest native export observation (not live audio)").into(),
        ),
    ]);
    #[cfg(windows)]
    if let Some(diagnostics) = &status.diagnostics {
        rows.extend([
            (
                t!("Observation generation").into(),
                diagnostics.generation.to_string(),
            ),
            (
                t!("Native transport preparation").into(),
                if diagnostics.transport_prepared {
                    t!("Prepared (native observation only)").into()
                } else {
                    t!("Not prepared").into()
                },
            ),
            (
                t!("Endpoint error").into(),
                diagnostics
                    .error
                    .clone()
                    .unwrap_or_else(|| t!("None reported").into()),
            ),
        ]);
        if let Some(telemetry) = &diagnostics.telemetry {
            rows.extend(telemetry_rows(telemetry));
            rows.extend(packet_service_rows(telemetry));
            // Only playback stamps carry the native sequence. Microphone
            // stamps describe host submissions and cannot establish these holes.
            if status.intent.direction == ExportDirection::Playback {
                rows.push((
                    t!("Playback native sequence gaps").into(),
                    t!(
                        "%{observations} gap observations / %{slots} unobserved native sequence slots (not proof of signal loss; first-packet prefix unknown)",
                        observations = telemetry.sequence_gap_observations,
                        slots = telemetry.missing_sequence_slots,
                    )
                    .into(),
                ));
            }
        } else {
            rows.push((
                t!("Provider packet telemetry").into(),
                t!("Unavailable").into(),
            ));
        }
        return rows;
    }
    rows.extend([
        (
            t!("Native export diagnostics").into(),
            t!("Unavailable").into(),
        ),
        (
            t!("Native transport preparation").into(),
            t!("Unavailable").into(),
        ),
        (t!("Endpoint error").into(), t!("Unavailable").into()),
        (
            t!("Provider packet telemetry").into(),
            t!("Unavailable").into(),
        ),
    ]);
    rows
}

#[cfg(windows)]
fn packet_service_rows(
    telemetry: &moiren_windows_audio::vad::VadPcmTelemetrySnapshot,
) -> [(String, String); 5] {
    [
        (
            t!("Playback packet service").into(),
            packet_service_text(&telemetry.playback_transfer_service),
        ),
        (
            t!("Playback service clock diagnostics").into(),
            packet_service_clock_text(&telemetry.playback_transfer_service),
        ),
        (
            t!("Microphone packet service").into(),
            packet_service_text(&telemetry.microphone_submission_service),
        ),
        (
            t!("Microphone service clock diagnostics").into(),
            packet_service_clock_text(&telemetry.microphone_submission_service),
        ),
        (
            t!("Packet service observation scope").into(),
            t!("Completion service delay only; not source sample age or physical latency; independent live summaries.").into(),
        ),
    ]
}

#[cfg(windows)]
fn packet_service_text(
    summary: &moiren_windows_audio::latency::packet_service::PacketServiceSnapshot,
) -> String {
    use moiren_windows_audio::latency::packet_service::PacketStampOrigin;
    // The field's direction does not supply missing timestamp provenance.
    let origin = match summary.origin {
        PacketStampOrigin::Unknown => t!("Unknown stamp origin"),
        PacketStampOrigin::DriverTransfer => t!("Driver transfer -> owner collection"),
        PacketStampOrigin::HostSubmission => t!("Host submission -> owner collection"),
    };
    match (summary.min_delay_100ns, summary.max_delay_100ns) {
        (Some(min), Some(max))
            if summary.origin != PacketStampOrigin::Unknown
                && summary.qpc_frequency_hz.is_some_and(|hz| hz > 0)
                && summary.observations > 0
                && min <= max =>
        {
            // No floating-point conversion or ms-derived sample-age claim:
            // retain every integer digit and the raw summary's 100 ns units.
            t!(
                "%{origin}; %{count} observations; min %{min} / max %{max} x 100 ns",
                origin = origin,
                count = summary.observations,
                min = min,
                max = max,
            )
            .into()
        }
        _ => t!(
            "%{origin}; %{count} observations; service delay unknown",
            origin = origin,
            count = summary.observations,
        )
        .into(),
    }
}

#[cfg(windows)]
fn packet_service_clock_text(
    summary: &moiren_windows_audio::latency::packet_service::PacketServiceSnapshot,
) -> String {
    let frequency = summary
        .qpc_frequency_hz
        .filter(|&hz| hz > 0)
        .map(|hz| t!("%{hz} Hz", hz = hz).to_string())
        .unwrap_or_else(|| t!("Unknown").into());
    t!(
        "QPC %{frequency}; unavailable %{unavailable} / future stamps %{future} / conversion overflows %{overflows}",
        frequency = frequency,
        unavailable = summary.unavailable_clocks,
        future = summary.future_stamps,
        overflows = summary.conversion_overflows,
    )
    .into()
}

pub fn windows_name_match_text(status: &ExportStatus) -> String {
    if let Some(observation) = status.windows_name.as_ref() {
        return match observation.name.as_deref() {
            Some(name) if name == status.intent.name => t!("Matches requested name").into(),
            Some(_) => t!("Does not match requested name").into(),
            None if observation.error.is_some() => t!("Read failed").into(),
            None => t!("Unknown / not observed").into(),
        };
    }
    if status.rename_pending {
        t!("Pending native recreation").into()
    } else {
        t!("Unknown / not observed").into()
    }
}

#[cfg(windows)]
fn telemetry_rows(
    telemetry: &moiren_windows_audio::vad::VadPcmTelemetrySnapshot,
) -> Vec<(String, String)> {
    let frames = |count| {
        if count == 0 {
            t!("Unknown").to_string()
        } else {
            t!("%{frames} frames", frames = count).to_string()
        }
    };
    vec![
        (
            t!("Provider telemetry schema").into(),
            telemetry.schema_version.to_string(),
        ),
        (
            t!("Observed packet").into(),
            frames(telemetry.packet_frames),
        ),
        (
            t!("Maximum observed packet").into(),
            frames(telemetry.max_packet_frames),
        ),
        (
            t!("Completion interval observations").into(),
            if telemetry.completion_interval_observations == 0 {
                t!("0 observations; timing unknown").into()
            } else {
                // Preserve native 100 ns units and all integer digits. These
                // delivery observations are not physical latency measurements.
                t!(
                    "%{count} observations; max %{ticks} x 100 ns",
                    count = telemetry.completion_interval_observations,
                    ticks = telemetry.max_completion_interval_100ns,
                )
                .into()
            },
        ),
        (
            t!("Cumulative submissions").into(),
            t!(
                "%{attempts} attempts / %{submitted} submitted packets",
                attempts = telemetry.submission_attempts,
                submitted = telemetry.submitted_packets,
            )
            .into(),
        ),
        (
            t!("Cumulative completions").into(),
            t!("%{count} packets", count = telemetry.completed_packets).into(),
        ),
        (
            t!("Cumulative packet drops").into(),
            t!(
                "Driver %{driver} / quota %{quota} / carrier %{carrier} / total %{total} packets",
                driver = telemetry.driver_dropped_packets,
                quota = telemetry.quota_dropped_packets,
                carrier = telemetry.carrier_dropped_packets,
                total = telemetry.dropped_packets,
            )
            .into(),
        ),
        (
            t!("Cumulative stale packets").into(),
            t!("%{count} packets", count = telemetry.stale_packets).into(),
        ),
        (
            t!("Cumulative silence").into(),
            t!(
                "%{packets} packets / %{frames} frames",
                packets = telemetry.silence_packets,
                frames = telemetry.silence_frames,
            )
            .into(),
        ),
        (
            t!("Cumulative epoch resets").into(),
            telemetry.epoch_resets.to_string(),
        ),
        (
            t!("Cumulative recoveries").into(),
            telemetry.recovery_events.to_string(),
        ),
        (
            t!("Cumulative completion clock regressions").into(),
            telemetry.completion_clock_regressions.to_string(),
        ),
    ]
}
