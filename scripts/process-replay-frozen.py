#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Training-only integer recorder mapping to immutable actual SRC samples.

The first structurally valid, training-compatible window freezes the owner,
integer shift and per-channel gain/DC. Its held-out failure never selects a
different anchor. Later windows cannot search, fit or change that map.
"""

import importlib.util
from pathlib import Path

import numpy as np


SPEC = importlib.util.spec_from_file_location(
    "frozen_ncc", Path(__file__).with_name("obs-two-path.py")
)
NCC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(NCC)
GATES = {
    "coarse_peak_ratio_min": 1.8,
    "training_correlation_min": 0.99,
    "training_nrmse_max": 0.05,
    "heldout_correlation_min": 0.99,
    "heldout_nrmse_max": 0.05,
    "local_nrmse_max": 0.1,
}
MAX_FRAMES = 32 * 1024 * 1024
MAX_SEARCH_POSITIONS = 128 * 1024 * 1024


def require(value, reason):
    if not value:
        raise ValueError(reason)


def stereo(value):
    require(
        isinstance(value, np.ndarray)
        and value.ndim == 2
        and value.shape[1] == 2
        and 0 < len(value) <= MAX_FRAMES
        and np.issubdtype(value.dtype, np.floating),
        "bounded floating stereo required",
    )
    for start in range(0, len(value), 65536):
        block = value[start : start + 65536]
        require(
            np.isfinite(block).all() and (np.abs(block) < 1).all(),
            "nonfinite/clipped PCM",
        )


def metrics(predicted, target):
    a, b = predicted - predicted.mean(), target - target.mean()
    energy = float(np.sqrt(np.mean(b * b)))
    other = float(np.sqrt(np.mean(a * a)))
    if min(energy, other) <= 1e-10:
        return None
    return {
        "correlation": float(np.mean(a * b) / (energy * other)),
        "nrmse": float(np.sqrt(np.mean((predicted - target) ** 2)) / energy),
    }


def layout(width):
    positions = np.arange(width)
    training = (positions < width // 4) | (positions >= 3 * width // 4)
    heldout = (positions >= width // 4 + 16) & (positions < 3 * width // 4 - 16)
    return training, heldout


def supported(owner, start, width, generation=None):
    replay, valid, generations = owner
    if start < 0 or start + width > len(replay):
        return False
    if not np.all(valid[start : start + width] == 1):
        return False
    if generations is not None:
        values = generations[start : start + width]
        if values[0] == 0 or not np.all(values == values[0]):
            return False
        if generation is not None and values[0] != generation:
            return False
    return True


def train_channel(owner, recorded, start, width, channel):
    replay, valid, _ = owner
    patch = recorded[start : start + width // 4, channel]
    if np.std(patch) <= 1e-10 or len(replay) < len(patch):
        return {"compatible": False, "reason": "silent_or_incomplete_training_patch"}
    curve = NCC.ncc(patch, replay[:, channel])
    unknown = np.r_[0, np.cumsum(valid != 1)]
    curve[unknown[len(patch) :] != unknown[: -len(patch)]] = np.nan
    if not np.isfinite(curve).any():
        return {"compatible": False, "reason": "no_valid_training_origin"}
    peak = int(np.nanargmax(np.abs(curve)))
    score = float(abs(curve[peak]))
    competitor = np.abs(curve.copy())
    competitor[max(0, peak - 48) : peak + 49] = np.nan
    second = float(np.nanmax(competitor)) if np.isfinite(competitor).any() else 0.0
    ratio = score / max(second, 1e-12)
    result = {
        "compatible": False,
        "coarse_frame": peak,
        "coarse_abs": score,
        "coarse_peak_ratio": ratio,
        "reason": "ambiguous_training_origin",
    }
    if ratio < GATES["coarse_peak_ratio_min"]:
        return result
    training, _ = layout(width)
    target = np.asarray(recorded[start : start + width, channel], dtype=np.float64)
    candidates = []
    # Compare adjacent integer lags using training only; the coarse exclusion
    # neighborhood by itself does not establish a unique integer alignment.
    for location in range(peak - 2, peak + 3):
        if not supported(owner, location, width):
            continue
        values = np.asarray(
            replay[location : location + width, channel], dtype=np.float64
        )
        x, y = values[training], target[training]
        dx, dy = x - x.mean(), y - y.mean()
        energy = float(np.dot(dx, dx))
        if energy <= 1e-24:
            continue
        gain = float(np.dot(dx, dy) / energy)
        dc = float(y.mean() - gain * x.mean())
        measured = metrics(x * gain + dc, y)
        if measured is not None:
            candidates.append(
                {"shift": location - start, "gain": gain, "dc": dc, **measured}
            )
    accepted = [
        c
        for c in candidates
        if c["correlation"] >= GATES["training_correlation_min"]
        and c["nrmse"] <= GATES["training_nrmse_max"]
    ]
    result["integer_training_candidates"] = candidates
    if len(accepted) == 1:
        result.update(compatible=True, reason="unique_training_integer", **accepted[0])
    return result


def evaluate(owner, recorded, window, anchor):
    start, width = window["start_frame"], window["frames"]
    location = start + anchor["recorder_to_replay_shift_frames"]
    row = {
        "association_compatible": False,
        "reason": "unknown_outside_or_generation_support",
        "replay_start_frame": location,
        "channels": [],
    }
    if not supported(owner, location, width, anchor["generation"]):
        return row
    training, heldout = layout(width)
    for channel, fit in enumerate(anchor["channels"]):
        values = np.asarray(
            owner[0][location : location + width, channel], dtype=np.float64
        )
        target = np.asarray(recorded[start : start + width, channel], dtype=np.float64)
        prediction = values * fit["gain"] + fit["dc"]
        measured = metrics(prediction[heldout], target[heldout])
        rms = float(np.std(target[training]))
        if measured is None or rms <= 1e-10:
            row["channels"].append(
                {"compatible": False, "reason": "uninformative_signal"}
            )
            continue
        errors = prediction - target
        local = max(
            float(np.sqrt(np.mean(block * block)) / rms)
            for block in np.array_split(errors, max(1, width // 64))
        )
        compatible = (
            measured["correlation"] >= GATES["heldout_correlation_min"]
            and measured["nrmse"] <= GATES["heldout_nrmse_max"]
            and local <= GATES["local_nrmse_max"]
        )
        row["channels"].append(
            {"compatible": bool(compatible), **measured, "local_nrmse_max": local}
        )
    row["association_compatible"] = all(c["compatible"] for c in row["channels"])
    row["reason"] = (
        "frozen_integer_heldout_compatible"
        if row["association_compatible"]
        else "waveform_residual_or_silence"
    )
    return row


def associate(owners, recorded, windows, rate=48000):
    require(type(rate) is int and rate == 48000, "exact rate48000 required")
    stereo(recorded)
    require(
        type(owners) is dict and 1 <= len(owners) <= 8, "one to eight owners required"
    )
    checked, total = {}, recorded.nbytes
    for name, value in owners.items():
        require(
            type(name) is str
            and 0 < len(name) <= 256
            and type(value) is tuple
            and len(value) in (2, 3),
            "owner tuple required",
        )
        replay, valid = value[:2]
        generation = value[2] if len(value) == 3 else None
        stereo(replay)
        require(
            isinstance(valid, np.ndarray)
            and valid.shape == (len(replay),)
            and np.isin(valid, [0, 1]).all(),
            "binary validity length mismatch",
        )
        if generation is not None:
            require(
                isinstance(generation, np.ndarray)
                and generation.shape == valid.shape
                and np.issubdtype(generation.dtype, np.integer)
                and np.all(generation >= 0)
                and np.all(generation[valid == 1] > 0),
                "generation array invalid",
            )
        checked[name] = (replay, valid, generation)
        total += (
            replay.nbytes
            + valid.nbytes
            + (generation.nbytes if generation is not None else 0)
        )
    require(total <= 512 * 1024 * 1024, "combined array byte cap")
    require(
        type(windows) is list and 1 <= len(windows) <= 64, "one to64 windows required"
    )
    end = 0
    for window in windows:
        require(
            type(window) is dict and set(window) == {"start_frame", "frames"},
            "unlabelled window required",
        )
        start, width = window["start_frame"], window["frames"]
        require(
            type(start) is int
            and type(width) is int
            and 256 <= width <= 8192
            and start >= end
            and start + width <= len(recorded),
            "window bounds/order",
        )
        end = start + width
    require(
        sum(len(v[0]) for v in checked.values()) * len(windows) * 2
        <= MAX_SEARCH_POSITIONS,
        "aggregate search position cap",
    )
    report = {
        "qualified": False,
        "identity_accepted": False,
        "physical_latency_ms": None,
        "waveform_continuity_qualified": False,
        "anchor": None,
        "training_attempts": 0,
        "windows": [],
        "gates": dict(GATES),
        "scope": "frozen integer recorder mapping only; actual production SRC fixed",
    }
    for ordinal, window in enumerate(windows):
        attempted = []
        if report["anchor"] is None:
            report["training_attempts"] += 1
            candidates = []
            for name, owner in checked.items():
                channels = [
                    train_channel(
                        owner, recorded, window["start_frame"], window["frames"], c
                    )
                    for c in range(2)
                ]
                attempted.append({"owner": name, "channels": channels})
                if (
                    all(c["compatible"] for c in channels)
                    and channels[0]["shift"] == channels[1]["shift"]
                ):
                    candidates.append((name, channels))
            if len(candidates) == 1:
                name, channels = candidates[0]
                # An internally ambiguous owner remains a competing origin.
                # Reject conservatively using every owner's strongest coarse
                # peak, even when that owner's later training test refused.
                global_unique = all(
                    channels[c]["coarse_abs"]
                    / max(
                        max(
                            (
                                item["channels"][c].get("coarse_abs", 0.0)
                                for item in attempted
                                if item["owner"] != name
                            ),
                            default=0.0,
                        ),
                        1e-12,
                    )
                    >= GATES["coarse_peak_ratio_min"]
                    for c in range(2)
                )
                if global_unique:
                    location = window["start_frame"] + channels[0]["shift"]
                    generation = checked[name][2]
                    report["anchor"] = {
                        "window_index": ordinal,
                        "owner": name,
                        "recorder_to_replay_shift_frames": channels[0]["shift"],
                        "channels": [
                            {"gain": c["gain"], "dc": c["dc"]} for c in channels
                        ],
                        "generation": int(generation[location])
                        if generation is not None
                        else None,
                    }
        anchor = report["anchor"]
        row = (
            evaluate(checked[anchor["owner"]], recorded, window, anchor)
            if anchor
            else {
                "association_compatible": False,
                "reason": "no_unique_training_anchor",
                "channels": [],
            }
        )
        row.update(
            window_index=ordinal, window=dict(window), training_candidates=attempted
        )
        report["windows"].append(row)
    return report
