#!/usr/bin/env python3
"""OBS reference/shared-output path separation, not physical qualification.

Requires NumPy and SciPy; matplotlib is optional until rendering a report.
Run on retained files only. This script opens no audio clients or devices.
"""
import argparse
import hashlib
import html
import json
from pathlib import Path
import subprocess
import sys

import numpy as np
from scipy import signal
from scipy.io import wavfile


GATES = {
    "direct_correlation_min": 0.08,
    "delayed_correlation_min": 0.06,
    "peak_ratio_min": 1.8,
    "stereo_disagreement_ms_max": 0.25,
    "raw_highpass_disagreement_ms_max": 0.35,
    "adjacent_delay_jump_ms": 0.35,
    "broadband_entropy_min": 0.8,
    "broadband_top16_power_max": 0.1,
    "peak_exclusion_ms": 1.0,
}


def ncc(template, candidate):
    """Signed Pearson correlation; insufficient local energy stays unknown.

    Long-double window sums plus a cancellation-error bound prevent the FFT
    numerator's roundoff from inventing peaks in a silent tail. Do not divide
    by an arbitrary tiny floor and then call the result normalized.
    """
    x = np.asarray(template, dtype=np.float64)
    y = np.asarray(candidate, dtype=np.float64)
    if x.ndim != 1 or y.ndim != 1 or len(x) < 3 or len(y) < len(x):
        raise ValueError("correlation needs one-dimensional complete windows")
    if not np.isfinite(x).all() or not np.isfinite(y).all():
        raise ValueError("nonfinite correlation input")
    x = x - x.mean()
    energy = float(np.dot(x, x))
    if energy <= 1e-24:
        raise ValueError("silent reference")
    n = len(x)
    wide = y.astype(np.longdouble)
    sums = np.r_[np.longdouble(0), np.cumsum(wide)]
    squares = np.r_[np.longdouble(0), np.cumsum(wide * wide)]
    sy = sums[n:] - sums[:-n]
    ey = squares[n:] - squares[:-n] - sy * sy / n
    bound = float(64 * np.finfo(np.longdouble).eps * max(float(squares[-1]), 1e-30))
    valid = ey > max(bound, energy * 1e-14, 1e-24)
    numerator = signal.correlate(y, x, mode="valid", method="fft")
    result = np.full(len(numerator), np.nan)
    result[valid] = numerator[valid] / np.sqrt(np.asarray(ey[valid], dtype=np.float64) * energy)
    # Large violations are unknown, not clipped into a strong accepted peak.
    result[np.abs(result) > 1 + 1e-8] = np.nan
    return np.clip(result, -1, 1)


def peak(curve, lags, low, high, exclude=None):
    mask = np.isfinite(curve) & (lags >= low) & (lags <= high)
    if exclude is not None:
        mask &= np.abs(lags - exclude) > GATES["peak_exclusion_ms"]
    indices = np.flatnonzero(mask)
    if not len(indices):
        return None
    index = int(indices[np.argmax(np.abs(curve[indices]))])
    fraction = 0.0
    if 0 < index < len(curve) - 1 and np.isfinite(curve[index - 1:index + 2]).all():
        a, b, c = np.abs(curve[index - 1:index + 2])
        denominator = a - 2 * b + c
        if abs(denominator) > 1e-15:
            fraction = float(np.clip(0.5 * (a - c) / denominator, -0.5, 0.5))
    lag = float(np.interp(index + fraction, np.arange(len(lags)), lags))
    competitors = mask & (np.abs(lags - lag) > GATES["peak_exclusion_ms"])
    null = float(np.max(np.abs(curve[competitors]))) if competitors.any() else 0.0
    value = float(curve[index])
    return {"lag_ms": lag, "correlation": value, "abs_correlation": abs(value),
            "null_peak": null, "peak_ratio": abs(value) / max(null, 1e-12)}


