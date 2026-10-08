"""Condense local W00 JSON metadata; no audio payloads or third-party packages."""

import argparse
from collections import Counter
import json
import math
from pathlib import Path
import statistics


def distribution(values):
    ordered = sorted(values)
    if not ordered:
        return None

    def percentile(fraction):
        index = (len(ordered) - 1) * fraction
        low = math.floor(index)
        high = math.ceil(index)
        return ordered[low] + (ordered[high] - ordered[low]) * (index - low)

    return {
        "count": len(ordered),
        "min": ordered[0],
        "median": statistics.median(ordered),
        "p95": percentile(0.95),
        "p99": percentile(0.99),
        "max": ordered[-1],
    }


def fit(points, frequency, window):
    selected = [
        point for point in points
        if point["hresult"] == 0 and point["qpc_100ns"] > 0
        and window[0] <= point["qpc_100ns"] <= window[1]
    ]
    if len(selected) < 3 or frequency <= 0:
        return None
    q0 = selected[0]["qpc_100ns"]
    p0 = selected[0]["position_units"]
    xs = [(point["qpc_100ns"] - q0) / 10_000_000 for point in selected]
    ys = [(point["position_units"] - p0) / frequency for point in selected]
    if xs[-1] < 1 or any(b <= a for a, b in zip(xs, xs[1:])):
        return None
    if any(b < a for a, b in zip(ys, ys[1:])):
        return None
    mx = math.fsum(xs) / len(xs)
    my = math.fsum(ys) / len(ys)
    sxx = math.fsum((x - mx) ** 2 for x in xs)
    rate = math.fsum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sxx
    rms = math.sqrt(math.fsum(
        ((y - my) - rate * (x - mx)) ** 2 for x, y in zip(xs, ys)
    ) / len(xs)) * 1_000_000
    return {
        "samples": len(xs), "span_seconds": xs[-1], "rate_ratio": rate,
        "drift_ppm": (rate - 1) * 1_000_000, "rms_residual_us": rms,
    }


def summarize(report, trace_name):
    report["raw_trace"] = trace_name
    report["summary_note"] = (
        "Full before/after snapshots and run diagnostics retained. Streaming "
        "metadata condensed; 30-second fits are post-stop analysis. No PCM."
    )
    window = report["common_qpc_window_100ns"]
    for endpoint in report["endpoints"]:
        clocks = endpoint.pop("clock_points")
        device_clocks = endpoint.pop("device_clock_points", [])
        packets = endpoint.pop("capture_packets")
        demands = endpoint.pop("render_demands")
        endpoint["metadata_counts"] = {
            "clock_points": len(clocks), "capture_packets": len(packets),
            "render_demands": len(demands),
            "device_clock_points": len(device_clocks),
        }
        endpoint["device_clock_first_points"] = device_clocks[:6]
        endpoint["device_clock_last_points"] = device_clocks[-3:]
        endpoint["device_clock_hresult_histogram"] = dict(Counter(
            f"0x{point['hresult'] & 0xFFFFFFFF:08X}" for point in device_clocks
        ))
        endpoint["clock_first_points"] = clocks[:6]
        endpoint["clock_last_points"] = clocks[-3:]
        endpoint["clock_qpc_step_ms"] = distribution([
            (right["qpc_100ns"] - left["qpc_100ns"]) / 10_000
            for left, right in zip(clocks, clocks[1:])
            if left["hresult"] == right["hresult"] == 0 and left["qpc_100ns"] > 0
        ])
        endpoint["clock_reused_qpc_examples"] = [
            {"before": left, "after": right}
            for left, right in zip(clocks, clocks[1:])
            if left["qpc_100ns"] == right["qpc_100ns"] > 0
            and left["position_units"] != right["position_units"]
        ][:3]
        metadata = packets if endpoint["flow"] == "capture" else demands
        endpoint["arrival_interval_ms"] = distribution([
            right["arrival_ms"] - left["arrival_ms"]
            for left, right in zip(metadata, metadata[1:])
        ])
        endpoint["packet_frame_histogram"] = dict(Counter(p["frames"] for p in packets))
        endpoint["render_writable_histogram"] = dict(Counter(p["writable_frames"] for p in demands))
        endpoint["render_padding_histogram"] = dict(Counter(p["padding_frames"] for p in demands))
        endpoint["packet_position_gap_count"] = sum(
            right["device_position_frames"] != left["device_position_frames"] + left["frames"]
            for left, right in zip(packets, packets[1:])
            if not (left["flags"] | right["flags"]) & 4
        )
        endpoint["packet_qpc_step_ms"] = distribution([
            (right["qpc_100ns"] - left["qpc_100ns"]) / 10_000
            for left, right in zip(packets, packets[1:])
            if not (left["flags"] | right["flags"]) & 4
        ])
        if endpoint["flow"] == "capture":
            points = [
                {"qpc_100ns": p["qpc_100ns"], "position_units": p["device_position_frames"],
                 "hresult": 0 if p["flags"] & 4 == 0 else -1}
                for p in packets
            ]
            frequency = endpoint["format"]["sample_rate"]
            reference = endpoint["capture_packet_clock"]["fit"]
        else:
            points = clocks
            frequency = endpoint["frequency_units_per_second"]
            reference = endpoint["clock"]["fit"]
        endpoint["analysis_30s_windows"] = []
        if window and reference:
            independent = fit(points, frequency, window)
            if not independent or abs(independent["drift_ppm"] - reference["drift_ppm"]) > 0.001:
                raise ValueError("Independent fit disagrees with Rust result")
            endpoint["independent_fit_check"] = independent
            for start in range(window[0], window[1], 300_000_000):
                end = min(start + 300_000_000, window[1])
                endpoint["analysis_30s_windows"].append({
                    "qpc_window_100ns": [start, end], "fit": fit(points, frequency, (start, end))
                })
        hardware = endpoint.get("device_clock")
        if window and hardware and hardware["fit"]:
            independent = fit(device_clocks, 1, window)
            if not independent or abs(independent["rate_ratio"] - hardware["fit"]["frames_per_second"]) > 0.0001:
                raise ValueError("Independent device-clock fit disagrees with Rust result")
            endpoint["independent_device_clock_fit_check"] = {
                "samples": independent["samples"],
                "span_seconds": independent["span_seconds"],
                "frames_per_second": independent["rate_ratio"],
                "rms_residual_frames": independent["rms_residual_us"] / 1_000_000,
            }
    names = {e["endpoint_id"]: e["name"] for e in report["endpoints"]}
    for comparison in report["relative_clocks"]:
        comparison["left_name"] = names[comparison["left_endpoint"]]
        comparison["right_name"] = names[comparison["right_endpoint"]]
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("summary", type=Path)
    args = parser.parse_args()
    report = json.loads(args.trace.read_text(encoding="utf-8-sig"))
    summary = summarize(report, args.trace.as_posix())
    args.summary.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Saved {args.summary}; independent fits checked against Rust report.")


if __name__ == "__main__":
    main()
