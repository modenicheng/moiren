#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded, independent Process native-PCM/cursor audit and exact SRC replay.

Inputs are closed per-owner cursor archives from --cursor-trace-directory with
native PCM enabled. This is not OBS correspondence or physical qualification.
Raw offered packets (including dropped tails) remain untouched. Outputs are
disk-backed arrays, an explicit validity mask, compact JSON and a PNG. A refused
archive and any reconstructed prefix are retained, never silently repaired.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import sys

import numpy as np


MAX_INPUT_BYTES = 512 * 1024 * 1024
MAX_OUTPUT_BYTES = 512 * 1024 * 1024
MAX_JSON_BYTES = 4 * 1024 * 1024
MAX_PACKET_LINE = 4096
MAX_CHUNKS = 65536
MAX_OWNERS = 8
BLOCK_FRAMES = 65536
PLOT_BINS = 1200
U64_MAX = (1 << 64) - 1
SILENT = 2
PACKET_KEYS = {
    "packet_sequence",
    "offered_before_frames",
    "accepted_before_frames",
    "packet_frames",
    "packet_accepted_frames",
    "flags",
    "generation",
    "device_position_frames",
    "qpc_100ns",
    "stored_before_frames",
}
SPAN_KEYS = {
    "read_sequence",
    "output_start_frame",
    "input_start_frame",
    "frames",
    "generation",
    "phase_bits",
    "step_bits",
    "fade_remaining",
}
SCOPE = "Independent native offered/accepted PCM mapping and actual SRC replay; no OBS correspondence or physical latency qualification."


class Refusal(ValueError):
    pass


def require(condition, reason):
    if not condition:
        raise Refusal(reason)


def uint(value, name, maximum=U64_MAX, minimum=0):
    require(
        type(value) is int and minimum <= value <= maximum,
        "invalid unsigned index/count: " + name,
    )
    return value


def number(obj, key, maximum=U64_MAX, minimum=0):
    return uint(obj[key], key, maximum, minimum)


def check_object(obj, keys=None):
    require(
        type(obj) is dict and (keys is None or set(obj) == keys), "invalid object keys"
    )
    return obj


def no_duplicates(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, "duplicate JSON key: " + key)
        value[key] = item
    return value


def reject_constant(value):
    raise Refusal("nonfinite JSON token: " + value)


def finite_float(value):
    parsed = float(value)
    require(math.isfinite(parsed), "nonfinite JSON number: " + value[:128])
    return parsed


def decode_json(data):
    return check_object(
        json.loads(
            data,
            object_pairs_hook=no_duplicates,
            parse_constant=reject_constant,
            parse_float=finite_float,
        )
    )


def load_json(path):
    require(
        path.is_file() and not path.is_symlink(),
        "missing/nonregular JSON file: " + path.name,
    )
    require(
        path.stat().st_size <= MAX_JSON_BYTES,
        "JSON file exceeds bounded decode limit: " + path.name,
    )
    with path.open("rb") as source:
        data = source.read(MAX_JSON_BYTES + 1)
    require(len(data) <= MAX_JSON_BYTES, "JSON file grew beyond bounded decode limit")
    return decode_json(data)


def schema(value):
    require(
        type(value.get("schema_version")) is int and value["schema_version"] == 1,
        "unsupported/missing schema_version",
    )


def false_qualification(value):
    require(
        value.get("qualified") is False,
        "archive must explicitly remain qualified:false",
    )


def closed(value, *, error_key="error"):
    false_qualification(value)
    require(
        value.get("persistence_complete") is True
        and error_key in value
        and value[error_key] is None,
        "archive persistence incomplete or IO/cap refusal",
    )


def ieee64(value, name):
    require(
        type(value) is str
        and value.isascii()
        and value.isdecimal()
        and len(value) <= 20,
        name + " must be exact f64 bits as decimal string",
    )
    raw = uint(int(value), name)
    decoded = struct.unpack("<d", struct.pack("<Q", raw))[0]
    require(math.isfinite(decoded), "nonfinite " + name)
    return decoded


def float_bits(value):
    return struct.pack("<d", value)


