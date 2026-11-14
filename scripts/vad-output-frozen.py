#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare a closed actual MIC WRITE witness with an independent recording.

Reuse one frozen integer association and count every recorder frame. Accepted
WRITE/completion evidence is not native consumption, a common clock, physical
latency, or a quality pass. This tool never opens audio or changes thresholds.
"""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path
import re

import numpy as np


SPEC = importlib.util.spec_from_file_location(
    "output_frozen", Path(__file__).with_name("process-replay-frozen.py")
)
FROZEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FROZEN)
INPUT_NAMES = ("report.json", "collection.json", "vad-output.metadata.json",
               "vad-output.raw.bin", "measurement.f32le", "measurement.packets.jsonl")
MAX_FRAMES = 480000
OWNER = "actual_encoded_mic_write_prefix"


def require(ok, message):
    if not ok:
        raise ValueError(message)


def integer(value, low=0, high=2**64-1):
    require(type(value) is int and low <= value <= high, "invalid bounded integer")
    return value


def endpoint(value):
    require(type(value) is str and re.fullmatch(r"[0-9a-fA-F]{32}", value),
            "exact stored-byte endpoint hex required")
    return list(bytes.fromhex(value))


def decode_output(meta, raw, expected_endpoint):
    require(type(meta) is dict and type(meta.get("schema_version")) is int and meta["schema_version"] == 1
            and meta.get("qualified") is False, "unqualified output schema1 required")
    require(meta.get("worker_join") == "joined", "actual worker join required")
    require(meta.get("worker_outcome") in ("completed", "failed", "panicked"),
            "explicit terminal worker outcome required")
    limit = integer(meta["requested_frames"], 1, MAX_FRAMES)
    frames = integer(meta["stored_frames"], 1, limit)
    require(type(raw) is bytes and len(raw) == integer(meta["raw_bytes"], 1, 4*1024*1024),
            "raw byte cap/length mismatch")
    identity = meta["identity"]
    require(type(identity) is dict and identity["stable_id"] == endpoint(expected_endpoint),
            "output endpoint identity differs")
    require(type(identity["stable_id"]) is list and len(identity["stable_id"]) == 16,
            "stable identity shape")
    for value in identity["stable_id"]:
        integer(value, 0, 255)
    require(type(identity["native_stamp"]) is list and len(identity["native_stamp"]) == 5,
            "native identity stamp shape")
    for value in identity["native_stamp"]:
        integer(value)
    for key in ("reset_epoch", "admission_ticket", "active_revision"):
        integer(identity[key])
    fmt = integer(identity["format"], 1, 2)
    require(integer(identity["channels"], 2, 2) == 2, "stereo witness required")
    packet_frames = integer(identity["packet_frames"], 1, 4800)
    width = 4 if fmt == 1 else 8
    require(len(raw) == frames*width, "raw frame width mismatch")
    source = np.frombuffer(raw, dtype="<i2" if fmt == 1 else "<f4").reshape(-1, 2)
    values = source.astype("<f4") / np.float32(32768) if fmt == 1 else source.copy()
    valid = np.zeros(frames, dtype="u1")
    generation = np.ones(frames, dtype="<u8")
    rows = meta["packets"]
    require(type(rows) is list and 1 <= len(rows) <= 4096, "output packet cap")
    byte_offset = start = zero_fill = 0
    for row in rows:
        require(type(row) is dict, "output packet object required")
        n = integer(row["actual_frames"], 1, 4800)
        size = integer(row["byte_count"], 1, 38400)
        require(integer(row["byte_offset"]) == byte_offset and n == packet_frames
                and size == n*width and start+n <= frames, "output packet coverage/width")
        zero_fill += integer(row["silence_frames"], 0, n)
        for key in ("sequence", "host_submission_qpc"):
            if row[key] is not None:
                integer(row[key])
        require(row["identity_current_after_submit"] is None
                or type(row["identity_current_after_submit"]) is bool, "identity current type")
        require(type(row["submission"]) is str and type(row["completion"]) is str,
                "output disposition strings required")
        if row["completion_bytes"] is not None:
            integer(row["completion_bytes"], 0, 38400)
        if row["raw_hresult"] is not None:
            integer(row["raw_hresult"], -(2**31), 2**31-1)
        require(row["error_kind"] is None or type(row["error_kind"]) is str,
                "output error kind type")
        # Unknown/failed/fenced attempts retain their bytes but never provide
        # comparison support. Observed completion still is not consumption.
        supported = (meta["worker_outcome"] == "completed"
                     and row["submission"] == "accepted"
                     and row["identity_current_after_submit"] is True
                     and row["completion"] == "completed"
                     and row["completion_bytes"] == size
                     and row["sequence"] is not None and row["host_submission_qpc"] is not None
                     and row["error_kind"] is None and row["raw_hresult"] is None)
        valid[start:start+n] = int(supported)
        start += n
        byte_offset += size
    require(start == frames and byte_offset == len(raw), "output tail/denominator mismatch")
    receipt = {key: meta[key] for key in ("identity", "seal_reason", "worker_join",
               "worker_outcome", "requested_frames", "stored_frames")}
    receipt.update(qualified=False, supported_frames=int(valid.sum()),
                   unsupported_frames=frames-int(valid.sum()), bridge_zero_fill_frames=zero_fill,
                   packet_count=len(rows), submission_counts=dict(Counter(r["submission"] for r in rows)),
                   completion_counts=dict(Counter(r["completion"] for r in rows)),
                   scope="encoded attempts with observed current accepted completions; not consumption")
    for key in ("excluded_epoch_packets", "untracked_completion_observations"):
        receipt[key] = integer(meta[key])
    return values, valid, generation, receipt


def windows(frames):
    integer(frames, 256, MAX_FRAMES)
    result, start = [], 0
    while frames-start > 8192:
        n = 8192 if frames-start >= 8448 else frames-start-256
        result.append(dict(start_frame=start, frames=n))
        start += n
    result.append(dict(start_frame=start, frames=frames-start))
    require(len(result) <= 64, "window cap")
    return result


def recorder_packets(rows, frames):
    integer(frames, 1, MAX_FRAMES)
    require(type(rows) is list and 1 <= len(rows) <= 4096, "recorder packet cap")
    end, previous = 0, None
    flagged, gaps = [], []
    for index, row in enumerate(rows, 1):
        for key in ("packet_sequence", "flags", "device_position_frames", "qpc_100ns",
                    "stored_start_frame"):
            integer(row[key])
        n = integer(row["frames"], 1, 4800)
        require(row["packet_sequence"] == index and row["stored_start_frame"] == end,
                "recorder sequence/stored coverage mismatch")
        if row["flags"]:
            flagged.append(dict(row))
        if previous is not None:
            require(row["qpc_100ns"] >= previous["qpc_100ns"], "recorder QPC regression")
            delta = row["device_position_frames"]-previous["device_position_frames"]-previous["frames"]
            if delta:
                gaps.append(dict(packet_sequence=index, stored_start_frame=end, delta_frames=delta))
        end += n
        previous = row
    require(end == frames, "recorder tail/denominator mismatch")
    return dict(qualified=False, stored_frames=end, packet_count=len(rows), flagged_packets=flagged,
                position_gaps=gaps, scope="independent stored recorder axis; not source/physical time")


def analyze(meta, raw, recorded, rows, expected_endpoint):
    require(isinstance(recorded, np.ndarray) and recorded.ndim == 2
            and recorded.shape[1] == 2 and recorded.dtype == np.dtype("<f4")
            and 0 < len(recorded) <= MAX_FRAMES, "bounded exact F32 stereo recorder required")
    output, valid, generation, witness = decode_output(meta, raw, expected_endpoint)
    packet_report = recorder_packets(rows, len(recorded))
    result = dict(schema_version=1, qualified=False, waveform_continuity_qualified=False,
                  identity_accepted=False, physical_latency_ms=None, recorded_frames=len(recorded),
                  witness=witness, recorder_packets=packet_report, association=None,
                  full_recording_audit=None, analysis_refusal=None, unassessed_frames=len(recorded),
                  recording_nonfinite_samples=int((~np.isfinite(recorded)).sum()),
                  recording_clipped_samples=int((np.abs(recorded) >= 1).sum()),
                  output_nonfinite_samples=int((~np.isfinite(output)).sum()),
                  output_clipped_samples=int((np.abs(output) >= 1).sum()),
                  scope="one frozen stored-recorder/encoded-WRITE association; not native or physical qualification")
    try:
        owners = {OWNER: (output, valid, generation)}
        association = FROZEN.associate(owners, recorded, windows(len(recorded)))
        result["association"] = association
        if association["anchor"] is not None:
            result["full_recording_audit"] = FROZEN.audit_recording(owners, recorded, association)
            result["unassessed_frames"] = 0
        else:
            result["analysis_refusal"] = "no unique frozen association; all recorder frames remain unassessed"
    except ValueError as error:
        result["analysis_refusal"] = str(error)
    return result


def parse_json(data):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    return json.loads(data, object_pairs_hook=unique)


def pinned_inputs(root, manifest):
    require(type(manifest) is dict and type(manifest.get("schema_version")) is int and manifest["schema_version"] == 1
            and type(manifest.get("files")) is list, "input pin schema1 required")
    pins = {}
    for pin in manifest["files"]:
        name = pin["name"]
        require(type(name) is str and name in INPUT_NAMES and name not in pins,
                "unexpected/duplicate/unsafe input pin")
        integer(pin["bytes"], 0, 32*1024*1024)
        require(type(pin["sha256"]) is str and re.fullmatch(r"[0-9a-fA-F]{64}", pin["sha256"]),
                "invalid SHA256 pin")
        pins[name] = pin
    require(set(pins) == set(INPUT_NAMES) and sum(p["bytes"] for p in pins.values()) <= 64*1024*1024,
            "input set/aggregate cap mismatch")
    result = {}
    for name, pin in pins.items():
        path = root / name
        require(path.is_file() and not path.is_symlink() and path.stat().st_size == pin["bytes"],
                "regular input size/pin mismatch: " + name)
        with path.open("rb") as stream:
            data = stream.read(pin["bytes"]+1)
        require(len(data) == pin["bytes"] and hashlib.sha256(data).hexdigest().lower() == pin["sha256"].lower(),
                "input size/hash changed: " + name)
        result[name] = data
    return result


def validate_envelope(data, meta, expected_endpoint, recorded_frames):
    report, collection = parse_json(data["report.json"]), parse_json(data["collection.json"])
    require(report["qualified"] is False and report["status"] == "unqualified"
            and collection["qualified"] is False and report["collection"] == collection,
            "original unqualified report/collection mismatch")
    reference = report["native"]["vad_output_reference"]
    require(reference["qualified"] is False and reference["metadata"] == meta
            and reference["status"] == "retained_not_delivery_proof"
            and reference["failure"] is None, "persisted output-reference mismatch")
    route = collection["graph"]["route_evidence"]
    prepared, resolved = route["recorder_prepared_microphone"], route["resolved_microphone"]
    expected_stable = bytes(endpoint(expected_endpoint))[::-1].hex()
    require(prepared["stable_endpoint_id"] == expected_stable
            and route["prepared_recorder_endpoint_id"] == prepared["mmdevice_id"]
            and all(type(prepared[key]) is str and prepared[key]
                    and prepared[key] == resolved[key] for key in
                    ("stable_endpoint_id", "mmdevice_id", "adapter_instance", "root_instance"))
            and prepared["mmdevice_id"] == route["recorder_endpoint_id"],
            "prepared/resolved MIC identity mismatch")
    require(route["qualified"] is False and route["route"] == "microphone"
            and route["boundary"] == "internal" and route["bus"] is True
            and route["microphone_stable_endpoint_id"].lower() == expected_endpoint.lower()
            and route["recorder_identity_recheck"]["status"] == "matched"
            and route["recorder_endpoint_id"] == collection["measurement"]["info"]["input_endpoint_id"],
            "archived requested MIC route/recorder binding mismatch")
    info, sink = collection["measurement"]["info"], collection["measurement"]["sink"]
    require(integer(info["channels"], 2, 2) == 2 and integer(info["sample_rate"], 48000, 48000) == 48000
            and sink["written_frames"] == recorded_frames
            and sink["written_pcm_bytes"] == len(data["measurement.f32le"]), "recorder format/count mismatch")
    return dict(qualified=False, original_quality_passed=collection["quality_passed"],
                original_collection_complete=collection["collection_complete"],
                original_collection_failure=collection["failure"], original_cleanup_complete=collection["cleanup_complete"],
                original_native_error=report["native"]["error"],
                measurement=collection["measurement"], route_evidence=route)


def plot(result, recorded, path):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    figure, axes = plt.subplots(3, 1, figsize=(12, 9), constrained_layout=True)
    figure.suptitle("Actual encoded MIC output / recording: NOT latency or quality PASS")
    bins = [float(np.sqrt(np.mean(recorded[i:i+480].astype(float)**2)))
            for i in range(0, len(recorded), 480)]
    axes[0].plot(np.arange(len(bins))*480, bins)
    axes[0].set(ylabel="Recorder 10ms RMS", xlabel="Independent stored recorder frame")
    for row in result["recorder_packets"]["flagged_packets"]:
        axes[1].axvline(row["stored_start_frame"], color="red", alpha=.5)
    for row in result["recorder_packets"]["position_gaps"]:
        axes[1].scatter(row["stored_start_frame"], row["delta_frames"], marker="x", color="black")
    axes[1].set(ylabel="Original flags / position gaps", xlabel="Stored axis; no clock inference")
    audit = result["full_recording_audit"]
    if audit is None:
        axes[2].text(.05, .5, "All frames unassessed: " + result["analysis_refusal"], transform=axes[2].transAxes)
    else:
        palette = plt.get_cmap("tab20")
        for index, (key, count) in enumerate(audit["counts"].items()):
            if count:
                spans = [(row["start_frame"], row["frames"]) for row in audit["runs"] if row["category"] == key]
                axes[2].broken_barh(spans, (index, .8), facecolors=palette(index), label=f"{key}: {count}")
        axes[2].legend(fontsize=8, loc="upper left", ncol=2)
    axes[2].set(xlabel="Every recorder frame including unsupported, training, silence and tail", yticks=[])
    for axis in axes:
        axis.set_xlim(0, len(recorded))
        axis.grid(alpha=.2)
    figure.savefig(path, dpi=140)
    plt.close(figure)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("input-dir", "pins", "output"):
        parser.add_argument("--"+name, type=Path, required=True)
    parser.add_argument("--microphone-id", required=True)
    args = parser.parse_args(argv)
    endpoint(args.microphone_id)
    require(args.pins.is_file() and args.pins.stat().st_size <= 65536, "pin file cap")
    pins_bytes = args.pins.read_bytes()
    data = pinned_inputs(args.input_dir, parse_json(pins_bytes))
    raw = data["measurement.f32le"]
    require(0 < len(raw) <= MAX_FRAMES*8 and len(raw) % 8 == 0, "recorder frame/byte cap")
    recorded = np.frombuffer(raw, dtype="<f4").reshape(-1, 2)
    metadata = parse_json(data["vad-output.metadata.json"])
    envelope = validate_envelope(data, metadata, args.microphone_id, len(recorded))
    rows = [parse_json(line) for line in data["measurement.packets.jsonl"].splitlines() if line.strip()]
    result = analyze(metadata, data["vad-output.raw.bin"], recorded, rows, args.microphone_id)
    result.update(original=envelope, input_pins=parse_json(pins_bytes),
                  pins_sha256=hashlib.sha256(pins_bytes).hexdigest().upper())
    output, root = args.output.resolve(), args.input_dir.resolve()
    require(output != root and output not in root.parents and root not in output.parents
            and output != args.pins.resolve(), "output must be separate from input tree")
    output.mkdir(parents=False, exist_ok=False)
    with (output / "summary.json").open("x", encoding="ascii") as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
    plot(result, recorded, output / "actual-output-frozen.png")
    # Pin verification is repeated after plotting; no imported data authorizes
    # native origin, and a changed input must not yield a complete CLI receipt.
    require(pinned_inputs(args.input_dir, parse_json(pins_bytes)) == data
            and args.pins.read_bytes() == pins_bytes, "inputs changed during analysis")
    print(json.dumps(dict(complete=True, qualified=False, recorded_frames=len(recorded),
                         unassessed_frames=result["unassessed_frames"],
                         association_anchor=None if result["association"] is None else result["association"]["anchor"],
                         analysis_refusal=result["analysis_refusal"]), allow_nan=False))


if __name__ == "__main__":
    main()
