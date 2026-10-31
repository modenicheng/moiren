#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Independently read back bounded SRC scalar archives; never qualify audio.

Use explicit local paths. The imported chunks_path is a label, not a file to
open. One capped record plus per-boundary counters is retained during readback.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re


U64_MAX = (1 << 64) - 1
MAX_SUMMARY_BYTES = 192 * 1024 * 1024
MAX_RECORD_BYTES = 8 * 1024 * 1024
MAX_STREAM_BYTES = 32 * 1024 * 1024 * 1024
MAX_BOUNDARIES = 256
REFUSALS = {"missing_callbacks", "archive_limit", "history_unavailable", "open_writer",
            "invalid_history", "no_callbacks"}
LIMITS = {"pending_checkpoints": 4096, "queued_chunks": 4,
          "record_bytes": MAX_RECORD_BYTES, "stream_bytes": MAX_STREAM_BYTES}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, name, minimum=0, maximum=U64_MAX):
    require(type(value) is int and minimum <= value <= maximum, f"invalid integer {name}")
    return value


def boolean(value, name):
    require(type(value) is bool, f"invalid boolean {name}")


def shape(value, names, name):
    require(type(value) is dict and set(value) == set(names.split()), f"invalid {name} schema")


def finite(value, name, minimum=0, inclusive=False):
    try:
        valid = (type(value) in (int, float) and math.isfinite(value) and
                 (value >= minimum if inclusive else value > minimum))
    except OverflowError:
        valid = False
    require(valid, f"invalid finite {name}")


