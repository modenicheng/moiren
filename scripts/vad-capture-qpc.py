#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Associate two retained capture witnesses with raw WASAPI packet QPC labels.

Offline diagnostic only; never infer a packet start or accept audio quality.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re


MAX_TRACE_BYTES = 4 * 1024 * 1024
MAX_TRACE_LINES = 100000
MAX_LINE_BYTES = 4096
MAX_PACKETS = 100000
MAX_METADATA_BYTES = 16 * 1024 * 1024
MAX_RECORD_BYTES = 1024
MAX_REPORT_BYTES = 16 * 1024 * 1024
U64_MAX = (1 << 64) - 1
U32_MAX = (1 << 32) - 1
IDENTITY = "endpoint session generation".split()
FIELDS = {
    "Capture boundary shape": IDENTITY + "stream kind gate native request qpc packets "
                              "packet_bytes native_bytes ring_offset packet_offset".split(),
    "Capture boundary prefix": IDENTITY + "stream kind bytes source_nonzero "
                               "destination_nonzero equal".split(),
    "Capture packet notify": IDENTITY + "kind gate native completion_qpc packets status".split(),
    "Capture nonzero notify": IDENTITY + "gate native request index copy_qpc "
                              "enter_qpc return_qpc status".split(),
}
PACKET_FIELDS = "packet_sequence flags device_position_frames qpc_100ns frames stored_start_frame".split()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, name, minimum=0, maximum=U64_MAX):
    require(type(value) is int and minimum <= value <= maximum, f"invalid integer {name}")
    return value


def field(item, *names):
    for name in names:
        require(isinstance(item, dict) and name in item, f"missing report field {name}")
        item = item[name]
    return item


def load_json(raw):
    def unique(pairs):
        result = {}
        for name, value in pairs:
            require(name not in result, f"duplicate JSON key {name}")
            result[name] = value
        return result

    def invalid_constant(value):
        raise ValueError(f"invalid JSON constant {value}")

    return json.loads(raw, object_pairs_hook=unique, parse_constant=invalid_constant)


def read_bounded(path, limit):
    # Bound the read itself, including a file growing after any stat check.
    with path.open("rb") as source:
        raw = source.read(limit + 1)
    require(len(raw) <= limit, f"input byte cap exceeded: {path}")
    return raw


def report_context(report):
    require(field(report, "schema_version") == 1 and
            type(report["schema_version"]) is int and field(report, "mode") == "live_vad" and
            field(report, "qualified") is False, "requires unqualified live_vad schema1 report")
    provider = field(report, "native", "vad_microphone_provider")
    require(field(provider, "intent") == "Microphone" and
            field(provider, "diagnostic_provenance") == "Fresh" and
            field(provider, "error") is None, "requires fresh microphone provider")
    snapshot = field(provider, "snapshot")
    for name, expected in {"state": "Present", "stream_state": "Running", "intent": "Microphone",
                           "sample_format": "Float32", "channels": "Stereo"}.items():
        require(field(snapshot, name) == expected, f"unsupported microphone {name}")
    require(field(snapshot, "gate_open") is True and field(snapshot, "audio_ready") is True,
            "requires Running/open audio-ready microphone")
    stamp = field(snapshot, "stamp")
    require(isinstance(stamp, list) and len(stamp) == 5, "invalid native stamp")
    for value in stamp:
        integer(value, "native stamp", 1)
    generation = integer(field(snapshot, "stream_generation"), "stream generation", 1)
    gate = integer(field(snapshot, "gate_epoch"), "gate epoch", 1)
    require(stamp[0] == stamp[2] and stamp[3:] == [generation, gate],
            "snapshot stamp endpoint/stream/gate mismatch")
    # Stamp order is PDO token, session, endpoint generation, stream, gate.
    # This ABI requires equal runtime PDO token/endpoint generation; WPP uses
    # that runtime endpoint id, never the persistent stable GUID.
    identity = (stamp[2], stamp[1], generation)
    frequency = integer(field(provider, "telemetry", "microphone_submission_service",
                              "qpc_frequency_hz"), "QPC frequency", 1)
    require(frequency == 10000000, "only exact 10MHz QPC label comparison is supported")
    frames = integer(field(snapshot, "packet_frames"), "packet frames", 48, 4800)
    collection = field(report, "collection")
    require(field(collection, "qualified") is False and
            type(field(collection, "quality_complete")) is bool, "invalid collection quality scope")
    measurement = field(collection, "measurement")
    capture = field(measurement, "native_capture")
    require(integer(field(capture, "buffer_frames"), "native buffer", 1) == frames,
            "native buffer and snapshot packet shape differ")
    evidence = field(capture, "evidence_metadata")
    for context in (field(measurement, "info"), capture, evidence):
        require(integer(field(context, "sample_rate"), "sample rate", 1) == 48000 and
                integer(field(context, "channels"), "channels", 1) == 2,
                "requires observed 48kHz stereo capture")
    count = integer(field(evidence, "packets"), "evidence packets", 1, MAX_PACKETS)
    stored = integer(field(evidence, "stored_frames"), "stored frames", 1)
    sink = field(measurement, "sink")
    require(integer(field(sink, "complete_packets"), "sink packets", 1) == count and
            integer(field(sink, "written_frames"), "sink frames", 1) == stored,
            "sink/evidence coverage mismatch")
    return identity, gate, frames, count, stored, collection["quality_complete"]


