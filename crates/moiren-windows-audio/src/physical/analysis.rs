//! Clock fitting happens after all endpoint owners stop, so its temporary
//! vectors and pairwise comparisons cannot allocate on a streaming thread.
use super::{EndpointReport, RelativeClock};
use crate::{
    clock::{ClockPoint, analyze_clock, analyze_device_clock, relative_ppm},
    stats::TIMESTAMP_ERROR,
};
use windows::Win32::Foundation::E_UNEXPECTED;

pub(super) fn analyze_endpoints(
    endpoints: &mut [EndpointReport],
) -> (Option<(u64, u64)>, Vec<RelativeClock>) {
    let comparison_points = comparison_points(endpoints);
    let window = common_window(&comparison_points);
    for (endpoint, points) in endpoints.iter_mut().zip(&comparison_points) {
        analyze_endpoint(endpoint, points, window);
    }
    (window, compare_rates(endpoints, window))
}

fn comparison_points(endpoints: &[EndpointReport]) -> Vec<Vec<ClockPoint>> {
    // Packet timestamps refer to the first captured frame. Preserve the separate
    // IAudioClock observations, including stale or inconsistent capture QPC pairs.
    // These post-stop vectors contain metadata only.
    endpoints
        .iter()
        .map(|endpoint| {
            if endpoint.flow == "capture" {
                endpoint
                    .capture_packets
                    .iter()
                    .map(|packet| ClockPoint {
                        arrival_ms: packet.arrival_ms,
                        position_units: packet.device_position_frames,
                        qpc_100ns: packet.qpc_100ns,
                        hresult: if packet.flags & TIMESTAMP_ERROR == 0 {
                            0
                        } else {
                            E_UNEXPECTED.0
                        },
                    })
                    .collect()
            } else {
                endpoint.clock_points.clone()
            }
        })
        .collect()
}

fn common_window(comparison_points: &[Vec<ClockPoint>]) -> Option<(u64, u64)> {
    // Exclude the first second and fit all endpoints over the same correlated QPC interval.
    let usable = |p: &&ClockPoint| p.hresult == 0 && p.qpc_100ns > 0 && p.arrival_ms >= 1000.0;
    let starts: Vec<_> = comparison_points
        .iter()
        .filter_map(|points| points.iter().find(usable).map(|p| p.qpc_100ns))
        .collect();
    let ends: Vec<_> = comparison_points
        .iter()
        .filter_map(|points| points.iter().rev().find(usable).map(|p| p.qpc_100ns))
        .collect();
    starts
        .iter()
        .max()
        .zip(ends.iter().min())
        .and_then(|(&start, &end)| (start < end).then_some((start, end)))
}

fn analyze_endpoint(
    endpoint: &mut EndpointReport,
    points: &[ClockPoint],
    window: Option<(u64, u64)>,
) {
    if !endpoint.device_clock_points.is_empty() {
        let mut summary = analyze_device_clock(&endpoint.device_clock_points, None);
        if summary.diagnostics.regressions == 0
            && summary.diagnostics.inconsistent_qpc_reads == 0
            && let Some(window) = window
        {
            summary.fit = analyze_device_clock(&endpoint.device_clock_points, Some(window)).fit;
        }
        endpoint.device_clock = Some(summary);
    }
    endpoint.clock = analyze_clock(
        &endpoint.clock_points,
        endpoint.frequency_units_per_second.unwrap_or(0),
        None,
    );
    if endpoint.clock.regressions == 0
        && endpoint.clock.inconsistent_qpc_reads == 0
        && let Some(window) = window
    {
        endpoint.clock.fit = analyze_clock(
            &endpoint.clock_points,
            endpoint.frequency_units_per_second.unwrap_or(0),
            Some(window),
        )
        .fit;
    }
    if endpoint.flow == "capture" {
        let frequency = endpoint
            .format
            .as_ref()
            .map_or(0, |f| u64::from(f.sample_rate));
        let mut packet_clock = analyze_clock(points, frequency, None);
        if packet_clock.regressions == 0
            && packet_clock.inconsistent_qpc_reads == 0
            && let Some(window) = window
        {
            packet_clock.fit = analyze_clock(points, frequency, Some(window)).fit;
        }
        if endpoint
            .capture
            .as_ref()
            .is_some_and(|capture| capture.later_discontinuities > 0)
        {
            packet_clock.fit = None;
        }
        endpoint.capture_packet_clock = Some(packet_clock);
        endpoint.comparison_clock_source = "capture_packet_frames / sample_rate";
    } else if endpoint.flow == "render" {
        endpoint.comparison_clock_source = "IAudioClock_position / GetFrequency";
    }
}