def owner_files(owner, limit):
    """Bounded inventory without materializing a recursive directory listing."""
    uint(limit, "input_limit_bytes", MAX_INPUT_BYTES, 1)
    total = chunks = files = 0
    for directory in (owner, owner / "native-pcm"):
        require(
            directory.is_dir() and not directory.is_symlink(),
            "missing/symlink archive directory",
        )
        with os.scandir(directory) as entries:
            for entry in entries:
                require(not entry.is_symlink(), "symlink is not a native archive file")
                if not entry.is_file(follow_symlinks=False):
                    require(
                        directory == owner
                        and entry.name == "native-pcm"
                        and entry.is_dir(follow_symlinks=False),
                        "unexpected nested archive directory",
                    )
                    continue
                files += 1
                require(files <= MAX_CHUNKS + 16, "archive file-count cap reached")
                total += entry.stat(follow_symlinks=False).st_size
                require(total <= limit, "input archive 512 MiB/configured cap reached")
                if (
                    directory == owner
                    and entry.name.startswith("chunk-")
                    and entry.name.endswith(".json")
                ):
                    require(
                        len(entry.name) == 19 and entry.name[6:14].isdigit(),
                        "invalid cursor chunk filename",
                    )
                    chunks += 1
                require(
                    entry.name != "failed-chunk.json",
                    "failed cursor chunk preserved by producer",
                )
    require(chunks <= MAX_CHUNKS, "cursor chunk-count cap reached")
    return total, chunks


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


class Arrays:
    """Memmaps stay owned until plotting/receipts finish; no raw Python lists."""

    def __init__(self, output):
        self.output = output
        self.maps = {}

    def create(self, name, dtype, count, channels=None):
        shape = (count, channels) if channels else (count,)
        if count:
            value = np.memmap(self.output / name, mode="w+", dtype=dtype, shape=shape)
        else:
            (self.output / name).touch(exist_ok=False)
            value = np.empty(shape, dtype=dtype)
        self.maps[name] = value
        return value

    def read(self, name, path, dtype, count, channels=None):
        shape = (count, channels) if channels else (count,)
        value = (
            np.memmap(path, mode="r", dtype=dtype, shape=shape)
            if count
            else np.empty(shape, dtype=dtype)
        )
        self.maps[name] = value
        return value

    def finish(self):
        # Release every mapping even if one derived output cannot be flushed.
        # Detach first so a refusal path cannot retry failed persistence later.
        maps, self.maps = self.maps, {}
        errors = []
        for name, value in maps.items():
            if isinstance(value, np.memmap):
                try:
                    value.flush()
                except (OSError, ValueError) as error:
                    errors.append(name + " flush: " + str(error))
                finally:
                    try:
                        value._mmap.close()
                    except (OSError, ValueError) as error:
                        errors.append(name + " close: " + str(error))
        if errors:
            raise OSError("array persistence failed: " + "; ".join(errors)[:1024])


