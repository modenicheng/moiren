#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Training-only integer recorder mapping to immutable actual SRC samples.

The first structurally valid, training-compatible window freezes the owner,
integer shift and per-channel gain/DC. Its held-out failure never selects a
different anchor. Later windows cannot search, fit or change that map.
"""

import importlib.util
from copy import deepcopy
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
MAX_AUDIT_RUNS = 4096


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


def audit_recording(owners, recorded, association_report, rate=48000):
    """Account for every recorder frame using only the supplied frozen map.

    No association, search, lag adjustment, resampling, interpolation or gain/DC
    fitting occurs here. Nonfinite samples are evidence categories, not dropped
    rows. Report validity is structural; it cannot authenticate its provenance.
    """
    anchor, training = _audit_inputs(owners, recorded, association_report, rate)
    replay, valid, generations = owners[anchor["owner"]]
    shift, generation = anchor["recorder_to_replay_shift_frames"], anchor["generation"]
    gain = np.array([c["gain"] for c in anchor["channels"]], dtype=np.float64)
    dc = np.array([c["dc"] for c in anchor["channels"]], dtype=np.float64)
    categories = (
        "fixed_map_compatible", "outside_replay", "unknown_replay",
        "generation_mismatch", "nonfinite_recording", "nonfinite_replay",
        "clipped_recording", "clipped_replay", "uninformative_signal",
        "anchor_training", "partial_block_tail", "waveform_residual",
        "unsupported_block_context",
    )
    counts = dict.fromkeys(categories, 0)
    runs = []
    encoding_match = replay.dtype.str == recorded.dtype.str
    raw = dict(sample_encoding_match=encoding_match, support_frames=0,
               equal_stereo_frames=0, unequal_stereo_frames=0, incomparable_frames=0)
    raw["nontraining"] = dict(support_frames=0, equal_stereo_frames=0,
                              unequal_stereo_frames=0, incomparable_frames=0)
    sums, squares, maxima = np.zeros((2, 2)), np.zeros((2, 2)), np.zeros((2, 2))
    error_frames = [0, 0]
    blocks = dict(frames=64, complete=0, informative_pass=0, informative_fail=0,
                  uninformative=0, unsupported=0, training_overlap=0, partial=0)
    for start in range(0, len(recorded), 65536):
        stop = min(start + 65536, len(recorded))
        width = stop - start
        target_raw = recorded[start:stop]
        target = np.asarray(target_raw, dtype=np.float64)
        source = np.zeros((width, 2), dtype=np.float64)
        known, inside = np.zeros(width, dtype=bool), np.zeros(width, dtype=bool)
        codes = np.full(width, 1, dtype=np.uint8)
        is_training = np.zeros(width, dtype=bool)
        for first, last in training:
            is_training[max(0, first - start):max(0, min(width, last - start))] = True
        lo, hi = max(start, -shift), min(stop, len(replay) - shift)
        if lo < hi:
            local = slice(lo - start, hi - start)
            native = slice(lo + shift, hi + shift)
            inside[local] = True
            source[local] = replay[native]
            supported_frames = valid[native] == 1
            matching = generations[native] == generation
            known[local] = supported_frames & matching
            codes[local] = np.where(~supported_frames, 2, np.where(~matching, 3, 0))
            if encoding_match:
                # Never cast to float32 or compare numeric zeros: signed-zero
                # and other representation differences must survive this audit.
                a = np.ascontiguousarray(replay[native]).view(np.uint8).reshape(hi-lo, -1)
                b = np.ascontiguousarray(target_raw[local]).view(np.uint8).reshape(hi-lo, -1)
                equal = np.all(a == b, axis=1)
            for mask, counter in ((known[local], raw),
                                  (known[local] & ~is_training[local], raw["nontraining"])):
                n = int(np.count_nonzero(mask))
                counter["support_frames"] += n
                if encoding_match:
                    same = int(np.count_nonzero(equal & mask))
                    counter["equal_stereo_frames"] += same
                    counter["unequal_stereo_frames"] += n - same
                else:
                    counter["incomparable_frames"] += n
        finite_source = np.isfinite(source).all(axis=1)
        finite_target = np.isfinite(target).all(axis=1)
        codes[(codes == 0) & ~finite_source] = 5
        codes[~finite_target] = 4
        codes[(codes == 0) & (np.abs(source) >= 1).any(axis=1)] = 7
        codes[(codes == 0) & (np.abs(target) >= 1).any(axis=1)] = 6
        with np.errstate(invalid="ignore", over="ignore"):
            prediction = source * gain + dc
            errors = np.abs(prediction - target)
        good = codes == 0
        for group, mask in enumerate((good, good & ~is_training)):
            values = errors[mask]
            if len(values):
                error_frames[group] += len(values)
                sums[group] += np.sum(values, axis=0)
                squares[group] += np.sum(values * values, axis=0)
                maxima[group] = np.maximum(maxima[group], np.max(values, axis=0))
        for offset in range(0, width, 64):
            end = min(offset + 64, width)
            part = codes[offset:end]
            eligible = part == 0
            if end-offset < 64:
                blocks["partial"] += 1
            else:
                blocks["complete"] += 1
            if not np.all(eligible):
                part[eligible] = 12
                blocks["unsupported"] += 1
                continue
            measured = [metrics(prediction[offset:end, c], target[offset:end, c])
                        for c in range(2)]
            if any(value is None for value in measured):
                part[:] = 8
                blocks["uninformative"] += 1
            elif end-offset < 64:
                part[:] = 10
            else:
                passes = all(m["correlation"] >= GATES["heldout_correlation_min"]
                             and m["nrmse"] <= GATES["heldout_nrmse_max"]
                             and m["nrmse"] <= GATES["local_nrmse_max"] for m in measured)
                part[:] = 0 if passes else 11
                trained = is_training[offset:end]
                if np.any(trained):
                    blocks["training_overlap"] += 1
                    part[:] = 12
                    part[trained] = 9
                else:
                    blocks["informative_pass" if passes else "informative_fail"] += 1
        boundaries = np.r_[0, np.flatnonzero(codes[1:] != codes[:-1]) + 1, width]
        for first, last in zip(boundaries[:-1], boundaries[1:]):
            category, length = categories[int(codes[first])], int(last-first)
            counts[category] += length
            if runs and runs[-1]["category"] == category:
                runs[-1]["frames"] += length
            else:
                require(len(runs) < MAX_AUDIT_RUNS, "full-recording detail run cap")
                runs.append(dict(start_frame=start+int(first), frames=length, category=category))
    error = dict(support_frames=error_frames[0],
                 channels=_audit_error_stats(error_frames[0], sums[0], squares[0], maxima[0]),
                 nontraining=dict(support_frames=error_frames[1],
                     channels=_audit_error_stats(error_frames[1], sums[1], squares[1], maxima[1])),
                 scope="finite unclipped known-support samples, including training, silence and tails")
    require(sum(counts.values()) == len(recorded), "full-recording denominator mismatch")
    return dict(schema_version=1, status="recording_accounted", recorded_frames=len(recorded),
                counts=counts, runs=runs, anchor=anchor, gates=dict(GATES),
                raw_bits=raw, fixed_gain_dc_error=error, blocks=blocks,
                sample_rate=rate, detail_run_cap=MAX_AUDIT_RUNS,
                training_intervals=[dict(start_frame=a, frames=b-a) for a, b in training],
                qualified=False, waveform_continuity_qualified=False,
                identity_accepted=False, physical_latency_ms=None,
                scope="diagnostic nominal recorder/master integer map possible; no native proof",
                denominator="every decoded recorder frame, including unsupported, training, silence and tails",
                raw_bit_scope="both channels equal in the original encoding, within known support only; not global quality",
                limits=["No search/reanchor/rate/phase/interpolation/refit",
                        "Only the supplied frozen owner's generation is compared",
                        "64-frame informative gates never qualify silence or training",
                        "Association report and input authentication belong to the parent"])


def _audit_error_stats(frames, sums, squares, maxima):
    return [dict(frames=frames, mean_abs=float(sums[c]/frames) if frames else None,
                 rms=float(np.sqrt(squares[c]/frames)) if frames else None,
                 max_abs=float(maxima[c]) if frames else None) for c in range(2)]


def _audit_inputs(owners, recorded, report, rate):
    require(type(rate) is int and rate == 48000, "exact rate48000 required")
    require(type(owners) is dict and 1 <= len(owners) <= 8, "one to eight owners required")
    def shape(value):
        require(isinstance(value, np.ndarray) and value.ndim == 2 and value.shape[1] == 2
                and 0 < len(value) <= MAX_FRAMES and value.dtype.kind == "f"
                and value.dtype.itemsize in (4, 8), "bounded float32/64 stereo required")
    shape(recorded)
    total = recorded.nbytes
    for name, owner in owners.items():
        require(type(name) is str and 0 < len(name) <= 256
                and type(owner) is tuple and len(owner) == 3, "owner generation tuple required")
        replay, valid, generation = owner
        shape(replay)
        require(isinstance(valid, np.ndarray) and valid.shape == (len(replay),)
                and valid.dtype.kind in "uib", "binary validity shape/type")
        require(isinstance(generation, np.ndarray) and generation.shape == valid.shape
                and generation.dtype.kind in "iu" and generation.dtype.itemsize <= 8,
                "generation shape/type invalid")
        total += replay.nbytes + valid.nbytes + generation.nbytes
    require(total <= 512*1024*1024, "combined array byte cap")
    for replay, valid, generation in owners.values():
        for start in range(0, len(replay), 65536):
            v, g = valid[start:start+65536], generation[start:start+65536]
            require(np.isin(v, [0, 1]).all() and np.all(g >= 0)
                    and np.all(g[v == 1] > 0), "binary validity or generation invalid")
    require(type(report) is dict and report.get("qualified") is False
            and report.get("identity_accepted") is False
            and report.get("waveform_continuity_qualified") is False
            and "physical_latency_ms" in report and report["physical_latency_ms"] is None
            and report.get("gates") == GATES, "unqualified unchanged-gates association report required")
    anchor = report.get("anchor")
    require(type(anchor) is dict and set(anchor) == {"owner", "window_index", "generation",
            "recorder_to_replay_shift_frames", "channels"}, "complete frozen anchor required")
    require(type(anchor["owner"]) is str and anchor["owner"] in owners
            and type(anchor["window_index"]) is int
            and type(anchor["recorder_to_replay_shift_frames"]) is int
            and abs(anchor["recorder_to_replay_shift_frames"]) <= MAX_FRAMES
            and type(anchor["generation"]) is int and 0 < anchor["generation"] < 2**64,
            "invalid fixed integer anchor/owner/generation")
    require(type(anchor["channels"]) is list and len(anchor["channels"]) == 2,
            "frozen stereo channel fits required")
    for channel in anchor["channels"]:
        require(type(channel) is dict and set(channel) == {"gain", "dc"}, "fixed gain/DC only")
        for value in channel.values():
            require(type(value) in (int, float) and abs(value) <= 1e12 and np.isfinite(value),
                    "finite bounded frozen gain/DC required")
        require(abs(channel["gain"]) > 1e-12, "zero frozen gain")
    windows = report.get("windows")
    require(type(windows) is list and 1 <= len(windows) <= 64
            and 0 <= anchor["window_index"] < len(windows), "bounded association windows required")
    require(type(report.get("training_attempts")) is int
            and report["training_attempts"] == anchor["window_index"] + 1,
            "frozen training-attempt count mismatch")
    training, end = [], 0
    for index, row in enumerate(windows):
        require(type(row) is dict and type(row.get("window_index")) is int
                and row["window_index"] == index and type(row.get("association_compatible")) is bool,
                "association window ordinal/type")
        window = row.get("window")
        require(type(window) is dict and set(window) == {"start_frame", "frames"},
                "unlabelled window required")
        first, width = window["start_frame"], window["frames"]
        require(type(first) is int and type(width) is int and 256 <= width <= 8192
                and first >= end and first+width <= len(recorded), "window bounds/order")
        end = first+width
        if index <= anchor["window_index"]:
            training.extend([(first, first+width//4), (first+3*width//4, end)])
        if index == anchor["window_index"]:
            location = first+anchor["recorder_to_replay_shift_frames"]
            require(0 <= location and location+width <= len(owners[anchor["owner"]][0]),
                    "frozen anchor outside replay")
            require(int(owners[anchor["owner"]][2][location]) == anchor["generation"],
                    "frozen anchor generation absent at origin")
    return deepcopy(anchor), training
