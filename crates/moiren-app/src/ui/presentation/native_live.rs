//! Pure live native diagnostics. The caller applies phase/generation/running
//! fencing before delegation; none of these rows grant engine authority.

use moiren_core::graph::NodeId;
use moiren_windows_audio::{
    clock_bridge::BridgeSnapshot, engine_backend::NativeObservationSnapshot,
    render::RenderPaddingSnapshot,
};
use rust_i18n::t;

pub(super) fn append_rows(
    rows: &mut Vec<(String, String)>,
    observation: &NativeObservationSnapshot,
) {
    for bridge in &observation.bridges {
        let prefix = stream_prefix(observation, bridge.node);
        append_delivery(rows, &prefix, bridge.counters);
        append_age(rows, &prefix, bridge.counters);
    }
    for padding in &observation.render_padding {
        let prefix = stream_prefix(observation, padding.node);
        append_padding(rows, &prefix, padding.sample);
    }
    rows.push((
        t!("Physical end-to-end latency").into(),
        t!("Unknown / unqualified").into(),
    ));
}

fn stream_prefix(observation: &NativeObservationSnapshot, node: NodeId) -> String {
    observation
        .streams
        .iter()
        .position(|stream| stream.node == node)
        .map(|index| t!("Stream %{index}", index = index + 1).into())
        .unwrap_or_else(|| t!("Unmapped stream").into())
}

fn unknown() -> String {
    t!("Unknown").into()
}

fn frames(value: Option<u64>) -> String {
    value
        .map(|frames| t!("%{frames} frames", frames = frames).into())
        .unwrap_or_else(unknown)
}

fn append_delivery(rows: &mut Vec<(String, String)>, prefix: &str, counters: BridgeSnapshot) {
    let delivery = counters.native_delivery;
    // Zero packets is an observed counter but is not packet timing evidence.
    // Zero gaps likewise cannot authorize displaying a retained max as timing.
    let observed = delivery.filter(|sample| sample.packets != 0);
    rows.extend([
        (
            t!("%{prefix}: native delivery packets", prefix = prefix).into(),
            delivery
                .map(|sample| t!("%{count} packets", count = sample.packets).into())
                .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: maximum native packet", prefix = prefix).into(),
            frames(observed.and_then(|sample| {
                (sample.max_packet_frames != 0).then_some(u64::from(sample.max_packet_frames))
            })),
        ),
        (
            t!("%{prefix}: maximum callback burst", prefix = prefix).into(),
            frames(observed.and_then(|sample| {
                (sample.max_callback_burst_frames != 0).then_some(sample.max_callback_burst_frames)
            })),
        ),
        (
            t!("%{prefix}: native arrival gaps", prefix = prefix).into(),
            delivery
                .map(|sample| {
                    if sample.packets == 0 || sample.gap_samples == 0 {
                        t!(
                            "%{count} observations; timing unknown",
                            count = sample.gap_samples,
                        )
                        .into()
                    } else {
                        t!(
                            "%{count} observations; max %{ticks} x 100 ns",
                            count = sample.gap_samples,
                            ticks = sample.max_gap_100ns,
                        )
                        .into()
                    }
                })
                .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: delivery bound violations", prefix = prefix).into(),
            delivery
                .map(|sample| {
                    t!(
                        "Packet %{packet} / gap %{gap} observations",
                        packet = sample.packet_bound_violations,
                        gap = sample.gap_bound_violations,
                    )
                    .into()
                })
                .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: delivery clock observations", prefix = prefix).into(),
            delivery
                .map(|sample| {
                    t!(
                        "Unavailable %{unavailable} / regressed %{regressed} observations",
                        unavailable = sample.unavailable_clocks,
                        regressed = sample.regressed_clocks,
                    )
                    .into()
                })
                .unwrap_or_else(unknown),
        ),
    ]);
}

fn append_age(rows: &mut Vec<(String, String)>, prefix: &str, counters: BridgeSnapshot) {
    // Only this DTO correlates the actual oldest pending frame. Fill/rate and
    // last_qpc_100ns describe different things and cannot substitute for None.
    let age = counters.oldest_frame_age;
    rows.extend([
        (
            t!("%{prefix}: oldest pending frame age estimate", prefix = prefix).into(),
            age.map(|age| {
                t!(
                    "%{ticks} x 100 ns (owner-local estimate, not physical E2E)",
                    ticks = age.estimated_age_100ns,
                )
                .into()
            })
            .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: oldest frame correlation", prefix = prefix).into(),
            age.map(|age| {
                t!(
                    "Generation %{generation}; packet position %{position} frames; offset %{offset} / accepted %{accepted} frames",
                    generation = age.stamp.generation,
                    position = age.stamp.packet_position_frames,
                    offset = age.stamp.packet_offset_frames,
                    accepted = age.stamp.packet_accepted_frames,
                )
                .into()
            })
            .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: age QPC correlation", prefix = prefix).into(),
            age.map(|age| {
                t!(
                    "Packet %{packet} / frame %{frame} / observed %{observed} x 100 ns",
                    packet = age.stamp.packet_qpc_100ns,
                    frame = age.estimated_frame_qpc_100ns,
                    observed = age.observed_qpc_100ns,
                )
                .into()
            })
            .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: age rate basis", prefix = prefix).into(),
            age.map(|age| match age.stamp.rate_correction_ppm {
                Some(ppm) => t!(
                    "%{rate} Hz; qualified Capture/QPC correction %{ppm} ppm",
                    rate = age.input_sample_rate,
                    ppm = ppm,
                )
                .into(),
                None => t!(
                    "%{rate} Hz nominal rate (Capture/QPC correction unqualified)",
                    rate = age.input_sample_rate,
                )
                .into(),
            })
            .unwrap_or_else(unknown),
        ),
    ]);
}

fn append_padding(
    rows: &mut Vec<(String, String)>,
    prefix: &str,
    sample: Option<RenderPaddingSnapshot>,
) {
    rows.extend([
        (
            t!("%{prefix}: native render padding", prefix = prefix).into(),
            sample
                .map(|sample| {
                    t!(
                        "%{padding} / capacity %{capacity} frames (native queue only, not E2E)",
                        padding = sample.padding_frames,
                        capacity = sample.capacity_frames,
                    )
                    .into()
                })
                .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: observed render rate", prefix = prefix).into(),
            sample
                .and_then(|sample| sample.sample_rate)
                .map(|rate| t!("%{rate} Hz", rate = rate).into())
                .unwrap_or_else(unknown),
        ),
        (
            t!("%{prefix}: observed render period", prefix = prefix).into(),
            frames(sample.and_then(|sample| sample.period_frames.map(u64::from))),
        ),
        (
            t!("%{prefix}: render sample sequence", prefix = prefix).into(),
            sample
                .map(|sample| sample.sequence.to_string())
                .unwrap_or_else(unknown),
        ),
    ]);
}