def fragments(decoded, identity):
    require(isinstance(decoded, str) and len(decoded) <= MAX_TRACE_BYTES and
            len(decoded.encode("utf-8")) <= MAX_TRACE_BYTES, "trace byte cap exceeded")
    lines = decoded.splitlines()
    require(len(lines) <= MAX_TRACE_LINES, "trace line count cap exceeded")
    selected = {}
    for number, line in enumerate(lines, 1):
        require(len(line.encode("utf-8")) <= MAX_LINE_BYTES, f"line {number}: trace line cap")
        message = line.split("[vad]", 1)[-1].strip()
        label = next((name for name in FIELDS if message.startswith(name + " ")), None)
        if label is None:
            continue
        values = {}
        for token in message[len(label):].split():
            require(token.count("=") == 1, f"line {number}: malformed field")
            name, value = token.split("=", 1)
            require(name not in values, f"line {number}: duplicate field {name}")
            if name != "status":
                require(re.fullmatch(r"[0-9]{1,20}", value) is not None,
                        f"line {number}: invalid integer {name}")
                value = integer(int(value), name)
            values[name] = value
        require(set(values) == set(FIELDS[label]), f"line {number}: unexpected schema")
        if tuple(values[name] for name in IDENTITY) != identity:
            continue
        kind = values.get("kind", 1)
        require(kind in (0, 1), "unsupported witness kind")
        key = (label, kind)
        require(key not in selected, f"duplicate selected fragment {key}")
        selected[key] = values
    expected = {(label, kind) for label in FIELDS for kind in
                ((0, 1) if label.startswith("Capture boundary") else
                 (0,) if label == "Capture packet notify" else (1,))}
    require(set(selected) == expected, "missing or unsupported selected capture fragments")
    return selected


