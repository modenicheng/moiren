//! Event-loop phases share the owner's time and metadata limits.
use super::{unexpected, write_silence};
use crate::{
    clock::ClockPoint,
    owner::PacketLease,
    physical::{DemandRecord, EndpointReport},
    probe::PacketRecord,
    stats::{CaptureStats, SILENT, analyze_f32},
};
use std::time::{Duration, Instant};
use windows::{
    Win32::Media::Audio::{
        IAudioCaptureClient, IAudioClient, IAudioClock, IAudioClock2, IAudioRenderClient,
        WAVEFORMATEX,
    },
    core::{Interface, Result},
};

// One time window governs both packet draining and bounded metadata recording.
// Otherwise a busy capture endpoint could keep draining past the requested end.
pub(super) struct SamplingWindow {
    pub(super) start: Instant,
    pub(super) duration: Duration,
    pub(super) metadata_limit: usize,
}

pub(super) fn sample_clocks(
    report: &mut EndpointReport,
    clock: &IAudioClock,
    device_clock: Option<&IAudioClock2>,
    window: &SamplingWindow,
) -> Result<()> {
    report.last_stage = "IAudioClock::GetPosition (before buffer processing)";
    let (mut position, mut qpc) = (0, 0);
    // Preserve S_FALSE, which the convenience binding converts into Ok(()).
    let hr = unsafe {
        (Interface::vtable(clock).GetPosition)(Interface::as_raw(clock), &mut position, &mut qpc)
    };
    if report.clock_points.len() < window.metadata_limit {
        report.clock_points.push(ClockPoint {
            arrival_ms: window.start.elapsed().as_secs_f64() * 1000.0,
            position_units: position,
            qpc_100ns: qpc,
            hresult: hr.0,
        });
    } else {
        report.metadata_dropped += 1;
    }
    hr.ok()?;
    if let Some(device_clock) = device_clock {
        let (mut position, mut qpc) = (0, 0);
        let hr = unsafe {
            (Interface::vtable(device_clock).GetDevicePosition)(
                Interface::as_raw(device_clock),
                &mut position,
                &mut qpc,
            )
        };
        if report.device_clock_points.len() < window.metadata_limit {
            report.device_clock_points.push(ClockPoint {
                arrival_ms: window.start.elapsed().as_secs_f64() * 1000.0,
                position_units: position,
                qpc_100ns: qpc,
                hresult: hr.0,
            });
        } else {
            report.metadata_dropped += 1;
        }
        // Optional hardware-clock failures remain in the raw trace and
        // diagnostics; they do not suppress an otherwise working stream.
    }
    Ok(())
}

pub(super) fn capture_packets(
    report: &mut EndpointReport,
    capture: &IAudioCaptureClient,
    capacity: u32,
    base: WAVEFORMATEX,
    stats: &mut CaptureStats,
    window: &SamplingWindow,
) -> Result<()> {
    while window.start.elapsed() < window.duration {
        report.last_stage = "GetNextPacketSize";
        if unsafe { capture.GetNextPacketSize()? } == 0 {
            break;
        }
        let (mut data, mut frames, mut flags, mut position, mut qpc) =
            (std::ptr::null_mut(), 0, 0, 0, 0);
        report.last_stage = "GetBuffer";
        unsafe {
            capture.GetBuffer(
                &mut data,
                &mut frames,
                &mut flags,
                Some(&mut position),
                Some(&mut qpc),
            )?;
        }
        if frames == 0 {
            break;
        }
        let lease = PacketLease {
            client: capture,
            frames,
            released: false,
        };
        report.last_stage = "capture packet validation";
        if frames > capacity || (flags & SILENT == 0 && data.is_null()) {
            return Err(unexpected());
        }
        let bytes = if flags & SILENT != 0 {
            &[]
        } else {
            // SAFETY: native f32 validation fixes the frame width; the checked
            // packet lies within the live lease, which outlives these statistics.
            unsafe {
                std::slice::from_raw_parts(data, frames as usize * usize::from(base.nBlockAlign))
            }
        };
        let metrics = analyze_f32(bytes, frames, base.nChannels, flags & SILENT != 0)
            .map_err(|_| unexpected())?;
        if report.capture_packets.len() < window.metadata_limit {
            report.capture_packets.push(PacketRecord {
                arrival_ms: window.start.elapsed().as_secs_f64() * 1000.0,
                frames,
                flags,
                device_position_frames: position,
                qpc_100ns: qpc,
                rms: metrics.rms(),
                peak: metrics.peak,
            });
        } else {
            report.metadata_dropped += 1;
        }
        stats.observe(frames, flags, position, qpc, metrics);
        report.last_stage = "ReleaseBuffer(capture)";
        lease.release()?;
    }
    Ok(())
}

pub(super) fn render_silence(
    report: &mut EndpointReport,
    client: &IAudioClient,
    render: &IAudioRenderClient,
    capacity: u32,
    window: &SamplingWindow,
) -> Result<()> {
    report.last_stage = "GetCurrentPadding";
    let padding = unsafe { client.GetCurrentPadding()? };
    let writable = capacity.checked_sub(padding).ok_or_else(unexpected)?;
    let summary = report.render.as_mut().ok_or_else(unexpected)?;
    summary.zero_demand_wakes += u64::from(writable == 0);
    summary.empty_padding_wakes += u64::from(padding == 0);
    summary.min_writable_frames = Some(
        summary
            .min_writable_frames
            .map_or(writable, |n| n.min(writable)),
    );
    summary.max_writable_frames = summary.max_writable_frames.max(writable);
    if writable != 0 {
        report.last_stage = "GetBuffer / ReleaseBuffer(render silence)";
        write_silence(render, writable)?;
        summary.submitted_frames += u64::from(writable);
    }
    if report.render_demands.len() < window.metadata_limit {
        report.render_demands.push(DemandRecord {
            arrival_ms: window.start.elapsed().as_secs_f64() * 1000.0,
            padding_frames: padding,
            writable_frames: writable,
        });
    } else {
        report.metadata_dropped += 1;
    }
    Ok(())
}
