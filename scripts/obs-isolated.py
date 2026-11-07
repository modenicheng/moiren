#!/usr/bin/env python3
"""Analyze two isolated OBS tracks with a retained no-Moiren control.

Offline files only: no audio devices, process control or network access. A clear
control bounds leakage in that interval; window correlations never certify
continuous lossless audio or calibrated physical latency.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

import numpy as np
from scipy import signal

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("obs_two_path", Path(__file__).with_name("obs-two-path.py"))
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)

GATES = {
    "source_reference_ncc_min": 0.99,
    "return_ncc_min": 0.95,
    "peak_ratio_min": 1.8,
    "agreement_ms_max": 0.25,
    "control_quiet_rms_max": 1e-6,
    "control_quiet_peak_max": 1e-5,
}


def stereo(values):
    values = np.asarray(values, dtype=np.float64)
    if values.ndim != 2 or values.shape[1] != 2 or not len(values) or not np.isfinite(values).all():
        raise ValueError("finite nonempty stereo PCM required")
    return values


def match(template, candidate, rate):
    if len(candidate) < len(template) or np.dot(template - template.mean(), template - template.mean()) <= 1e-24:
        return None
    curve = BASE.ncc(template, candidate)
    if not np.isfinite(curve).any():
        return None
    index = int(np.nanargmax(np.abs(curve)))
    competitors = np.abs(curve).copy()
    radius = max(1, int(round(rate / 1000)))
    competitors[max(0, index - radius):index + radius + 1] = np.nan
    null = float(np.nanmax(competitors)) if np.isfinite(competitors).any() else 0.0
    correlation = float(curve[index])
    return {"frame": index, "correlation": correlation,
            "competitor_abs": null, "peak_ratio": abs(correlation) / max(null, 1e-12)}


def strong(point, minimum):
    return point is not None and abs(point["correlation"]) >= minimum and point["peak_ratio"] >= GATES["peak_ratio_min"]


def correspondence(template, reference, rate):
    points = [match(template[:, c], reference[:, c], rate) for c in range(2)]
    established = all(strong(p, GATES["source_reference_ncc_min"]) for p in points)
    if established:
        established = abs(points[0]["frame"] - points[1]["frame"]) * 1000 / rate <= GATES["agreement_ms_max"]
    return {"established": bool(established), "channels": points,
            "start_frame": points[0]["frame"] if established else None}


def control_isolation(control, reference, returned, rate):
    control, reference, returned = map(stereo, [control, reference, returned])
    mapping = correspondence(control, reference, rate)
    result = {"established": False, "verdict": "control_reference_unresolved",
              "reference": mapping, "control_start_frame": mapping["start_frame"],
              "scope": "bounded pre-Moiren interval only; not an all-time routing proof"}
    if not mapping["established"]:
        return result
    start = mapping["start_frame"]
    left, right = max(0, start - rate // 4), start + len(control) + rate // 2
    result["checked_recording_frames"] = [left, right]
    if right > len(returned) or right > len(reference):
        result["verdict"] = "control_return_incomplete"
        return result
    section = returned[left:right]
    points = [match(control[:, c], section[:, c], rate) for c in range(2)]
    rms = float(np.sqrt(np.mean(section ** 2)))
    peak = float(np.max(np.abs(section)))
    result.update(return_matches=points, return_rms=rms, return_peak=peak)
    if any(strong(p, GATES["return_ncc_min"]) for p in points):
        result["verdict"] = "correlated_control_leakage"
    elif rms > GATES["control_quiet_rms_max"] or peak > GATES["control_quiet_peak_max"]:
        result["verdict"] = "nonquiet_control_return"
    else:
        result.update(established=True, verdict="bounded_control_clear")
    return result


def relative_window(reference, returned, start, rate, filtered=None):
    reference, returned = map(stereo, [reference, returned])
    result = {"time_s": (start + rate / 4) / rate, "supported": False,
              "offset_frames": None, "raw": [], "highpass": [],
              "physical_qualified": False, "waveform_continuity_qualified": False}
    width = rate // 2
    if start < 0 or start + width > len(reference) or start + width > len(returned):
        result["reason"] = "incomplete_window"
        return result
    left, right = max(0, start - rate // 4), min(len(returned), start + width + rate // 2)
    if filtered is None:
        hp = signal.butter(6, min(3000, rate * .375), "highpass", fs=rate, output="sos")
        filtered = [signal.sosfiltfilt(hp, values, axis=0) for values in [reference, returned]]
    for label, values in [("raw", [reference, returned]), ("highpass", filtered)]:
        for channel in range(2):
            x = values[0][start:start + width, channel]
            point = match(x, values[1][left:right, channel], rate)
            if point:
                actual = left + point["frame"]
                y = values[1][actual:actual + width, channel]
                gain = float(np.dot(x, y) / np.dot(x, x))
                point.update(offset_frames=actual - start, gain=gain,
                             residual_rms=float(np.sqrt(np.mean((y - gain * x) ** 2))))
            result[label].append(point)
    points = result["raw"] + result["highpass"]
    supported = all(strong(p, GATES["return_ncc_min"]) for p in points)
    if supported:
        offsets = [p["offset_frames"] for p in points]
        supported = (max(offsets) - min(offsets)) * 1000 / rate <= GATES["agreement_ms_max"]
    result["supported"] = bool(supported)
    if supported:
        result["offset_frames"] = float(np.mean(offsets))
    else:
        result["reason"] = "weak_ambiguous_or_disagreeing_correspondence"
    return result


def analyze(base, output, ffmpeg, ffprobe):
    run = json.loads((base / "capture-result.json").read_text(encoding="utf-8-sig"))
    recording = Path(run["record_stop"]["outputPath"])
    metadata = json.loads(subprocess.check_output([ffprobe, "-v", "error", "-show_streams", "-of", "json", str(recording)], timeout=20))
    streams = [s for s in metadata["streams"] if s["codec_type"] == "audio"]
    if len(streams) != 2 or any(s["codec_name"] != "pcm_s24le" or s["channels"] != 2 or int(s["sample_rate"]) != 48000 for s in streams):
        raise ValueError("exactly two lossless stereo 48k PCM24 tracks required")
    output.mkdir(exist_ok=False)
    tracks = []
    for channel in range(2):
        path = output / ("track%d.wav" % (channel + 1))
        subprocess.run([ffmpeg, "-nostdin", "-v", "error", "-i", str(recording), "-map", "0:a:%d" % channel, "-c:a", "pcm_f32le", str(path)], check=True, timeout=60)
        rate, values = BASE.normalize_wav(path)
        if rate != 48000:
            raise ValueError("decoded sample rate changed")
        tracks.append(stereo(values))
    reference, returned = tracks
    source_rate, source = BASE.normalize_wav(base / "stimulus-float.wav")
    control_rate, control = BASE.normalize_wav(base / "control-float.wav")
    if source_rate != rate or control_rate != rate:
        raise ValueError("source and recording rates must match; no implicit resampling")
    manifest = json.loads((base / "stimulus.json").read_text())
    noise_start = int(manifest["noise_start_s"] * rate)
    noise_end = int(manifest["noise_end_s"] * rate)
    if noise_end - noise_start < 2 * rate or noise_end > len(source):
        raise ValueError("complete changing source noise interval required")
    isolation = control_isolation(control, reference, returned, rate)
    mapping = correspondence(source[noise_start:noise_start + rate], reference, rate)
    rows = []
    if mapping["established"]:
        hp = signal.butter(6, 3000, "highpass", fs=rate, output="sos")
        filtered = [signal.sosfiltfilt(hp, x, axis=0) for x in tracks]
        first = mapping["start_frame"]
        for start in range(first + rate, first + noise_end - noise_start - rate, rate // 2):
            row = relative_window(reference, returned, start, rate, filtered=filtered)
            row["broadband_reference"] = BASE.broadband_reference(reference[start:start + rate // 2, 0], rate)
            row["noise_eligible"] = row["supported"] and row["broadband_reference"]["supported"]
            rows.append(row)
    accepted = [r for r in rows if r["noise_eligible"]]
    offsets = np.array([r["offset_frames"] * 1000 / rate for r in accepted])
    summary = {"schema": 1, "physical_qualified": False, "waveform_continuity_qualified": False,
               "scope": "OBS Process-reference versus output-capture clock; uncalibrated software offsets only",
               "gates": GATES, "isolation": isolation, "source_reference": mapping,
               "recording": str(recording), "recording_bytes": recording.stat().st_size,
               "recording_sha256": BASE.file_hash(recording), "streams": streams,
               "track_stats": [BASE.audio_stats(x) for x in tracks], "windows": rows,
               "supported_noise_windows": len(accepted), "total_windows": len(rows),
               "diagnostic_offset_ms": {name: float(np.percentile(offsets, q)) for name, q in [("min", 0), ("p50", 50), ("p95", 95), ("p99", 99), ("max", 100)]} if len(offsets) else None,
               "isolated_offset_supported": bool(isolation["established"] and accepted),
               "limits": ["Control covers only its retained pre-Moiren interval and fixed amplitude floor",
                          "No calibration of OBS clock offset or physical DAC/ADC boundary",
                          "Separated windows cannot rule out short holes, drops, repeats, resets or clock drift",
                          "Refused windows are retained; thresholds and transport budgets are not changed"]}
    BASE.write_json(output / "summary.json", summary)
    render(output, tracks, rate, summary)
    return summary


def render(output, tracks, rate, summary):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    fig, axes = plt.subplots(4, 1, figsize=(13, 12), constrained_layout=True)
    fig.suptitle("Process -> Bus -> DAWN | isolated software diagnostic, NOT physical latency", fontweight="bold")
    for label, values, color in [("Process reference", tracks[0], "#276C8E"), ("DAWN return", tracks[1], "#C65D27")]:
        width = rate // 20
        count = len(values) // width
        rms = np.sqrt(np.mean(values[:count * width].reshape(count, width, 2) ** 2, axis=(1, 2)))
        times = (np.arange(count) + .5) * width / rate
        axes[0].plot(times, rms, color=color, label=label)
        axes[1].semilogy(times, np.maximum(rms, 1e-9), color=color, label=label)
    isolation = summary["isolation"]
    interval = isolation.get("checked_recording_frames")
    if interval:
        for ax in axes[:2]:
            ax.axvspan(interval[0] / rate, interval[1] / rate, color="#DDC16D", alpha=.3)
        axes[1].set_xlim(max(0, interval[0] / rate - 1), interval[1] / rate + 1)
    axes[0].set(ylabel="50ms RMS", title="Retained control, noise and sweep; no assumed sample origin")
    axes[1].set(ylabel="50ms RMS (log)", title="No-Moiren control: " + isolation["verdict"])
    axes[0].legend()
    rows = summary["windows"]
    for key, label, color in [("raw", "Raw NCC", "#276C8E"), ("highpass", "3kHz HP NCC", "#C65D27")]:
        axes[2].plot([r["time_s"] for r in rows], [abs(r[key][0]["correlation"]) if r[key] and r[key][0] else np.nan for r in rows], ".-", label=label, color=color)
    axes[2].axhline(GATES["return_ncc_min"], linestyle="--", color="#555555")
    axes[2].set(ylabel="Absolute NCC", title="Refused candidates retained; fixed support gates")
    axes[2].legend()
    accepted = [r for r in rows if r["noise_eligible"]]
    axes[3].plot([r["time_s"] for r in accepted], [r["offset_frames"] * 1000 / rate for r in accepted], "o-", color="#276C8E")
    if not accepted:
        axes[3].text(.5, .5, "No supported offset distribution; correspondence REFUSED", ha="center", transform=axes[3].transAxes, color="#A4262C")
    axes[3].set(ylabel="OBS relative offset (ms)", xlabel="Recording time (s)", title="Not physical latency; no waveform continuity certification")
    for ax in axes:
        ax.grid(alpha=.2)
    fig.savefig(output / "overview.png", dpi=160)
    plt.close(fig)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--ffprobe", default="ffprobe")
    args = parser.parse_args()
    summary = analyze(args.base, args.output, args.ffmpeg, args.ffprobe)
    print(json.dumps({k: v for k, v in summary.items() if k not in ["windows", "streams"]}, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