def validate_receipts(owner, input_bytes, chunk_count):
    origin = load_json(owner / "origin.json")
    final = load_json(owner / "final.json")
    stream = load_json(owner / "native-pcm" / "stream.json")
    pcm_final = load_json(owner / "native-pcm" / "final.json")
    for document in (origin, final, stream, pcm_final):
        schema(document)
    for document in (final, pcm_final):
        closed(document, error_key="io_error" if document is final else "error")
    false_qualification(stream)
    require(origin == final["origin"], "cursor owner origin mismatch")
    require(
        len(json.dumps(origin)) <= 16384,
        "origin exceeds compact identity/policy budget",
    )
    number(origin, "owner_generation", minimum=1)
    number(origin, "input_sample_rate", 768000, 1)
    number(origin, "consumer_sample_rate", 768000, 1)
    require(
        final.get("native_pcm_enabled") is True
        and final.get("native_pcm_complete") is True
        and final.get("final_history_available") is True,
        "native PCM/cursor closure incomplete",
    )
    require(
        stream["sample_format"] == "f32le-bits"
        and type(stream["frame_bytes"]) is int
        and stream["frame_bytes"] == 8,
        "unsupported raw sample/frame format",
    )
    channels = number(stream, "channels", 2, 1)
    rate = number(stream, "sample_rate", 768000, 1)
    disk_limit = number(stream, "disk_limit_bytes", MAX_INPUT_BYTES, 1)
    require(
        rate == origin["input_sample_rate"], "native/cursor input sample-rate mismatch"
    )
    summary = check_object(pcm_final["summary"])
    for key in ("capacity_frames", "sample_rate", "channels"):
        number(summary, key, minimum=1)
        require(
            stream["summary"][key] == summary[key],
            "native PCM summary configuration mismatch: " + key,
        )
    require(
        summary["channels"] == channels and summary["sample_rate"] == rate,
        "native PCM top-level configuration mismatch",
    )
    for key in ("lost_frames", "overflow_packets", "reader_abandoned_packets"):
        require(number(summary, key) == 0, "native PCM evidence loss: " + key)
    require(
        summary.get("saturated") is False and summary.get("writer_finished") is True,
        "native PCM saturated or unclosed writer",
    )
    for key in ("packets", "offered_frames", "accepted_frames", "stored_frames"):
        number(summary, key)
    require(
        summary["packets"] > 0
        and summary["stored_frames"] == summary["offered_frames"]
        and summary["accepted_frames"] <= summary["offered_frames"],
        "native PCM offered/accepted totals invalid",
    )
    require(
        "partial_packet" in pcm_final
        and pcm_final["partial_packet"] is None
        and number(pcm_final, "pending_frames") == 0,
        "native PCM partial packet/pending frames at closure",
    )
    require(
        number(pcm_final, "persisted_frames") == summary["stored_frames"]
        and number(pcm_final, "persisted_packets") == summary["packets"],
        "native PCM persisted counters mismatch",
    )
    raw = owner / "native-pcm" / "samples.f32le"
    packets = owner / "native-pcm" / "packets.jsonl"
    require(
        raw.is_file() and packets.is_file(), "missing native PCM raw/metadata files"
    )
    require(
        raw.stat().st_size == summary["offered_frames"] * 8,
        "native PCM raw bad length/partial packet",
    )
    require(
        number(pcm_final, "persisted_bytes")
        == raw.stat().st_size + packets.stat().st_size
        and pcm_final["persisted_bytes"] <= disk_limit,
        "native PCM persisted byte tally/cap mismatch",
    )
    artifact = check_object(final["artifact"])
    schema(artifact)
    false_qualification(artifact)
    require(
        artifact.get("history_complete") is True
        and artifact.get("finalized") is True
        and artifact.get("writer_finished") is True
        and "refusal" in artifact
        and artifact["refusal"] is None,
        "cursor structural history incomplete/saturated/unclosed",
    )
    for key in ("retained_events", "missing_events", "unretained_events"):
        require(number(artifact, key) == 0, "cursor incomplete history: " + key)
    require(artifact.get("events") == [], "unpersisted cursor events at closure")
    number(artifact, "event_limit", 131072, 1)
    requested = number(artifact, "requested_frames")
    reads = number(artifact, "started_reads", minimum=1)
    require(
        number(artifact, "completed_reads") == reads and requested >= reads,
        "cursor read/requested frontier incomplete",
    )
    events = number(artifact, "total_events")
    require(
        number(artifact, "drained_events")
        == events
        == number(final, "persisted_events"),
        "cursor event persistence tally mismatch",
    )
    require(
        number(artifact, "drained_chunks", MAX_CHUNKS)
        == chunk_count
        == number(final, "persisted_chunks", MAX_CHUNKS),
        "cursor chunk count mismatch/gap/extra chunks",
    )
    # Account for all disk-backed derived arrays before allocating even one.
    derived_bytes = summary["accepted_frames"] * 16 + requested * 17
    require(
        derived_bytes <= MAX_OUTPUT_BYTES, "derived PCM/replay allocation cap reached"
    )
    return origin, summary, artifact, raw, packets, channels, rate