def packet_summary(packets, frames, count, stored):
    require(isinstance(packets, list) and 1 <= len(packets) <= MAX_PACKETS,
            "packet count cap exceeded or empty packet set")
    require(len(packets) == count, "packet count does not match report")
    result = dict(packet_count=count, stored_frames=stored, flagged_packets=0,
                  timestamp_unknown_packets=0, qpc_intervals_checked=0,
                  qpc_policy_violations=0, qpc_repeat_or_rewind_intervals=0,
                  position_step_mismatches=0)
    previous = None
    for index, packet in enumerate(packets):
        require(isinstance(packet, dict) and set(packet) == set(PACKET_FIELDS),
                f"packet {index}: unexpected schema")
        for name in PACKET_FIELDS:
            integer(packet[name], name, maximum=U32_MAX if name in ("flags", "frames") else U64_MAX)
        require(packet["packet_sequence"] == index + 1 and packet["frames"] == frames and
                packet["stored_start_frame"] == index * frames,
                "requires complete sequence-one fixed-shape stored packet set")
        require(packet["device_position_frames"] + frames <= U64_MAX,
                "native position range overflow")
        result["flagged_packets"] += packet["flags"] != 0
        known = packet["qpc_100ns"] != 0 and packet["flags"] & 4 == 0
        result["timestamp_unknown_packets"] += not known
        if previous is not None:
            result["position_step_mismatches"] += (
                packet["device_position_frames"] != previous["device_position_frames"] + frames)
            if known and previous["qpc_100ns"] != 0 and previous["flags"] & 4 == 0:
                result["qpc_intervals_checked"] += 1
                delta = packet["qpc_100ns"] - previous["qpc_100ns"]
                if delta <= 0:
                    result["qpc_repeat_or_rewind_intervals"] += 1
                else:
                    # Exactly the existing pcm/packets.rs diagnostic policy.
                    expected = frames * 10000000
                    result["qpc_policy_violations"] += (
                        abs(delta * 48000 - expected) > 10000000 + expected // 1000)
        previous = packet
    require(len(packets) * frames == stored, "stored packet coverage does not match report")
    result["qpc_interval_policy"] = "nominal_schedule_rate; tolerance_one_frame_plus_1000ppm; diagnostic_invalidation_not_clock_calibration"
    return result


def associate(parts, packets, identity, gate, frames, kind, stream):
    shape = parts[("Capture boundary shape", kind)]
    prefix = parts[("Capture boundary prefix", kind)]
    require(shape["stream"] == prefix["stream"] == stream > 0 and shape["gate"] == gate,
            "different capture stream or gate")
    native = shape["native"]
    require(native < U64_MAX and shape["request"] == native + 1 and shape["packets"] == 2 and
            shape["packet_bytes"] == shape["native_bytes"] == frames * 8 and
            shape["packet_offset"] == 0 and shape["ring_offset"] <= U32_MAX,
            "unsupported capture packet/request shape")
    require(prefix["bytes"] == 16 and prefix["equal"] == 1 and
            prefix["source_nonzero"] in (0, 1) and
            prefix["source_nonzero"] == prefix["destination_nonzero"], "invalid 16-byte prefix")
    if kind == 0:
        require(native == 0, "first-copy witness is not native sequence zero")
        notify = parts[("Capture packet notify", 0)]
        require(notify["packets"] == 2 and notify["completion_qpc"] == shape["qpc"],
                "first notify does not match copy callback")
    else:
        require(prefix["source_nonzero"] == 1, "nonzero witness prefix is zero")
        notify = parts[("Capture nonzero notify", 1)]
        require(notify["request"] == shape["request"] and notify["index"] == native % 2 and
                0 < shape["qpc"] == notify["copy_qpc"] <= notify["enter_qpc"] <= notify["return_qpc"],
                "nonzero notify does not match copy callback/request/index")
    require(notify["gate"] == gate and notify["native"] == native and
            notify["status"] == "STATUS_SUCCESS", "different gate/native sequence or failed notify")
    matches = [packet for packet in packets if packet["qpc_100ns"] == shape["qpc"]]
    require(shape["qpc"] > 0 and len(matches) == 1, "missing or ambiguous exact packet QPC label")
    packet = matches[0]
    require(packet["flags"] == 0 and packet["packet_sequence"] == native + 1 and
            packet["stored_start_frame"] == native * frames and
            packet["device_position_frames"] == (native + 1) * frames,
            "selected packet flags/sequence/storage/position mismatch")
    result = {**dict(zip(IDENTITY, identity)), "stream": stream, "gate": gate, "kind": kind,
              "native_sequence": native, "request_sequence": shape["request"],
              "callback_qpc": shape["qpc"], "raw_qpc_equals_callback": True,
              **packet, "prefix_bytes": 16, "prefix_equal": True,
              "prefix_nonzero": bool(prefix["source_nonzero"]),
              "packet_bytes": shape["packet_bytes"], "ring_offset_bytes": shape["ring_offset"],
              "packet_offset_bytes": shape["packet_offset"], "notify_status": notify["status"]}
    if kind == 1:
        result.update(notify_enter_qpc=notify["enter_qpc"], notify_return_qpc=notify["return_qpc"],
                      callback_to_notify_ticks=notify["enter_qpc"] - shape["qpc"],
                      notify_call_ticks=notify["return_qpc"] - notify["enter_qpc"])
    return result


def analyze(decoded, packets, report):
    identity, gate, frames, count, stored, complete = report_context(report)
    parts = fragments(decoded, identity)
    summary = packet_summary(packets, frames, count, stored)
    stream = parts[("Capture boundary shape", 0)]["stream"]
    rows = [associate(parts, packets, identity, gate, frames, kind, stream) for kind in (0, 1)]
    require(rows[1]["native_sequence"] >= rows[0]["native_sequence"], "reversed copy witnesses")
    return {"schema_version": 1, "qualified": False, "all_interval_trace_complete": False,
            "packet_start_time": "unknown_not_derived", "qpc_frequency_hz": 10000000,
            "meaning": "two selected raw WASAPI labels equal native callback/completion QPC; "
                       "not packet starts, PCM identity, full trace coverage or audio/physical acceptance",
            "input_collection_quality_complete": complete,
            "associations": rows, "packet_summary": summary}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("decoded_trace", type=Path)
    parser.add_argument("--packets", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="create-new diagnostic receipt")
    args = parser.parse_args()
    try:
        trace = read_bounded(args.decoded_trace, MAX_TRACE_BYTES)
        metadata = read_bounded(args.packets, MAX_METADATA_BYTES)
        native = read_bounded(args.report, MAX_REPORT_BYTES)
        records = metadata.splitlines(keepends=True)
        require(1 <= len(records) <= MAX_PACKETS, "packet record count cap exceeded or empty input")
        packets = []
        for record in records:
            require(len(record) <= MAX_RECORD_BYTES and record.endswith(b"\n"),
                    "oversized or unterminated packet record")
            packets.append(load_json(record.decode("utf-8")))
        result = analyze(trace.decode("utf-8-sig"), packets, load_json(native.decode("utf-8-sig")))
        result["inputs"] = [{"path": str(path.resolve()), "bytes": len(raw),
                             "sha256": hashlib.sha256(raw).hexdigest().upper()}
                            for path, raw in ((args.decoded_trace, trace), (args.packets, metadata),
                                              (args.report, native))]
        serialized = json.dumps(result, indent=2, allow_nan=False) + "\n"
        with args.output.open("x", encoding="utf-8", newline="\n") as output:
            output.write(serialized)
        print(serialized, end="")
    except (ValueError, OSError, UnicodeError, RecursionError) as error:
        parser.exit(1, f"REFUSED: {error}\n")


if __name__ == "__main__":
    main()