def load_json(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key {key}")
            result[key] = value
        return result

    def number(text):
        value = float(text)
        require(math.isfinite(value), "nonfinite JSON number")
        return value

    def invalid(text):
        raise ValueError(f"invalid JSON constant {text}")

    try:
        return json.loads(raw, object_pairs_hook=unique, parse_float=number, parse_constant=invalid)
    except (UnicodeError, RecursionError) as error:
        raise ValueError("invalid JSON encoding or nesting") from error


def identity(boundary):
    node, kind = boundary["node"], boundary["kind"]
    require(type(node) is str and 0 < len(node) <= 256, "invalid boundary node")
    require(type(kind) is str and kind in {"virtual_playback", "virtual_microphone"},
            "unsupported boundary kind")
    return node, kind


def validate_cursor(value):
    if value is None:
        return
    shape(value, "reset_generation input_frames output_frames next_input_frame fraction step saturated",
          "cursor")
    for name in ["reset_generation", "input_frames", "output_frames", "next_input_frame"]:
        integer(value[name], name)
    finite(value["fraction"], "cursor fraction", inclusive=True)
    require(value["fraction"] < 1, "cursor fraction outside [0,1)")
    finite(value["step"], "cursor step")
    boolean(value["saturated"], "cursor saturated")


def validate_point(point):
    shape(point, "schema_version sequence timeline_epoch timeline_start engine_frames "
          "engine_sample_rate boundary_epoch_before boundary_epoch_after requested_epoch_after "
          "before after processing_step carrier_frames muted_frames transferred_frames discontinuity xruns",
          "checkpoint")
    require(integer(point["schema_version"], "checkpoint schema") == 1, "unsupported checkpoint schema")
    for name in ["sequence", "engine_frames"]:
        integer(point[name], name, 1)
    integer(point["engine_sample_rate"], "Engine rate", 8000, 384000)
    for name in ["timeline_epoch", "timeline_start", "boundary_epoch_before", "boundary_epoch_after",
                 "requested_epoch_after", "carrier_frames", "muted_frames", "transferred_frames", "xruns"]:
        integer(point[name], name)
    boolean(point["discontinuity"], "checkpoint discontinuity")
    validate_cursor(point["before"])
    validate_cursor(point["after"])
    if point["processing_step"] is not None:
        finite(point["processing_step"], "processing step")


def validate_route(trace, states):
    if trace["schema_version"] == 2:
        return
    route = trace["route"]
    shape(route, "prepared invalidated", "route")
    boolean(route["invalidated"], "route invalidated")
    prepared = route["prepared"]
    if prepared is None:
        return
    shape(prepared, "schema_version prepared_graph_revision source bus sink source_endpoint sink_endpoint "
          "channels engine_sample_rate engine_block_frames", "prepared route")
    require(integer(prepared["schema_version"], "route schema") == 1, "unsupported route schema")
    integer(prepared["prepared_graph_revision"], "route revision")
    for name in ["source", "bus", "sink"]:
        require(type(prepared[name]) is str and 0 < len(prepared[name]) <= 256, "invalid route node")
    require(len({prepared[name] for name in ["source", "bus", "sink"]}) == 3, "duplicate route node")
    require(set(states) == {(prepared["source"], "virtual_playback"),
                            (prepared["sink"], "virtual_microphone")}, "route boundary linkage mismatch")
    for name in ["source_endpoint", "sink_endpoint"]:
        require(type(prepared[name]) is str and re.fullmatch(r"[0-9a-f]{32}", prepared[name]) is not None,
                "invalid route endpoint")
    require(prepared["source_endpoint"] != prepared["sink_endpoint"], "duplicate route endpoint")
    integer(prepared["channels"], "route channels", 1, 2)
    require(integer(prepared["engine_sample_rate"], "route rate") in [44100, 48000, 88200, 96000],
            "unsupported route rate")
    integer(prepared["engine_block_frames"], "route block", 1, 4096)


def route_report(trace):
    route = trace.get("route", {"prepared": None, "invalidated": False})
    status = ("unknown" if route["prepared"] is None else "invalidated_preparation"
              if route["invalidated"] else "imported_preparation_only")
    return {**route, "status": status,
            "scope": "initial exact IO/Bus/IO preparation only, not active plan or controls; "
                     "imported labels cannot authenticate native origin or compose FIR stages"}


def prepare(summary, boundary_limit):
    shape(summary, "schema_version trace stream chunks_path history_complete qualified limits", "summary")
    require(integer(summary["schema_version"], "summary schema") == 2, "unsupported summary schema")
    require(summary["qualified"] is False, "summary must not qualify audio")
    boolean(summary["history_complete"], "summary history_complete")
    require(type(summary["chunks_path"]) is str and 0 < len(summary["chunks_path"]) <= 32768,
            "invalid chunks path label")
    shape(summary["limits"], "pending_checkpoints queued_chunks record_bytes stream_bytes", "limits")
    for name, value in LIMITS.items():
        require(integer(summary["limits"][name], name) == value, "unsupported writer limits")
    stream = summary["stream"]
    shape(stream, "complete chunks checkpoints bytes failure failed_chunks", "writer outcome")
    require(stream["complete"] is True and stream["failure"] is None and stream["failed_chunks"] == [],
            "incomplete writer outcome; retain original failed evidence")
    for name in ["chunks", "checkpoints", "bytes"]:
        integer(stream[name], "stream " + name)
    trace = summary["trace"]
    require(type(trace) is dict, "invalid trace schema")
    version = integer(trace.get("schema_version"), "trace schema")
    require(version in [2, 3], "unsupported trace schema")
    shape(trace, "schema_version owner_generation checkpoint_limit retained_checkpoints drained_checkpoints "
          "drained_chunks finalized history_complete qualified refusal boundaries" +
          (" route" if version == 3 else ""), "trace")
    integer(trace["owner_generation"], "owner generation")
    integer(trace["checkpoint_limit"], "checkpoint limit", 1, 131072)
    require(trace["checkpoint_limit"] == LIMITS["pending_checkpoints"], "pending limit mismatch")
    require(trace["finalized"] is True and trace["qualified"] is False, "unfinalized/qualified trace")
    require(integer(trace["retained_checkpoints"], "retained checkpoints") == 0,
            "trace has undrained checkpoints")
    for name in ["drained_checkpoints", "drained_chunks"]:
        integer(trace[name], name)
    boolean(trace["history_complete"], "trace history_complete")
    require(trace["refusal"] is None or
            (type(trace["refusal"]) is str and trace["refusal"] in REFUSALS), "unknown trace refusal")
    require(type(trace["boundaries"]) is list and 1 <= len(trace["boundaries"]) <= boundary_limit,
            "boundary count outside cap")
    states = {}
    nodes = set()
    for boundary in trace["boundaries"]:
        shape(boundary, "node kind total_callbacks missing_callbacks unretained_callbacks unavailable_polls "
              "writer_finished drained_checkpoints checkpoints", "trace boundary")
        key = identity(boundary)
        require(key[0] not in nodes, "duplicate trace boundary node")
        nodes.add(key[0])
        for name in ["total_callbacks", "missing_callbacks", "unretained_callbacks", "unavailable_polls",
                     "drained_checkpoints"]:
            integer(boundary[name], name)
        boolean(boundary["writer_finished"], "writer_finished")
        require(boundary["checkpoints"] == [], "trace boundary has undrained checkpoints")
        states[key] = {"node": key[0], "kind": key[1], "observed_checkpoints": 0,
                       "last_sequence": 0, "callbacks_with_xruns": 0,
                       "callbacks_with_discontinuity": 0, "callbacks_with_mute": 0,
                       "callbacks_with_unknown_or_saturated_cursor": 0}
    validate_route(trace, states)
    return trace, stream, states


def accept_chunk(chunk, trace, states, expected_sequence, checkpoint_observer):
    shape(chunk, "schema_version owner_generation sequence boundaries", "chunk")
    require(integer(chunk["schema_version"], "chunk schema") == 1, "unsupported chunk schema")
    require(integer(chunk["owner_generation"], "chunk owner") == trace["owner_generation"],
            "chunk owner changed")
    require(integer(chunk["sequence"], "chunk sequence", 1) == expected_sequence, "chunk sequence mismatch")
    boundaries = chunk["boundaries"]
    require(type(boundaries) is list and 1 <= len(boundaries) <= len(states), "invalid chunk boundaries")
    seen = set()
    count = 0
    for boundary in boundaries:
        shape(boundary, "node kind checkpoints", "chunk boundary")
        key = identity(boundary)
        require(key in states and key not in seen, "unknown or duplicate chunk boundary")
        seen.add(key)
        points = boundary["checkpoints"]
        require(type(points) is list and 1 <= len(points) <= trace["checkpoint_limit"],
                "empty or oversized checkpoint list")
        state = states[key]
        count += len(points)
        require(count <= trace["checkpoint_limit"], "chunk checkpoint cap exceeded")
        for point in points:
            validate_point(point)
            route = trace.get("route", {}).get("prepared")
            if route is not None:
                require(point["engine_sample_rate"] == route["engine_sample_rate"] and
                        point["engine_frames"] <= route["engine_block_frames"],
                        "callback outside prepared route rate/block")
            require(point["sequence"] > state["last_sequence"], "callback order regressed or duplicated")
            state["last_sequence"] = point["sequence"]
            state["observed_checkpoints"] += 1
            state["callbacks_with_xruns"] += point["xruns"] > 0
            state["callbacks_with_discontinuity"] += point["discontinuity"]
            state["callbacks_with_mute"] += point["muted_frames"] > 0
            state["callbacks_with_unknown_or_saturated_cursor"] += any(
                cursor is None or cursor["saturated"] for cursor in [point["before"], point["after"]])
            if checkpoint_observer is not None:
                checkpoint_observer(key[0], key[1], point)
    return count


def verify(summary, source, *, record_limit=MAX_RECORD_BYTES, stream_limit=MAX_STREAM_BYTES,
           boundary_limit=MAX_BOUNDARIES, checkpoint_observer=None):
    integer(record_limit, "record cap", 1, MAX_RECORD_BYTES)
    integer(stream_limit, "stream cap", 1, MAX_STREAM_BYTES)
    integer(boundary_limit, "boundary cap", 1, MAX_BOUNDARIES)
    trace, stream, states = prepare(summary, boundary_limit)
    chunks = checkpoints = byte_count = 0
    digest = hashlib.sha256()
    terminal = False
    while True:
        # readline(size) bounds both no-newline inputs and ordinary JSONL. The
        # independent byte/work limit is charged before parsing each record.
        raw = source.readline(min(record_limit + 1, stream_limit - byte_count + 1))
        if not raw:
            break
        byte_count += len(raw)
        require(byte_count <= stream_limit, "stream byte cap exceeded")
        require(len(raw) <= record_limit, "record byte cap exceeded")
        require(not terminal, "data after terminal record")
        require(raw.endswith(b"\n"), "truncated record without newline")
        digest.update(raw)
        record = load_json(raw)
        require(type(record) is dict, "invalid stream record")
        if record.get("record") == "chunk":
            shape(record, "record chunk", "chunk record")
            checkpoints += accept_chunk(record["chunk"], trace, states, chunks + 1, checkpoint_observer)
            chunks += 1
        else:
            shape(record, "record summary", "terminal record")
            # Canonical JSON distinguishes bool from integer and float from
            # integer; Python dictionary equality alone would equate True/1.
            require(record["record"] == "final" and
                    json.dumps(record["summary"], sort_keys=True, allow_nan=False) ==
                    json.dumps(trace, sort_keys=True, allow_nan=False), "terminal trace mismatch")
            terminal = True
    require(terminal, "missing terminal record")
    require(byte_count == stream["bytes"], "writer byte count mismatch")
    require(chunks == stream["chunks"] == trace["drained_chunks"], "chunk count mismatch")
    require(checkpoints == stream["checkpoints"] == trace["drained_checkpoints"], "checkpoint count mismatch")
    covered = True
    for boundary in trace["boundaries"]:
        state = states[identity(boundary)]
        count, total = state["observed_checkpoints"], boundary["total_callbacks"]
        require(count == boundary["drained_checkpoints"] and state["last_sequence"] <= total,
                "boundary count or final sequence mismatch")
        require(total - count == boundary["missing_callbacks"] + boundary["unretained_callbacks"],
                "boundary gap accounting mismatch")
        state["unobserved_callbacks"] = total - count
        state["writer_finished"] = boundary["writer_finished"]
        for name in ["total_callbacks", "drained_checkpoints", "missing_callbacks",
                     "unretained_callbacks", "unavailable_polls"]:
            state[name] = boundary[name]
        covered &= boundary["writer_finished"] and total > 0 and total == count
    complete = bool(covered and trace["refusal"] is None)
    require(trace["history_complete"] == complete and summary["history_complete"] == complete,
            "claimed history completeness contradicts readback")
    require(complete or trace["refusal"] is not None, "incomplete coverage without refusal")
    return {"schema_version": 1, "mode": "offline_src_scalar_archive_readback",
            "scalar_readback_complete": complete, "qualified": False, "native_execution": False,
            "source_origin": "unknown_not_estimated", "phase_mapping": "not_evaluated",
            "waveform_quality": "not_evaluated", "physical_delay": "not_evaluated",
            "scope": "serialized scalar coverage only; imported data cannot issue native provenance, "
                     "source/recorder correspondence, clean waveform or physical acceptance",
            "owner_generation": trace["owner_generation"], "trace_refusal": trace["refusal"],
            "route": route_report(trace),
            "stream": {"bytes": byte_count, "sha256": digest.hexdigest(), "chunks": chunks,
                       "checkpoints": checkpoints}, "boundaries": list(states.values()),
            "reader_limits": {"record_bytes": record_limit, "stream_bytes": stream_limit,
                              "boundaries": boundary_limit}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", required=True, type=Path)
    parser.add_argument("--chunks", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        with args.summary.open("rb") as source:
            raw = source.read(MAX_SUMMARY_BYTES + 1)
        require(len(raw) <= MAX_SUMMARY_BYTES, "summary byte cap exceeded")
        summary = load_json(raw)
        with args.chunks.open("rb") as source:
            result = verify(summary, source)
        result["inputs"] = [
            {"path": str(args.summary), "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()},
            {"path": str(args.chunks), **result["stream"]}]
        with args.output.open("x", encoding="ascii", newline="\n") as output:
            output.write(json.dumps(result, indent=2, allow_nan=False) + "\n")
    except (ValueError, OSError, RecursionError) as error:
        parser.exit(1, f"refused: {error}\n")
    print(f"scalar_readback_complete={str(result['scalar_readback_complete']).lower()}; qualified=false")
    return 0 if result["scalar_readback_complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