def map_native(raw, packets, summary, channels, arrays):
    offered = summary["offered_frames"]
    accepted_count = summary["accepted_frames"]
    native = arrays.read("native", raw, "<f4", offered, 2)
    accepted = arrays.create("accepted.f32le", "<f4", accepted_count, 2)
    generations = arrays.create("accepted-generation.u64le", "<u8", accepted_count)
    counts = dict(
        packets=0,
        offered_frames=0,
        accepted_frames=0,
        stored_frames=0,
        dropped_tail_frames=0,
        normalized_nonfinite_samples=0,
        silent_offered_frames=0,
        native_discontinuity_packets=0,
        timestamp_error_packets=0,
    )
    previous_generation = 0
    with packets.open("rb") as source:
        while line := source.readline(MAX_PACKET_LINE + 1):
            require(
                len(line) <= MAX_PACKET_LINE and line.endswith(b"\n"),
                "partial/oversized packet metadata line",
            )
            packet = check_object(decode_json(line), PACKET_KEYS)
            for key in PACKET_KEYS:
                number(
                    packet,
                    key,
                    (1 << 32) - 1
                    if key in ("packet_frames", "packet_accepted_frames", "flags")
                    else U64_MAX,
                )
            require(
                packet["packet_sequence"] == counts["packets"] + 1,
                "packet sequence gap/overlap",
            )
            require(
                packet["offered_before_frames"] == counts["offered_frames"]
                and packet["accepted_before_frames"] == counts["accepted_frames"]
                and packet["stored_before_frames"] == counts["stored_frames"],
                "packet offered/accepted/stored index gap/overlap",
            )
            frames, count = packet["packet_frames"], packet["packet_accepted_frames"]
            require(
                frames > 0 and count <= frames, "invalid native packet/accepted length"
            )
            generation = packet["generation"]
            require(
                generation > 0 and generation >= previous_generation,
                "packet generation regression/zero",
            )
            start = counts["stored_frames"]
            destination = counts["accepted_frames"]
            require(
                start + frames <= offered and destination + count <= accepted_count,
                "partial native packet/excess accepted index",
            )
            for offset in range(0, frames, BLOCK_FRAMES):
                block = np.asarray(
                    native[start + offset : start + min(frames, offset + BLOCK_FRAMES)]
                )
                raw_bits = block.view("<u4")
                require(
                    channels != 1 or bool(np.all(raw_bits[:, 1] == 0)),
                    "mono padding bits not zero",
                )
                require(
                    not packet["flags"] & SILENT or bool(np.all(raw_bits == 0)),
                    "SILENT packet raw bits not zero",
                )
                used = max(0, min(len(block), count - offset))
                if used:
                    converted = np.array(block[:used], dtype="<f4", copy=True)
                    finite = np.isfinite(converted)
                    counts["normalized_nonfinite_samples"] += int(
                        np.count_nonzero(~finite[:, :channels])
                    )
                    converted[~finite] = 0.0
                    if packet["flags"] & SILENT:
                        converted[:] = 0.0
                    elif channels == 1:
                        converted[:, 1] = converted[:, 0]
                    accepted[destination + offset : destination + offset + used] = (
                        converted
                    )
                    generations[destination + offset : destination + offset + used] = (
                        generation
                    )
            counts["packets"] += 1
            counts["offered_frames"] += frames
            counts["stored_frames"] += frames
            counts["accepted_frames"] += count
            counts["dropped_tail_frames"] += frames - count
            counts["silent_offered_frames"] += frames if packet["flags"] & SILENT else 0
            counts["native_discontinuity_packets"] += bool(packet["flags"] & 1)
            counts["timestamp_error_packets"] += bool(packet["flags"] & 4)
            previous_generation = generation
    require(
        all(
            counts[key] == summary[key]
            for key in ("packets", "offered_frames", "accepted_frames", "stored_frames")
        ),
        "native packet metadata final tally mismatch",
    )
    return counts


