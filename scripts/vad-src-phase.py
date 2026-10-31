#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Query reset/fault-aware FIR center intervals in a verified scalar archive.

Coordinates describe successful FIR outputs versus pushed FIR input. They are
not native source, packet or recorder clocks. No waveform verdict is issued.
"""
import argparse
from fractions import Fraction
import hashlib
import importlib.util
import json
from pathlib import Path


SPEC = importlib.util.spec_from_file_location("vad_src_archive", Path(__file__).with_name("vad-src-archive.py"))
ARCHIVE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ARCHIVE)
MAX_QUERY_BYTES = 256 * 1024
MAX_QUERIES = 256
MAX_FRAMES = 1 << 20
ROUNDING_PER_OUTPUT = Fraction(1, 1 << 32)


def pair(value):
    return [value.numerator, value.denominator]


def phase(cursor):
    return Fraction(cursor["fraction"])


def center_interval(before, step, count):
    # Never cast the integer origin to float. Fraction keeps the serialized
    # binary64 phase/step exact, including corrections at large reset origins.
    offset = phase(before) + count * Fraction(step)
    error = count * ROUNDING_PER_OUTPUT
    return offset - error, offset + error


def defect(point, previous, kind):
    before, after = point["before"], point["after"]
    if point["sequence"] != (previous["sequence"] + 1 if previous else 1):
        return "callback_gap"
    if previous and (point["timeline_epoch"] != previous["timeline_epoch"] or
                     point["engine_sample_rate"] != previous["engine_sample_rate"] or
                     point["timeline_start"] != previous["timeline_start"] + previous["engine_frames"]):
        return "timeline_barrier"
    if before is None or after is None:
        return "unknown_cursor"
    if before["saturated"] or after["saturated"]:
        return "saturated_cursor"
    if previous and previous["after"] != before:
        return "cursor_discontinuity"
    if point["boundary_epoch_before"] != point["boundary_epoch_after"] or \
            point["boundary_epoch_after"] != point["requested_epoch_after"] or \
            (previous and previous["boundary_epoch_after"] != point["boundary_epoch_before"]):
        return "boundary_epoch_barrier"
    if before["reset_generation"] != after["reset_generation"]:
        return "reset_generation_barrier"
    if point["processing_step"] is None:
        return "no_processing_step"
    if point["xruns"] or point["discontinuity"] or point["muted_frames"] or \
            point["transferred_frames"] != point["engine_frames"]:
        return "fault_mute_or_short_transfer"
    count = after["output_frames"] - before["output_frames"]
    pushed = after["input_frames"] - before["input_frames"]
    step = point["processing_step"]
    if not (0 <= count <= MAX_FRAMES and 0 <= pushed <= MAX_FRAMES and
            point["engine_frames"] <= MAX_FRAMES and 0.02 <= step <= 50 and after["step"] == step):
        return "unsupported_or_regressed_frontier"
    nominal = (48000 / point["engine_sample_rate"] if kind == "virtual_playback"
               else point["engine_sample_rate"] / 48000)
    # The production servo clips at +/-2000 ppm. A small fixed binary64
    # product allowance is arithmetic slack, not an empirically fitted gate.
    if not nominal * (1 - 0.002) - 1e-12 <= step <= nominal * (1 + 0.002) + 1e-12:
        return "step_outside_direction_rate_authority"
    if kind == "virtual_playback":
        valid = count == point["engine_frames"] and pushed == point["carrier_frames"]
    else:
        valid = pushed == point["engine_frames"] and count == point["carrier_frames"]
    if not valid:
        return "carrier_or_engine_frontier_mismatch"
    lo, hi = center_interval(before, step, count)
    actual = after["next_input_frame"] - before["next_input_frame"] + phase(after)
    if not lo <= actual <= hi:
        return "after_center_outside_rounding_bound"
    if count:
        # Every interpolation kernel touches floor(center)-15 .. +16. A
        # center after the pushed frontier cannot represent a successful FIR.
        _, last = center_interval(before, step, count - 1)
        if before["next_input_frame"] + last.numerator // last.denominator + 16 >= after["input_frames"]:
            return "insufficient_fir_lookahead"
    return None


class PhaseObserver:
    """Bounded per-boundary state plus at most 256 explicit query results."""

    def __init__(self, request):
        ARCHIVE.shape(request, "schema_version owner_generation queries", "phase queries")
        ARCHIVE.require(ARCHIVE.integer(request["schema_version"], "query schema") == 1,
                        "unsupported phase query schema")
        self.owner = ARCHIVE.integer(request["owner_generation"], "query owner")
        rows = request["queries"]
        ARCHIVE.require(type(rows) is list and 1 <= len(rows) <= MAX_QUERIES, "phase query cap exceeded")
        self.queries = {}
        self.by_callback = {}
        self.order = []
        self.states = {}
        self.results = {}
        for row in rows:
            ARCHIVE.shape(row, "node kind sequence output_offset", "phase query")
            node, kind = ARCHIVE.identity(row)
            seq = ARCHIVE.integer(row["sequence"], "query sequence", 1)
            offset = ARCHIVE.integer(row["output_offset"], "query output offset", 0, MAX_FRAMES)
            key = (node, kind, seq, offset)
            ARCHIVE.require(key not in self.queries, "duplicate phase query")
            self.queries[key] = row.copy()
            self.by_callback.setdefault(key[:3], []).append(key)
            self.order.append(key)

    def observe(self, node, kind, point):
        key = (node, kind)
        if key not in self.states:
            ARCHIVE.require(len(self.states) < ARCHIVE.MAX_BOUNDARIES, "phase boundary cap exceeded")
            self.states[key] = {"previous": None, "clean_from": None, "barriers": 0,
                                "checked_callbacks": 0, "segment": 0, "saturated": False}
        state = self.states[key]
        before, after = point["before"], point["after"]
        reason = defect(point, state["previous"], kind)
        state["saturated"] |= any(cursor is not None and cursor["saturated"]
                                  for cursor in [before, after])
        if state["saturated"]:
            reason = "saturated_cursor"
        state["checked_callbacks"] += 1
        if reason:
            state["barriers"] += 1
            state["segment"] += 1
            state["clean_from"] = None if after is None else after["input_frames"]
        elif state["clean_from"] is None:
            state["clean_from"] = before["input_frames"]
        for query_key in self.by_callback.get((node, kind, point["sequence"]), []):
            offset = query_key[3]
            result = {**self.queries[query_key], "status": "refused", "reason": reason,
                      "center": None}
            if reason is None:
                if offset >= after["output_frames"] - before["output_frames"]:
                    result["reason"] = "output_offset_out_of_range"
                else:
                    lo, hi = center_interval(before, point["processing_step"], offset)
                    support_lo = before["next_input_frame"] + lo.numerator // lo.denominator - 15
                    support_hi = before["next_input_frame"] + hi.numerator // hi.denominator + 16
                    if support_lo < state["clean_from"] or support_hi >= after["input_frames"]:
                        result["reason"] = "tainted_or_startup_support"
                    else:
                        result.update(status="mapped", reason=None,
                                      segment=state["segment"], reset_generation=before["reset_generation"],
                                      fir_output_frame=before["output_frames"] + offset,
                                      center={"input_frame": before["next_input_frame"],
                                              "relative_interval": [pair(lo), pair(hi)]})
            self.results[query_key] = result
        # Hold one point only. Archive.verify owns its strict sequence/schema
        # checks; results must not be released until its terminal checks pass.
        state["previous"] = point

    def report(self, archive_report):
        ARCHIVE.require(archive_report["scalar_readback_complete"] is True and
                        archive_report["owner_generation"] == self.owner,
                        "phase queries require complete scalar readback and the same owner")
        rows = [self.results.get(key, {**self.queries[key], "status": "refused",
                                      "reason": "callback_not_observed", "center": None})
                for key in self.order]
        return {"schema_version": 1, "mode": "offline_callback_local_fir_phase_intervals",
                "qualified": False, "native_execution": False,
                "source_origin": "unknown_not_estimated", "waveform_quality": "not_evaluated",
                "physical_delay": "not_evaluated", "owner_generation": self.owner,
                "scope": "reset-scoped successful FIR output to pushed-input center only; "
                         "no native packet/source/recorder correspondence or waveform acceptance",
                "all_queries_mapped": all(row["status"] == "mapped" for row in rows),
                "rounding_per_output_input_frames": pair(ROUNDING_PER_OUTPUT),
                "queries": rows,
                "boundaries": [{"node": node, "kind": kind,
                                "checked_callbacks": state["checked_callbacks"],
                                "barrier_callbacks": state["barriers"]}
                               for (node, kind), state in self.states.items()]}


def read_bounded(path, limit):
    with path.open("rb") as source:
        raw = source.read(limit + 1)
    ARCHIVE.require(len(raw) <= limit, "phase input byte cap exceeded")
    return raw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["summary", "chunks", "queries", "output"]:
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    try:
        summary_raw = read_bounded(args.summary, ARCHIVE.MAX_SUMMARY_BYTES)
        query_raw = read_bounded(args.queries, MAX_QUERY_BYTES)
        observer = PhaseObserver(ARCHIVE.load_json(query_raw))
        with args.chunks.open("rb") as source:
            archive_report = ARCHIVE.verify(ARCHIVE.load_json(summary_raw), source,
                                           checkpoint_observer=observer.observe)
        result = observer.report(archive_report)
        result["archive_readback"] = archive_report
        result["inputs"] = [{"path": str(path), "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
                            for path, raw in [(args.summary, summary_raw), (args.queries, query_raw)]]
        result["inputs"].append({"path": str(args.chunks), **archive_report["stream"]})
        with args.output.open("x", encoding="ascii", newline="\n") as output:
            output.write(json.dumps(result, indent=2, allow_nan=False) + "\n")
    except (ValueError, OSError, RecursionError) as error:
        parser.exit(1, f"refused: {error}\n")
    print(f"all_queries_mapped={str(result['all_queries_mapped']).lower()}; qualified=false")
    return 0 if result["all_queries_mapped"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