def correlate_window(reference, output, start, rate):
    left = max(0, start - int(np.ceil(0.160 * rate)))
    right = min(len(output), start + int(np.ceil(0.500 * rate)) + len(reference))
    curve = ncc(reference, output[left:right])
    lags = (left - start + np.arange(len(curve))) / rate * 1000
    first = peak(curve, lags, -120, 500)
    second = peak(curve, lags, -120, 500, first["lag_ms"] if first else None)
    if not first or not second:
        return curve, lags, first, second
    direct, returned = sorted([first, second], key=lambda point: point["lag_ms"])
    # OBS may timestamp the direct capture before the reference track. Search
    # both paths together rather than assume returned lag >= +20 ms or that
    # the direct path is stronger; either assumption would hide low delays.
    if direct["lag_ms"] > 45:
        return curve, lags, None, returned
    competitors = np.isfinite(curve) & (lags >= -120) & (lags <= 500)
    for point in [direct, returned]:
        competitors &= np.abs(lags - point["lag_ms"]) > GATES["peak_exclusion_ms"]
    null = float(np.max(np.abs(curve[competitors]))) if competitors.any() else 0.0
    for point in [direct, returned]:
        point["null_peak"] = null
        point["peak_ratio"] = point["abs_correlation"] / max(null, 1e-12)
    return curve, lags, direct, returned


def gate(raw, highpass):
    reasons, means, spreads = [], [], []
    for name, channels in [("raw", raw), ("highpass", highpass)]:
        separations = []
        if len(channels) != 2:
            reasons.append(name + "_stereo_missing")
        for channel in channels:
            direct, returned = channel.get("direct"), channel.get("delayed")
            if not direct or not returned:
                reasons.append(name + "_peak_missing")
                continue
            separation = returned["lag_ms"] - direct["lag_ms"]
            if not np.isfinite(direct["lag_ms"]) or not np.isfinite(returned["lag_ms"]) or separation <= 0:
                reasons.append(name + "_invalid_path_order")
                continue
            for label, point, minimum in [
                ("direct", direct, GATES["direct_correlation_min"]),
                ("delayed", returned, GATES["delayed_correlation_min"]),
            ]:
                if not np.isfinite(point["abs_correlation"]) or not minimum <= point["abs_correlation"] <= 1:
                    reasons.append(name + "_" + label + "_weak")
                if not np.isfinite(point["peak_ratio"]) or point["peak_ratio"] < GATES["peak_ratio_min"]:
                    reasons.append(name + "_" + label + "_ambiguous")
            separations.append(separation)
        mean = float(np.mean(separations)) if len(separations) == 2 else None
        spread = abs(separations[0] - separations[1]) if len(separations) == 2 else None
        if spread is not None and spread > GATES["stereo_disagreement_ms_max"]:
            reasons.append(name + "_stereo_disagreement")
        means.append(mean)
        spreads.append(spread)
    if all(value is not None for value in means):
        if abs(means[0] - means[1]) > GATES["raw_highpass_disagreement_ms_max"]:
            reasons.append("raw_highpass_disagreement")
    return {"confidence_pass": not reasons, "reasons": sorted(set(reasons)),
            "separation_ms": means[0], "highpass_separation_ms": means[1],
            "stereo_disagreement_ms": spreads[0], "highpass_stereo_disagreement_ms": spreads[1]}


def delay_jumps(values, maximum):
    return [False] + [a is not None and b is not None and abs(b - a) > maximum
                      for a, b in zip(values, values[1:])] if values else []


def transfer(x, y, rate, nperseg):
    """H1 estimate Pxy/Pxx with coherence and explicit unsupported bins.

    scipy.signal.csd uses conj(FFT(x))*FFT(y). Divide by REAL input power,
    never np.maximum on a complex FFT denominator.
    """
    if len(x) != len(y) or len(x) < nperseg:
        raise ValueError("transfer requires equal complete segments")
    kwargs = dict(fs=rate, nperseg=nperseg, noverlap=nperseg * 3 // 4)
    frequency, pxx = signal.welch(x, **kwargs)
    _, pyy = signal.welch(y, **kwargs)
    _, pxy = signal.csd(x, y, **kwargs)
    valid = (pxx > max(float(np.max(pxx)) * 1e-8, 1e-24)) & (pyy > 1e-24)
    response = np.full(len(pxx), np.nan + 1j * np.nan)
    coherence = np.full(len(pxx), np.nan)
    response[valid] = pxy[valid] / pxx[valid]
    coherence[valid] = np.clip(np.abs(pxy[valid]) ** 2 / (pxx[valid] * pyy[valid]), 0, 1)
    return {"frequency": frequency, "response": response, "coherence": coherence, "valid": valid}


def broadband_reference(values, rate):
    """Reject silence/sweeps for NOISE statistics without deleting candidates."""
    measures = []
    # A sweep can cover the whole band over 500 ms; each 125 ms subwindow
    # must also be noise-like. Otherwise retain it as a non-noise candidate.
    for chunk in np.array_split(values, 4):
        frequency, power = signal.welch(chunk, fs=rate, nperseg=min(4096, len(chunk)))
        power = power[(frequency >= 500) & (frequency <= 18000)]
        total = float(power.sum())
        if total <= 1e-24:
            return {"supported": False, "entropy_min": None, "top16_power_max": None}
        probability = power / total
        entropy = float(-np.sum(probability * np.log(np.maximum(probability, 1e-300))) / np.log(len(power)))
        measures.append((entropy, float(np.sort(probability)[-16:].sum())))
    entropy = min(item[0] for item in measures)
    concentrated = max(item[1] for item in measures)
    return {"supported": entropy >= GATES["broadband_entropy_min"]
            and concentrated <= GATES["broadband_top16_power_max"],
            "entropy_min": entropy, "top16_power_max": concentrated}


def file_hash(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest().upper()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False), encoding="utf-8")


