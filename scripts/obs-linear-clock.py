#!/usr/bin/env python3
"""Held-out affine linear-interpolation diagnostic for retained Process PCM.

The production Process bridge uses linear interpolation with a bounded, changing
servo. A local affine fit is only a falsifiable approximation, not its actual
cursor history, an epoch association, continuity proof or physical latency.
Original raw-NCC refusals are read without modification.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np
from scipy import optimize

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("obs_isolated", Path(__file__).with_name("obs-isolated.py"))
ISOLATED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ISOLATED)

GATES = {
    "model_correction_ppm_max": 2000,
    "heldout_correlation_min": .99,
    "heldout_nrmse_max": .05,
    "local_nrmse_max": .1,
    "stereo_offset_disagreement_ms_max": .25,
    "stereo_rate_disagreement_ppm_max": 20,
    "coarse_peak_ratio_min": 1.8,
}


def predict_linear(reference, locations):
    if locations[0] < 0 or locations[-1] >= len(reference) - 1:
        return None
    # The sample grid is dense integers. Gather only the two neighbors per
    # position instead of copying a whole strided recording for every residual.
    whole = np.floor(locations).astype(np.intp)
    fraction = locations - whole
    left = reference[whole]
    return left + (reference[whole + 1] - left) * fraction


def fit_channel(reference, returned, start, rate):
    width = rate // 2
    center = start + (width - 1) / 2
    positions = np.arange(width, dtype=np.float64) + start
    target = returned[start:start + width]
    # Outer quarters (including the coarse seed) train; the middle half is
    # held out. Sixteen-frame guards also separate interpolation neighbors.
    local_positions = np.arange(width)
    training = (local_positions < width // 4) | (local_positions >= 3 * width // 4)
    holdout = (local_positions >= width // 4 + 16) & (local_positions < 3 * width // 4 - 16)
    rms = float(np.sqrt(np.mean((target[training] - target[training].mean()) ** 2)))
    if rms <= 1e-10:
        return {"compatible": False, "reason": "silent_return"}
    patch_width = min(512, width // 8)
    patch_start = start + width // 8 - patch_width // 2
    left = max(0, patch_start - rate // 2)
    right = min(len(reference), patch_start + patch_width + rate // 4)
    point = ISOLATED.match(returned[patch_start:patch_start + patch_width], reference[left:right], rate)
    if point is None or point["peak_ratio"] < GATES["coarse_peak_ratio_min"]:
        return {"compatible": False, "reason": "ambiguous_coarse_correspondence", "coarse": point}
    shift = left + point["frame"] - patch_start
    seed_center = patch_start + (patch_width - 1) / 2
    phase_margin = 3 + abs(center - seed_center) * .002
    def predict(parameters):
        locations = positions + parameters[0] + (positions - center) * parameters[1] * .001
        return predict_linear(reference, locations)
    def estimate(predicted):
        x, y = predicted[training], target[training]
        dx, dy = x - x.mean(), y - y.mean()
        denominator = float(np.dot(dx, dx))
        if denominator <= 1e-24:
            return None
        gain = float(np.dot(dx, dy) / denominator)
        return gain, float(y.mean() - gain * x.mean())
    def residual(parameters):
        predicted = predict(parameters)
        fit = estimate(predicted) if predicted is not None else None
        if fit is None:
            return np.full(int(training.sum()), 1e3)
        gain, dc = fit
        return (predicted[training] * gain + dc - target[training]) / rms
    seeds = []
    for phase in np.linspace(shift - 2, shift + 2, 17):
        for correction in np.linspace(-2, 2, 17):
            parameters = np.array([phase + (center - seed_center) * correction * .001, correction])
            errors = residual(parameters)
            seeds.append((float(np.dot(errors, errors)), parameters))
    best = None
    for _, initial in sorted(seeds, key=lambda item: item[0])[:3]:
        result = optimize.least_squares(residual, initial, bounds=([shift - phase_margin, -2], [shift + phase_margin, 2]),
                                        max_nfev=100, ftol=1e-10, xtol=1e-10, gtol=1e-10)
        if best is None or result.cost < best.cost:
            best = result
    predicted = predict(best.x)
    fit = estimate(predicted) if predicted is not None else None
    if fit is None:
        return {"compatible": False, "reason": "model_support_outside_reference"}
    gain, dc = fit
    modeled = predicted * gain + dc
    errors = modeled - target
    heldout_rms = float(np.sqrt(np.mean((target[holdout] - target[holdout].mean()) ** 2)))
    if heldout_rms <= 1e-10 or np.std(modeled[holdout]) <= 1e-10:
        return {"compatible": False, "reason": "silent_heldout_support"}
    correlation = float(np.corrcoef(modeled[holdout], target[holdout])[0, 1])
    nrmse = float(np.sqrt(np.mean(errors[holdout] ** 2)) / heldout_rms)
    local = [float(np.sqrt(np.mean(chunk ** 2)) / rms) for chunk in np.array_split(errors, max(1, width // 64))]
    saturated = abs(best.x[1]) >= 1.999
    supported = bool(best.success and not saturated and correlation >= GATES["heldout_correlation_min"]
                     and nrmse <= GATES["heldout_nrmse_max"] and max(local) <= GATES["local_nrmse_max"])
    return {"compatible": supported, "reason": "heldout_model_compatible" if supported else "model_residual_refused",
            "offset_frames": float(-best.x[0]), "correction_ppm": float(best.x[1] * 1000),
            "gain": gain, "dc": dc, "raw_correlation": point["correlation"],
            "heldout_correlation": correlation, "heldout_nrmse": nrmse, "local_nrmse_max": max(local),
            "rate_bound_saturated": bool(saturated),
            "optimizer_converged": bool(best.success), "training_frames": int(training.sum()),
            "heldout_frames": int(holdout.sum()), "coarse": point}


def fit_linear_window(reference, returned, start, rate):
    reference, returned = map(ISOLATED.stereo, [reference, returned])
    if rate < 8000 or rate > 192000:
        raise ValueError("supported diagnostic rate is8..192kHz")
    result = {"time_s": (start + rate / 4) / rate, "model_compatible": False,
              "physical_qualified": False, "waveform_continuity_qualified": False,
              "relative_offset_frames": None, "channels": []}
    if start < 0 or start + rate // 2 > min(len(reference), len(returned)):
        result["reason"] = "incomplete_window"
        return result
    for channel in range(2):
        result["channels"].append(fit_channel(reference[:, channel], returned[:, channel], start, rate))
    if all(p["compatible"] for p in result["channels"]):
        a, b = result["channels"]
        offset_agrees = abs(a["offset_frames"] - b["offset_frames"]) * 1000 / rate <= GATES["stereo_offset_disagreement_ms_max"]
        rate_agrees = abs(a["correction_ppm"] - b["correction_ppm"]) <= GATES["stereo_rate_disagreement_ppm_max"]
        result["model_compatible"] = bool(offset_agrees and rate_agrees)
        if result["model_compatible"]:
            result["relative_offset_frames"] = (a["offset_frames"] + b["offset_frames"]) / 2
    result["reason"] = "local_affine_linear_model_compatible" if result["model_compatible"] else "unresolved_or_nonaffine_waveform"
    return result


def analyze(base, prior, output):
    original_path = prior / "summary.json"
    original = json.loads(original_path.read_text())
    rate, reference = ISOLATED.BASE.normalize_wav(prior / "track1.wav")
    return_rate, returned = ISOLATED.BASE.normalize_wav(prior / "track2.wav")
    if rate != return_rate:
        raise ValueError("no implicit resampling of recorder tracks")
    rows = []
    for old in original["windows"]:
        # Fit the same retained output-coordinate windows; no window is omitted
        # because it was refused by the raw, equal-rate NCC diagnostic.
        start = int(round(old["time_s"] * rate - rate / 4))
        row = fit_linear_window(reference, returned, start, rate)
        row["original_raw_supported"] = old["supported"]
        rows.append(row)
    result = {"schema": 1, "model": "local_affine_linear_interpolation_v1",
              "scope": "held-out model compatibility of OBS PCM; not actual servo reconstruction or physical latency",
              "physical_qualified": False, "waveform_continuity_qualified": False,
              "isolation_established": original["isolation"]["established"],
              "original_summary_sha256": ISOLATED.BASE.file_hash(original_path),
              "original_recording_sha256": original["recording_sha256"],
              "track_sha256": [ISOLATED.BASE.file_hash(prior / name) for name in ["track1.wav", "track2.wav"]],
              "gates": GATES, "total_windows": len(rows),
              "model_compatible_windows": sum(r["model_compatible"] for r in rows), "windows": rows,
              "limits": ["Original raw/control refusals remain unchanged",
                         "Piecewise32-frame production servo is only locally approximated by one affine rate",
                         "Fitted offset includes uncalibrated OBS timing and cannot compare physical paths",
                         "Epoch association is unknown; unobserved resets can lie inside fitted windows",
                         "No whole-run continuity verdict from separated windows"]}
    output.mkdir(exist_ok=False)
    ISOLATED.BASE.write_json(output / "summary.json", result)
    render(output, rate, result)
    return result


def render(output, rate, summary):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    fig, axes = plt.subplots(3, 1, figsize=(13, 10), constrained_layout=True)
    fig.suptitle("Retained Process PCM | held-out linear clock MODEL, NOT qualification", fontweight="bold")
    rows = summary["windows"]
    times = [r["time_s"] for r in rows]
    for key, label, color in [("raw_correlation", "Raw coarse NCC", "#276C8E"), ("heldout_correlation", "Held-out linear model NCC", "#C65D27")]:
        axes[0].plot(times, [abs(r["channels"][0].get(key, np.nan)) if r["channels"] else np.nan for r in rows], ".-", label=label, color=color)
    axes[0].axhline(GATES["heldout_correlation_min"], linestyle="--", color="#555555")
    axes[0].set(ylabel="Absolute correlation", title="Training/held-out separation; all windows retained")
    axes[0].legend()
    axes[1].plot(times, [r["channels"][0].get("correction_ppm", np.nan) if r["channels"] else np.nan for r in rows], ".-", color="#276C8E")
    axes[1].set(ylabel="Fitted local rate (ppm)", title="Not logged production servo state; changing rate stays visible")
    accepted = [r for r in rows if r["model_compatible"]]
    axes[2].plot([r["time_s"] for r in accepted], [r["relative_offset_frames"] * 1000 / rate for r in accepted], "o", color="#276C8E", label="Model-compatible only")
    axes[2].set(ylabel="OBS relative offset (ms)", xlabel="Recording time (s)", title="No isolation, continuity or physical qualification implied")
    axes[2].legend()
    for ax in axes:
        ax.grid(alpha=.2)
    fig.savefig(output / "overview.png", dpi=160)
    plt.close(fig)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--prior", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = analyze(args.base, args.prior, args.output)
    print(json.dumps({k: v for k, v in result.items() if k != "windows"}, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