def replay(owner, artifact, arrays):
    requested = artifact["requested_frames"]
    accepted = arrays.maps["accepted.f32le"]
    generations = arrays.maps["accepted-generation.u64le"]
    output = arrays.create("reconstructed.f32le", "<f4", requested, 2)
    valid = arrays.create("reconstructed-valid.u8", "u1", requested)
    cursor = arrays.create("cursor-input.f64le", "<f8", requested)
    for start in range(0, requested, BLOCK_FRAMES):
        stop = min(requested, start + BLOCK_FRAMES)
        output[start:stop] = 0.0
        valid[start:stop] = 0
        cursor[start:stop] = np.nan
    events = emitted = 0
    previous = None
    for chunk_index in range(1, artifact["drained_chunks"] + 1):
        chunk = load_json(owner / ("chunk-%08d.json" % chunk_index))
        schema(chunk)
        require(
            number(chunk, "sequence") == chunk_index
            and type(chunk["events"]) is list
            and 1 <= len(chunk["events"]) <= artifact["event_limit"],
            "cursor chunk sequence/size invalid",
        )
        for row in chunk["events"]:
            check_object(row, {"sequence", "span"})
            require(
                number(row, "sequence") == events + 1,
                "cursor event sequence gap/overlap",
            )
            span = check_object(row["span"], SPAN_KEYS)
            for key in SPAN_KEYS - {"phase_bits", "step_bits"}:
                number(span, key)
            frames = number(span, "frames", 32, 1)
            read = number(span, "read_sequence", artifact["started_reads"], 1)
            start = span["output_start_frame"]
            require(
                start + frames <= requested and start >= read - 1,
                "cursor output/read index outside requested frontier",
            )
            phase = ieee64(span["phase_bits"], "phase_bits")
            step = ieee64(span["step_bits"], "step_bits")
            require(0.0 <= phase < 1.0 and step > 0.0, "invalid cursor phase/step")
            # A malformed DTO must not turn exact repeated subtraction into an
            # unbounded loop. This is a replay work cap, not a latency gate.
            require(step <= 4096.0, "cursor step exceeds bounded replay work budget")
            fade = number(span, "fade_remaining", 32)
            index = span["input_start_frame"]
            generation = number(span, "generation", minimum=1)
            if previous:
                require(
                    read >= previous["read"] and start >= previous["end"],
                    "cursor output overlap/read regression",
                )
                require(
                    start - previous["end"] >= max(0, read - previous["read"] - 1),
                    "missing nonempty read output frontier",
                )
                require(
                    index >= previous["last_input"],
                    "cursor accepted input index regression",
                )
                if read == previous["read"]:
                    require(
                        start == previous["end"], "cursor gap inside one source read"
                    )
                    require(
                        index == previous["next_input"]
                        and float_bits(phase) == previous["next_phase"]
                        and fade == previous["fade"],
                        "cursor same-read recurrence/phase/fade mismatch",
                    )
                # Recovery/prime runs before a new read's emission loop and
                # may reset phase/fade even at the predicted input frontier.
                # Without queue/reset history a read boundary is not proof of
                # recurrence continuity. The new actual span is authoritative.
            indices = np.empty(frames, dtype=np.intp)
            phases = np.empty(frames, dtype=np.float64)
            gains = np.empty(frames, dtype=np.float64)
            for offset in range(frames):
                # Match the native source's operation order, not phase+k*step.
                while phase >= 1.0:
                    phase -= 1.0
                    index += 1
                require(
                    index + 1 < len(accepted),
                    "cursor accepted input pair outside retained PCM",
                )
                indices[offset] = index
                phases[offset] = phase
                gains[offset] = (33 - fade) / 32 if fade else 1.0
                fade = max(0, fade - 1)
                phase += step
            require(
                bool(np.all(generations[indices] == generation))
                and bool(np.all(generations[indices + 1] == generation)),
                "cursor/native PCM generation mismatch/crossing pair",
            )
            a = np.asarray(accepted[indices], dtype=np.float64)
            b = np.asarray(accepted[indices + 1], dtype=np.float64)
            pair = (a * (1.0 - phases[:, None]) + b * phases[:, None]) * gains[:, None]
            output[start : start + frames] = pair.astype("<f4")
            cursor[start : start + frames] = indices + phases
            valid[start : start + frames] = 1
            emitted += frames
            events += 1
            last_input = index
            while phase >= 1.0:
                phase -= 1.0
                index += 1
            previous = dict(
                read=read,
                end=start + frames,
                next_input=index,
                last_input=last_input,
                next_phase=float_bits(phase),
                fade=fade,
                generation=generation,
            )
    require(events == artifact["total_events"], "cursor final event count mismatch")
    if previous:
        require(
            requested - previous["end"] >= artifact["started_reads"] - previous["read"],
            "cursor last requested/read frontier inconsistent",
        )
    return dict(
        events=events,
        reconstructed_frames=emitted,
        requested_frames=requested,
        unknown_or_priming_frames=requested - emitted,
        all_requested_frames_reconstructed=emitted == requested,
        holes="Zero-filled storage with validity=0; UNKNOWN/priming, not successful audio.",
        operation="normalize phase before EACH sample; repeated f64 add/subtract; interpolate in f64; 32-frame fade; f32 cast",
        obs_correspondence="not_established",
        audio_correctness_qualified=False,
    )


