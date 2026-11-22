#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Fixed-candidate Rust-compatible moments only; never search or qualification."""
from collections.abc import Sequence
import math
import struct


def _integer(value, low, high, name):
    if type(value) is not int or not low <= value <= high:
        raise ValueError(name + " must be a bounded integer")
    return value


def _number(value, name):
    if type(value) not in (int, float):
        raise ValueError(name + " must be numeric, not bool")
    try:
        value = float(value)
    except OverflowError as error:
        raise ValueError(name + " must be finite") from error
    if not math.isfinite(value):
        raise ValueError(name + " must be finite")
    return value


def _sample(value):
    value = _number(value, "sample")
    if abs(value) >= 1:
        raise ValueError("clipped sample")
    value = struct.unpack("<f", struct.pack("<f", value))[0]
    if abs(value) >= 1:
        raise ValueError("sample rounds to clipping in f32")
    return value


def marker_code(index, marker_frames, amplitude):
    """Return a contiguous segment of the seeded xorshift32 f32 codebook."""
    index = _integer(index, 0, 4095, "index")
    marker_frames = _integer(marker_frames, 32, 48000, "marker_frames")
    end = (index + 1) * marker_frames
    if end * 4 > 64 * 1024 * 1024:
        raise ValueError("codebook prefix exceeds 64 MiB")
    amplitude = _sample(amplitude)
    if amplitude <= 0:
        raise ValueError("amplitude must remain positive in f32")
    state, start, result = 0x6D2B79F5, index * marker_frames, []
    for position in range(end):
        state ^= (state << 13) & 0xFFFFFFFF
        state ^= state >> 17
        state ^= (state << 5) & 0xFFFFFFFF
        if position >= start:
            result.append(amplitude if state & 1 == 0 else -amplitude)
    return result


def _samples(values):
    if (not isinstance(values, Sequence) or isinstance(values, (str, bytes, bytearray))
            or not 32 <= len(values) <= 48000):
        raise ValueError("samples must be a sequence of 32..48000 frames")
    return [_sample(value) for value in values]


def _mean(values):
    # Explicit accumulation preserves Rust's ordered f64 sum on every Python version.
    total = 0.0
    for value in values:
        total += value
    return total / len(values)


def _metrics(reference, samples, reference_mean, recording_mean, start, end, polarity):
    dot = left = right = 0.0
    for index in range(start, end):
        a = reference[index] - reference_mean
        b = samples[index] - recording_mean
        dot += a * b
        left += a * a
        right += b * b
    signed = dot / math.sqrt(left * right) if left > 0 and right > 0 else None
    return {
        "reference_rms": math.sqrt(left / (end - start)),
        "recording_rms": math.sqrt(right / (end - start)),
        "signed_correlation": signed,
        "aligned_correlation": None if signed is None else polarity * signed,
    }


def _matches(metrics, min_correlation, min_rms):
    return (metrics["reference_rms"] >= min_rms
            and metrics["recording_rms"] >= min_rms
            and metrics["aligned_correlation"] is not None
            and metrics["aligned_correlation"] >= min_correlation)


def inspect_fixed(reference, samples, inverted, min_correlation=0.7, min_rms=0.0001):
    """Describe one supplied onset, using global means in all four sections."""
    if type(inverted) is not bool:
        raise ValueError("inverted must be bool")
    min_correlation = _number(min_correlation, "min_correlation")
    min_rms = _number(min_rms, "min_rms")
    if not 0 < min_correlation <= 1 or not 0 < min_rms <= 1:
        raise ValueError("thresholds must be in (0, 1]")
    reference, samples = _samples(reference), _samples(samples)
    if len(reference) != len(samples):
        raise ValueError("reference and samples must have equal lengths")
    reference_mean, recording_mean = _mean(reference), _mean(samples)
    polarity, frames = (-1.0 if inverted else 1.0), len(reference)
    full = _metrics(reference, samples, reference_mean, recording_mean, 0, frames, polarity)
    signed = full["signed_correlation"]
    full["confidence"] = None if signed is None else min(abs(signed), 1.0)
    sections = []
    for section in range(4):
        start = section * (frames // 4)
        end = frames if section == 3 else start + frames // 4
        metrics = _metrics(reference, samples, reference_mean, recording_mean,
                           start, end, polarity)
        sections.append({"start_frame": start, "frames": end - start, **metrics,
                         "matches": _matches(metrics, min_correlation, min_rms)})
    sections_match = (full["reference_rms"] >= min_rms
                      and full["recording_rms"] >= min_rms
                      and full["confidence"] is not None
                      and full["confidence"] >= min_correlation
                      and all(section["matches"] for section in sections))
    return {"qualified": False, "identity_accepted": False,
            "scope": "fixed_candidate_diagnostic_no_search", "full": full,
            "sections": sections, "sections_match": sections_match}
