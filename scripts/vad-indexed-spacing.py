#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Test constant-affine compatibility of complete frozen model intervals.

No PCM matching, clock-origin fit, detrending or audio acceptance occurs here.
"""
import argparse
from fractions import Fraction
import hashlib
import json
import math
from pathlib import Path


U64_MAX = (1 << 64) - 1
MODEL = "affine_fir_synthetic_v1_not_exact_cascade"
MAX_INPUT_BYTES = 4 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, name, minimum=0, maximum=U64_MAX):
    require(type(value) is int and minimum <= value <= maximum, f"invalid integer {name}")
    return value


def shape(value, names, name):
    require(type(value) is dict and set(value) == set(names.split()), f"invalid {name} schema")


def load_json(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key {key}")
            result[key] = value
        return result

    def invalid(value):
        raise ValueError(f"invalid JSON constant {value}")

    return json.loads(raw, object_pairs_hook=unique, parse_constant=invalid)


def read_bounded(path, limit):
    with path.open("rb") as source:
        raw = source.read(limit + 1)
    require(len(raw) <= limit, f"input byte cap exceeded: {path}")
    return raw


def source_starts(source):
    shape(source, "schema_version schedule_id waveform_version sample_rate channels marker_frames "
          "marker_amplitude total_source_frames starts", "source")
    require(type(source["schema_version"]) is int and source["schema_version"] == 1,
            "unsupported source schema")
    require(source["waveform_version"] == "moiren-marker-xorshift32-indexed-v2",
            "requires exact indexed-v2 source")
    require(type(source["schedule_id"]) is str and 1 <= len(source["schedule_id"]) <= 256,
            "source schedule identity unavailable")
    require(integer(source["sample_rate"], "source rate") == 48000 and
            integer(source["channels"], "channels") == 2, "requires 48kHz stereo source")
    frames = integer(source["marker_frames"], "marker frames", 128, 4096)
    amplitude = source["marker_amplitude"]
    require(type(amplitude) in (int, float) and 0 < amplitude < 1 and math.isfinite(amplitude),
            "invalid source amplitude")
    total = integer(source["total_source_frames"], "source length", 1, 48000 * 600)
    starts = source["starts"]
    require(type(starts) is list and 3 <= len(starts) <= 64, "requires complete 3..64 code set")
    previous_end = 0
    for start in starts:
        integer(start, "source marker start")
        require(start >= previous_end and start + frames <= total,
                "source markers overlap, regress or exceed schedule")
        previous_end = start + frames
    return starts


def intervals(batch, starts):
    shape(batch, "charged_correlation_samples global_identities max_aggregate_correlation_samples "
          "model_identity_set_complete model_source_unchanged model_window_reports native_execution "
          "packet_quality qualified scope source_origin", "model batch")
    require(batch["qualified"] is False and batch["native_execution"] is False and
            batch["model_source_unchanged"] is True and
            batch["model_identity_set_complete"] is True and
            batch["source_origin"] == "unknown_not_estimated", "unsupported model scope")
    require(type(batch["packet_quality"]) is str and
            batch["packet_quality"].startswith("not_evaluated") and
            len(batch["packet_quality"]) <= 1024 and
            type(batch["scope"]) is str and 0 < len(batch["scope"]) <= 1024,
            "model quality/scope unavailable")
    cap = integer(batch["max_aggregate_correlation_samples"], "aggregate work cap", 1, 7000000000)
    charged = integer(batch["charged_correlation_samples"], "aggregate charge", 1, cap)
    rows, reports = batch["global_identities"], batch["model_window_reports"]
    require(type(rows) is list and type(reports) is list and
            len(rows) == len(reports) == len(starts), "incomplete model set")
    ordered = [None] * len(starts)
    work = 0
    for window, (row, report) in enumerate(zip(rows, reports)):
        shape(row, "recording_frame_interval source_index status window_index", "global identity")
        require(integer(row["window_index"], "window index") == window, "window order changed")
        index = integer(row["source_index"], "source index", 0, len(starts) - 1)
        require(row["status"] == "Unique" and ordered[index] is None,
                "unresolved or duplicated source identity")
        bounds = row["recording_frame_interval"]
        require(type(bounds) is list and len(bounds) == 2, "invalid model coordinate interval")
        lo, hi = (integer(value, "model coordinate") for value in bounds)
        require(lo <= hi <= lo + 2, "unsupported model coordinate interval width/order")
        shape(report, "correlation_samples model packet_quality results source_origin", "window model")
        work += integer(report["correlation_samples"], "per-window charge", 1, 256000000)
        require(report["model"] == MODEL and
                report["packet_quality"] == "not_evaluated_no_complete_set_claim" and
                report["source_origin"] == "unknown_not_estimated", "window model scope changed")
        require(type(report["results"]) is list and len(report["results"]) == 1,
                "requires one immutable model result per window")
        result = report["results"][0]
        shape(result, "best_score recording_frame_interval source_index status", "window result")
        score = result["best_score"]
        require(type(score) in (int, float) and 0.7 <= score <= 1 and math.isfinite(score),
                "invalid accepted model score")
        result_bounds = result["recording_frame_interval"]
        require(type(result_bounds) is list and len(result_bounds) == 2 and
                all(type(value) is int for value in result_bounds), "invalid window coordinates")
        require(type(result["source_index"]) is int and
                result["source_index"] == index and result["status"] == row["status"] and
                result["recording_frame_interval"] == bounds,
                "global and individual model results differ")
        ordered[index] = {"source_index": index, "source_frame": starts[index],
                          "window_index": window, "recording_frame_interval": [lo, hi]}
    require(work == charged, "aggregate work receipt differs from window sum")
    return ordered


def rational(value):
    return [value.numerator, value.denominator]


def analyze(source, batch):
    starts = source_starts(source)
    rows = intervals(batch, starts)
    lower = upper = None
    adjacent = []
    for j, right in enumerate(rows):
        for i in range(j):
            left = rows[i]
            span = starts[j] - starts[i]
            a, b = left["recording_frame_interval"], right["recording_frame_interval"]
            lo, hi = b[0] - a[1], b[1] - a[0]
            pair = {"from_source_index": i, "to_source_index": j}
            low, high = Fraction(lo, span), Fraction(hi, span)
            if lower is None or low > lower[0]:
                lower = (low, {**pair, "fraction": rational(low)})
            if upper is None or high < upper[0]:
                upper = (high, {**pair, "fraction": rational(high)})
            if j == i + 1:
                adjacent.append({**pair, "source_delta_frames": span,
                                 "recording_delta_interval": [lo, hi],
                                 "spacing_residual_interval": [lo - span, hi - span],
                                 "relative_spacing_ppm_interval": [
                                     rational(Fraction((lo - span) * 1000000, span)),
                                     rational(Fraction((hi - span) * 1000000, span))]})
    # For y_i in [lo_i, hi_i], a common intercept exists exactly when
    # max(lo_i - b*x_i) <= min(hi_i - b*x_i). Eliminating that intercept
    # gives every pair's slope interval. Never fit or export the intercept.
    compatible = lower[0] <= upper[0] and upper[0] > 0
    return {"schema_version": 1, "mode": "offline_model_interval_affine_feasibility",
            "qualified": False, "native_execution": False,
            "source_origin": "unknown_not_estimated", "packet_quality": batch["packet_quality"],
            "scope": "model-coordinate compatibility only; not actual SRC/servo history, "
                     "source origin, continuous quality, calibrated clock drift or physical delay",
            "model": MODEL, "source_schedule_id": source["schedule_id"],
            "marker_count": len(rows), "pair_constraints": len(rows) * (len(rows) - 1) // 2,
            "constant_affine_compatible": compatible,
            "reason": "model_intervals_allow_positive_constant_affine_mapping" if compatible
                      else "no_positive_constant_affine_mapping",
            "slope_bounds": {"lower": lower[1], "upper": upper[1]},
            "source_ordered_intervals": rows, "adjacent_spacing": adjacent}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--model-report", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        inputs = [read_bounded(path, MAX_INPUT_BYTES) for path in [args.source, args.model_report]]
        result = analyze(*(load_json(raw) for raw in inputs))
        result["inputs"] = [{"path": str(path), "bytes": len(raw),
                             "sha256": hashlib.sha256(raw).hexdigest()}
                            for path, raw in zip([args.source, args.model_report], inputs)]
        with args.output.open("x", encoding="utf-8", newline="\n") as output:
            output.write(json.dumps(result, indent=2, allow_nan=False) + "\n")
    except (ValueError, OSError, UnicodeError) as error:
        parser.exit(1, f"refused: {error}\n")
    print("model interval diagnostic retained; qualified=false")


if __name__ == "__main__":
    main()