def audio_stats(data):
    finite = np.isfinite(data)
    return {"frames": len(data), "nonfinite_samples": int((~finite).sum()),
            "rms": float(np.sqrt(np.mean(data * data))) if finite.all() and len(data) else None,
            "peak": float(np.max(np.abs(data))) if finite.all() and len(data) else None,
            "clipped_samples": int((np.abs(data) >= 0.9999).sum()),
            "near_zero_frames": int((np.max(np.abs(data), axis=1) < 1e-7).sum())}


def normalize_wav(path):
    rate, values = wavfile.read(path, mmap=True)
    if values.ndim != 2 or values.shape[1] != 2:
        raise ValueError("expected stereo tracks")
    if np.issubdtype(values.dtype, np.integer):
        values = values.astype(np.float64) / (2 ** (values.dtype.itemsize * 8 - 1))
    return int(rate), np.asarray(values, dtype=np.float64)


def render(folder, reference, output, rate, rows, selected, summary):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    plt.rcParams.update({"font.size": 10, "axes.spines.top": False,
                         "axes.spines.right": False, "axes.grid": True, "grid.alpha": 0.2})
    figure, axes = plt.subplots(3, 2, figsize=(14, 12), layout="constrained")
    figure.suptitle("OBS two-path diagnostic | not physical latency or VAD qualification", fontsize=16)
    width = rate // 10
    count = min(len(reference), len(output)) // width
    times = (np.arange(count) + 0.5) * width / rate
    for values, color, label in [(reference, "#0072b2", "Python reference"),
                                 (output, "#d55e00", "Shared output")]:
        rms = np.sqrt(np.mean(values[:count * width].reshape(count, width, 2) ** 2, axis=(1, 2)))
        axes[0, 0].plot(times, rms, color=color, label=label)
    axes[0, 0].set(title="A. Recorded level; no assumed source origin", xlabel="Record time (s)", ylabel="RMS")
    axes[0, 0].legend()
    accepted = [row for row in rows if row["noise_eligible"]]
    rejected = [row for row in rows if not row["confidence_pass"] and row["separation_ms"] is not None]
    axes[0, 1].scatter([row["time_s"] for row in rejected], [row["separation_ms"] for row in rejected],
                       marker="x", color="#999999", label="Rejected candidates")
    axes[0, 1].scatter([row["time_s"] for row in accepted], [row["separation_ms"] for row in accepted],
                       color="#0072b2", s=12, label="Strict broadband pass")
    non_noise = [row for row in rows if row["confidence_pass"] and not row["noise_eligible"]]
    axes[0, 1].scatter([row["time_s"] for row in non_noise], [row["separation_ms"] for row in non_noise],
                       color="#cc79a7", s=12, label="Non-broadband candidate")
    axes[0, 1].set(title="B. Relative path separation; rejected windows retained", xlabel="Record time (s)", ylabel="Separation (ms)")
    axes[0, 1].legend()
    if selected:
        start = int(round((selected["time_s"] - 0.25) * rate))
        curve, lags, direct, returned = correlate_window(reference[start:start + rate // 2, 0], output[:, 0], start, rate)
        axes[1, 0].plot(lags, curve, color="#0072b2", linewidth=0.7)
        for point, label in [(direct, "Direct"), (returned, "Returned")]:
            axes[1, 0].axvline(point["lag_ms"], linestyle="--", color="#d55e00", label=label)
        axes[1, 0].set(title="C. Selected signed normalized correlation", xlabel="Output lag (ms)", ylabel="NCC", ylim=(-1, 1))
        axes[1, 0].legend()
        # A local averaged spectrum may lose coherence when the return delay
        # moves. Unsupported bins stay masked and never become comb evidence.
        left = max(0, start - rate // 2)
        length = min(rate * 2, len(reference) - left)
        offset = int(round(direct["lag_ms"] * rate / 1000))
        if left + offset >= 0 and left + offset + length <= len(output):
            spec = transfer(reference[left:left + length, 0], output[left + offset:left + offset + length, 0], rate, 32768)
            f, coherence = spec["frequency"], spec["coherence"]
            supported = spec["valid"] & (coherence >= 0.8)
            magnitude = 20 * np.log10(np.maximum(np.abs(spec["response"]), 1e-15))
            magnitude[~supported] = np.nan
            axes[1, 1].plot(f, magnitude, color="#0072b2", linewidth=1)
            axes[1, 1].set(title="D. H1 spectrum; coherence >= 0.8 only", xlabel="Frequency (Hz)", ylabel="Magnitude (dB)", xlim=(3000, 3250))
            axes[2, 0].plot(f, coherence, color="#009e73", linewidth=0.7)
            axes[2, 0].axhline(0.8, color="#d55e00", linestyle="--")
            axes[2, 0].set(title="E. Spectral support (same zoom as D)", xlabel="Frequency (Hz)", ylabel="Coherence", xlim=(3000, 3250), ylim=(0, 1.02))
            band = (f >= 500) & (f <= 18000)
            summary["spectrum"] = {"method": "H1 Pxy/Pxx", "nperseg": 32768,
                                   "band_hz": [500, 18000],
                                   "supported_fraction": float(np.mean(supported[band])),
                                   "independent_comb_delay_fit": False}
    else:
        for axis in [axes[1, 0], axes[1, 1], axes[2, 0]]:
            axis.text(0.1, 0.5, "No strict-confidence window; no spectral conclusion", transform=axis.transAxes)
    axes[2, 1].axis("off")
    distribution = summary["relative_path_ms"]
    lines = ["F. Evidence limits", "", f"Broadband windows: {len(accepted)} / {len(rows)} strict confidence",
             "Relative p50: " + (f"{distribution['p50']:.3f} ms" if distribution else "unresolved"),
             f"Noise adjacent jumps: {sum(row['delay_jump'] for row in rows)}",
             f"All candidate jumps: {sum(row['candidate_delay_jump'] for row in rows)}",
             "", "Shared-output loopback is not DAC/ADC calibration.",
             "This recording does not exercise VAD MIC.",
             "Confidence is not continuous-waveform correctness.",
             "Source write underflow status was not retained.",
             "No missing marker is relabelled as proven frame loss."]
    axes[2, 1].text(0.02, 0.96, "\n".join(lines), va="top", fontsize=11, linespacing=1.6)
    figure.savefig(folder / "overview.png", dpi=140)
    figure.savefig(folder / "overview.svg")
    plt.close(figure)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mkv", type=Path, required=True)
    parser.add_argument("--ffmpeg", type=Path, required=True)
    parser.add_argument("--ffprobe", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--recording-sha256", required=True)
    args = parser.parse_args()
    if args.out.exists():
        raise ValueError("preserve existing analysis output")
    actual_hash = file_hash(args.mkv)
    if actual_hash != args.recording_sha256.upper():
        raise ValueError("recording hash mismatch")
    metadata = json.loads(subprocess.check_output([str(args.ffprobe), "-v", "error", "-show_streams", "-of", "json", str(args.mkv)], text=True))
    streams = [item for item in metadata["streams"] if item["codec_type"] == "audio"]
    if len(streams) != 2 or any(item["codec_name"] != "pcm_s24le" or item["channels"] != 2 or int(item["sample_rate"]) != 48000 for item in streams):
        raise ValueError("expected exactly two lossless 48 kHz stereo PCM24 tracks")
    args.out.mkdir(parents=True)
    data = []
    for index in range(2):
        path = args.out / ("track%d.wav" % (index + 1))
        subprocess.run([str(args.ffmpeg), "-nostdin", "-v", "error", "-i", str(args.mkv),
                        "-map", "0:a:%d" % index, "-c:a", "pcm_f32le", str(path)], check=True, timeout=120)
        rate, values = normalize_wav(path)
        if rate != 48000:
            raise ValueError("unexpected decoded rate")
        data.append(values)
    reference, output = data
    length = min(map(len, data))
    if not all(np.isfinite(value).all() for value in data):
        raise ValueError("nonfinite recording; no latency inference")
    highpass = signal.butter(6, 3000, "highpass", fs=rate, output="sos")
    filtered = [signal.sosfiltfilt(highpass, value, axis=0) for value in data]
    rows = []
    for start in range(rate, length - rate, rate // 2):
        row = {"time_s": (start + rate / 4) / rate, "raw": [], "highpass": []}
        for name, values in [("raw", data), ("highpass", filtered)]:
            for channel in range(2):
                try:
                    _, _, direct, returned = correlate_window(values[0][start:start + rate // 2, channel], values[1][:, channel], start, rate)
                except ValueError:
                    direct = returned = None
                row[name].append({"direct": direct, "delayed": returned})
        row.update(gate(row["raw"], row["highpass"]))
        row["broadband"] = [broadband_reference(reference[start:start + rate // 2, channel], rate)
                            for channel in range(2)]
        row["noise_eligible"] = row["confidence_pass"] and all(item["supported"] for item in row["broadband"])
        rows.append(row)
    jumps = delay_jumps([row["separation_ms"] if row["noise_eligible"] else None for row in rows], GATES["adjacent_delay_jump_ms"])
    candidate_jumps = delay_jumps([row["separation_ms"] if row["confidence_pass"] else None for row in rows], GATES["adjacent_delay_jump_ms"])
    for row, jump, candidate_jump in zip(rows, jumps, candidate_jumps):
        row["delay_jump"] = bool(jump)
        row["candidate_delay_jump"] = bool(candidate_jump)
    accepted = [row for row in rows if row["noise_eligible"]]
    values = [row["separation_ms"] for row in accepted]
    distribution = dict(zip(["p50", "p95", "p99", "max", "min"],
                            [float(np.percentile(values, 50)), float(np.percentile(values, 95)),
                             float(np.percentile(values, 99)), max(values), min(values)])) if values else None
    selected = max(accepted, key=lambda row: min(channel["delayed"]["abs_correlation"] for channel in row["raw"])) if accepted else None
    summary = {"schema_version": 1, "status": "diagnostic_only_quality_unqualified",
               "physical_latency_qualified": False, "vad_mic_exercised": False,
               "recording": str(args.mkv), "recording_sha256": actual_hash,
               "analyzer_sha256": file_hash(Path(__file__)), "python": sys.version,
               "gates": GATES, "rate": rate, "frames_per_track": list(map(len, data)),
               "windows_total": len(rows), "windows_confidence_pass": sum(row["confidence_pass"] for row in rows),
               "windows_broadband_pass": len(accepted),
               "relative_path_ms": distribution, "selected": selected,
               "implied_comb_spacing_hz_not_spectral_fit": 1000 / selected["separation_ms"] if selected else None,
               "tracks": [audio_stats(value) for value in data],
               "limits": ["Shared-output relative delay, not calibrated physical latency",
                          "Broad search -120..500 ms; two paths <=1 ms apart remain unresolved",
                          "No source-to-record origin assumed from an RMS threshold",
                          "No whole-waveform quality pass follows from accepted windows",
                          "Source blocking-write underflow result was not retained",
                          "No native VAD direction is exercised by this Process-only recording"]}
    render(args.out, reference, output, rate, rows, selected, summary)
    write_json(args.out / "windows.json", rows)
    write_json(args.out / "summary.json", summary)
    document = "<html><meta charset='utf-8'><title>OBS relative-path diagnostic</title><h1>Unqualified two-path diagnostic</h1><img style='max-width:100%' src='overview.svg'><pre>" + html.escape(json.dumps(summary, indent=2)) + "</pre></html>"
    (args.out / "report.html").write_text(document, encoding="utf-8")
    print(json.dumps({key: value for key, value in summary.items() if key != "selected"}, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