fn compare_rates(endpoints: &[EndpointReport], window: Option<(u64, u64)>) -> Vec<RelativeClock> {
    let mut relative_clocks = Vec::new();
    for (i, left) in endpoints.iter().enumerate() {
        for right in &endpoints[i + 1..] {
            let left_clock = left.capture_packet_clock.as_ref().unwrap_or(&left.clock);
            let right_clock = right.capture_packet_clock.as_ref().unwrap_or(&right.clock);
            let ppm = window.and_then(|_| {
                left_clock
                    .fit
                    .as_ref()
                    .zip(right_clock.fit.as_ref())
                    .and_then(|(l, r)| relative_ppm(l.rate_ratio, r.rate_ratio))
            });
            relative_clocks.push(RelativeClock {
                left_endpoint: left.endpoint_id.clone(),
                right_endpoint: right.endpoint_id.clone(),
                left_faster_ppm: ppm,
            });
        }
    }
    relative_clocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{catalog::FormatSnapshot, probe::PacketRecord, stats::CaptureSummary};
    use windows::Win32::Media::Audio::WAVEFORMATEX;

    fn render_endpoint(id: &str, offset_seconds: u64) -> EndpointReport {
        let mut endpoint = EndpointReport::new(id.into(), None);
        endpoint.flow = "render";
        // This byte-based clock advances eight units per stereo f32 frame.
        endpoint.frequency_units_per_second = Some(384_000);
        endpoint.clock_points = (0..=6)
            .map(|second| ClockPoint {
                arrival_ms: second as f64 * 1000.0,
                position_units: second * 384_000,
                qpc_100ns: (20 + offset_seconds + second) * 10_000_000,
                hresult: 0,
            })
            .collect();
        endpoint
    }

    fn capture_endpoint() -> EndpointReport {
        let mut endpoint = render_endpoint("capture", 1);
        endpoint.flow = "capture";
        endpoint.format = Some(FormatSnapshot::from_base(WAVEFORMATEX {
            wFormatTag: 3,
            nSamplesPerSec: 48_000,
            ..Default::default()
        }));
        endpoint.capture = Some(CaptureSummary::default());
        endpoint.capture_packets = endpoint
            .clock_points
            .iter()
            .enumerate()
            .map(|(second, point)| PacketRecord {
                arrival_ms: point.arrival_ms,
                frames: 48_048,
                flags: 0,
                device_position_frames: second as u64 * 48_048,
                qpc_100ns: point.qpc_100ns,
                rms: 0.0,
                peak: 0.0,
            })
            .collect();
        for point in &mut endpoint.clock_points {
            point.position_units /= 2;
        }
        endpoint
    }

    #[test]
    fn capture_frames_and_render_bytes_use_distinct_rates_in_the_same_window() {
        let mut endpoints = [capture_endpoint(), render_endpoint("render", 0)];
        let (window, comparisons) = analyze_endpoints(&mut endpoints);

        assert_eq!(window, Some((220_000_000, 260_000_000)));
        assert_eq!(endpoints[0].clock.fit.as_ref().unwrap().rate_ratio, 0.5);
        let capture_fit = endpoints[0]
            .capture_packet_clock
            .as_ref()
            .unwrap()
            .fit
            .as_ref()
            .unwrap();
        assert!((capture_fit.rate_ratio - 1.001).abs() < 1e-12);
        assert_eq!(capture_fit.span_seconds, 4.0);
        assert_eq!(endpoints[1].clock.fit.as_ref().unwrap().span_seconds, 4.0);
        assert!((comparisons[0].left_faster_ppm.unwrap() - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn late_capture_discontinuity_invalidates_comparison_despite_a_valid_api_clock() {
        let mut endpoints = [capture_endpoint(), render_endpoint("render", 0)];
        endpoints[0].capture.as_mut().unwrap().later_discontinuities = 1;
        let (_, comparisons) = analyze_endpoints(&mut endpoints);

        assert!(endpoints[0].clock.fit.is_some());
        assert!(
            endpoints[0]
                .capture_packet_clock
                .as_ref()
                .unwrap()
                .fit
                .is_none()
        );
        assert!(comparisons[0].left_faster_ppm.is_none());
    }

    #[test]
    fn nonoverlapping_probes_keep_individual_fits_without_a_relative_rate() {
        let mut endpoints = [render_endpoint("early", 0), render_endpoint("late", 20)];
        let (window, comparisons) = analyze_endpoints(&mut endpoints);

        assert!(window.is_none());
        assert!(
            endpoints
                .iter()
                .all(|endpoint| endpoint.clock.fit.is_some())
        );
        assert!(comparisons[0].left_faster_ppm.is_none());
    }
}