def envelope(samples, *, mask=None):
    """Reduce bounded slices; even very long input never becomes a full array."""
    count = len(samples)
    width = max(1, (count + PLOT_BINS - 1) // PLOT_BINS)
    positions, lows, highs = [], [], []
    for start in range(0, count, width):
        part = np.asarray(samples[start : start + width])
        if mask is not None:
            part = part[np.asarray(mask[start : start + width], dtype=bool)]
        finite = part[np.isfinite(part)]
        positions.append(start + (min(width, count - start) - 1) / 2)
        lows.append(float(np.min(finite)) if finite.size else math.nan)
        highs.append(float(np.max(finite)) if finite.size else math.nan)
    return np.asarray(positions), np.asarray(lows), np.asarray(highs)


def plot_overview(report, arrays, path):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    figure, axes = plt.subplots(3, 1, figsize=(12, 8), constrained_layout=True)
    figure.suptitle(
        "Native PCM / actual Process SRC replay - "
        + report["status"]
        + "\nUNQUALIFIED: no OBS or physical latency correspondence",
        fontsize=12,
    )
    for ax, name, label in (
        (
            axes[0],
            "native",
            "Native OFFERED PCM (includes unpublished tails; nonfinite omitted for display only)",
        ),
        (
            axes[1],
            "reconstructed.f32le",
            "Reconstructed SRC output (valid mask only; gaps UNKNOWN/priming)",
        ),
    ):
        ax.set_title(label, fontsize=10)
        if name in arrays.maps:
            mask = (
                arrays.maps.get("reconstructed-valid.u8")
                if name.startswith("reconstructed")
                else None
            )
            x, lo, hi = envelope(arrays.maps[name], mask=mask)
            ax.fill_between(x, lo, hi, color="#167d8d", alpha=0.7)
        ax.set_ylabel("f32 amplitude")
        ax.set_xlabel(
            "offered input frame" if name == "native" else "requested output frame"
        )
        ax.grid(alpha=0.2)
    axes[2].set_title(
        "Actual accepted-input cursor path; no cursor invented for holes", fontsize=10
    )
    if "cursor-input.f64le" in arrays.maps:
        x, lo, hi = envelope(
            arrays.maps["cursor-input.f64le"],
            mask=arrays.maps["reconstructed-valid.u8"],
        )
        axes[2].fill_between(x, lo, hi, color="#c47824", alpha=0.6)
        axes[2].plot(x, (lo + hi) / 2, color="#9d5a16", linewidth=0.8)
    axes[2].set_xlabel("requested output frame")
    axes[2].set_ylabel("accepted input frame")
    axes[2].grid(alpha=0.2)
    if report["refusals"]:
        axes[2].text(
            0.02,
            0.95,
            "REFUSED: " + "\n".join(report["refusals"][:4]),
            transform=axes[2].transAxes,
            va="top",
            fontsize=9,
            bbox={"facecolor": "#fff4e6", "alpha": 0.95, "edgecolor": "#b03a2e"},
            wrap=True,
        )
    figure.savefig(path, dpi=140)
    plt.close(figure)


def plot_owner_ledger(report, path):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    owners = report["owners"]
    labels = [item["directory"] for item in owners]
    known = [item["replay"].get("reconstructed_frames", 0) for item in owners]
    unknown = [item["replay"].get("unknown_or_priming_frames", 0) for item in owners]
    figure, axes = plt.subplots(
        2,
        1,
        figsize=(12, 7),
        constrained_layout=True,
        gridspec_kw={"height_ratios": [2, 1]},
    )
    figure.suptitle(
        "Process native PCM archive coverage - "
        + report["status"]
        + "\nUNQUALIFIED: replay is not OBS correspondence or physical latency",
        fontsize=12,
    )
    if owners:
        rows = np.arange(len(owners))
        axes[0].barh(
            rows, known, color="#167d8d", label="reconstructed from actual spans"
        )
        axes[0].barh(
            rows,
            unknown,
            left=known,
            color="#d4b68c",
            label="UNKNOWN / priming / unfilled",
        )
        axes[0].set_yticks(rows, labels)
        axes[0].legend(loc="best", fontsize=9)
        axes[0].invert_yaxis()
    axes[0].set_xlabel(
        "requested output frames (per-owner waveforms in owner/overview.png)"
    )
    axes[0].grid(axis="x", alpha=0.2)
    axes[1].axis("off")
    lines = [item["directory"] + ": " + item["status"] for item in owners]
    lines.extend("REFUSED: " + reason for reason in report["refusals"][:4])
    axes[1].text(
        0.01,
        0.95,
        "\n".join(lines) or "No complete owner results retained.",
        va="top",
        fontsize=10,
        bbox={"facecolor": "#f6f1e9", "alpha": 0.95, "edgecolor": "#9d8d75"},
    )
    figure.savefig(path, dpi=140)
    plt.close(figure)


def write_report(path, report):
    with path.open("x", encoding="utf-8") as target:
        json.dump(report, target, indent=2, allow_nan=False)
        target.write("\n")


def audit_owner(
    owner_directory,
    output_directory,
    *,
    make_plot=True,
    input_limit_bytes=MAX_INPUT_BYTES,
):
    owner, output = Path(owner_directory).resolve(), Path(output_directory).resolve()
    require(
        not output.is_relative_to(owner) and not owner.is_relative_to(output),
        "analysis output must be outside the immutable source archive",
    )
    output.mkdir(parents=True, exist_ok=False)
    arrays = Arrays(output)
    report = dict(
        schema_version=1,
        status="refused",
        qualified=False,
        physical_latency_ms=None,
        source_directory=str(owner),
        scope=SCOPE,
        refusals=[],
        input_bytes=0,
        native_pcm={},
        replay={},
        outputs={},
        input_integrity="closed archive metadata cross-check; not authentication",
    )
    try:
        try:
            input_bytes, chunks = owner_files(owner, input_limit_bytes)
            report["input_bytes"] = input_bytes
            origin, summary, artifact, raw, packets, channels, rate = validate_receipts(
                owner, input_bytes, chunks
            )
            report["origin"] = origin
            report["native_pcm"] = map_native(raw, packets, summary, channels, arrays)
            report["native_pcm"].update(
                channels=channels,
                sample_rate=rate,
                raw_sha256=sha256(raw),
                metadata_sha256=sha256(packets),
            )
            report["replay"] = replay(owner, artifact, arrays)
            report["status"] = "structurally_complete"
        except (
            OSError,
            ValueError,
            KeyError,
            TypeError,
            OverflowError,
            RecursionError,
        ) as error:
            report["refusals"].append((str(error) or type(error).__name__)[:1024])
            report["replay"]["partial_outputs_untrusted"] = True
        for name in arrays.maps:
            if name != "native":
                report["outputs"][name] = {"bytes": (output / name).stat().st_size}
        if make_plot:
            try:
                plot_overview(report, arrays, output / "overview.png")
                report["outputs"]["overview.png"] = {
                    "bytes": (output / "overview.png").stat().st_size
                }
            except (OSError, ValueError, ImportError) as error:
                report["status"] = "refused"
                report["refusals"].append("visualization failed: " + str(error)[:1024])
        try:
            arrays.finish()
        except (OSError, ValueError) as error:
            report["status"] = "refused"
            report["refusals"].append(str(error)[:1024])
            report["replay"]["partial_outputs_untrusted"] = True
        write_report(output / "summary.json", report)
        return report
    finally:
        arrays.finish()


def audit_archive(
    archive_directory,
    output_directory,
    *,
    make_plot=True,
    input_limit_bytes=MAX_INPUT_BYTES,
):
    root, output = Path(archive_directory).resolve(), Path(output_directory).resolve()
    if (root / "native-pcm").is_dir():
        return audit_owner(
            root, output, make_plot=make_plot, input_limit_bytes=input_limit_bytes
        )
    require(
        not output.is_relative_to(root) and not root.is_relative_to(output),
        "analysis output must be outside the immutable source archive",
    )
    output.mkdir(parents=True, exist_ok=False)
    report = dict(
        schema_version=1,
        status="refused",
        qualified=False,
        physical_latency_ms=None,
        scope=SCOPE,
        refusals=[],
        owners=[],
    )
    try:
        receipt = load_json(root / "summary.json")
        schema(receipt)
        false_qualification(receipt)
        require(
            "error" in receipt
            and receipt["error"] is None
            and receipt.get("native_pcm_enabled") is True,
            "root archive error or native PCM disabled",
        )
        count = number(receipt, "expected_owners", MAX_OWNERS, 1)
        require(
            number(receipt, "sessions", MAX_OWNERS, 1) == count,
            "root expected owner coverage incomplete",
        )
        owners = []
        with os.scandir(root) as entries:
            for entry in entries:
                if entry.name.startswith("owner-"):
                    require(
                        entry.is_dir(follow_symlinks=False) and not entry.is_symlink(),
                        "invalid owner archive directory",
                    )
                    owners.append(Path(entry.path))
                    require(len(owners) <= MAX_OWNERS, "owner count cap reached")
        require(len(owners) == count, "owner archive count mismatch")
        total = (root / "summary.json").stat().st_size
        for owner in owners:
            total += owner_files(owner, input_limit_bytes)[0]
            require(
                total <= input_limit_bytes, "combined owner archive input cap reached"
            )
        for ordinal, owner in enumerate(sorted(owners), 1):
            require(
                owner.name.startswith("owner-%04d-generation-" % ordinal),
                "owner ordinal coverage gap",
            )
            item = audit_owner(
                owner,
                output / owner.name,
                make_plot=make_plot,
                input_limit_bytes=input_limit_bytes,
            )
            report["owners"].append(
                dict(
                    directory=owner.name,
                    status=item["status"],
                    qualified=False,
                    refusals=item["refusals"],
                    native_pcm=item["native_pcm"],
                    replay=item["replay"],
                )
            )
        require(
            all(item["status"] == "structurally_complete" for item in report["owners"]),
            "one or more owner audits refused; preserved per-owner outputs",
        )
        report["status"] = "structurally_complete"
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        OverflowError,
        RecursionError,
    ) as error:
        report["refusals"].append((str(error) or type(error).__name__)[:1024])
    if make_plot:
        try:
            plot_owner_ledger(report, output / "overview.png")
            report["overview_kind"] = "per_owner_coverage_ledger"
        except (OSError, ValueError, ImportError) as error:
            report["status"] = "refused"
            report["refusals"].append("visualization failed: " + str(error)[:1024])
    write_report(output / "summary.json", report)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--archive-directory",
        type=Path,
        required=True,
        help="closed owner directory or complete multi-owner archive root",
    )
    parser.add_argument(
        "--output-directory",
        type=Path,
        required=True,
        help="new directory outside source; never overwritten",
    )
    parser.add_argument(
        "--input-limit-mib",
        type=int,
        default=512,
        choices=range(1, 513),
        metavar="1..512",
    )
    parser.add_argument(
        "--no-plot", action="store_true", help="omit PNG, primarily for unit tests"
    )
    args = parser.parse_args(argv)
    try:
        report = audit_archive(
            args.archive_directory,
            args.output_directory,
            make_plot=not args.no_plot,
            input_limit_bytes=args.input_limit_mib * 1024 * 1024,
        )
    except (OSError, ValueError) as error:
        print("Refused: " + str(error), file=sys.stderr)
        return 2
    print(
        json.dumps(
            dict(
                status=report["status"],
                qualified=False,
                summary=str(args.output_directory / "summary.json"),
                refusals=report["refusals"],
            )
        )
    )
    return 0 if report["status"] == "structurally_complete" else 2


if __name__ == "__main__":
    sys.exit(main())
