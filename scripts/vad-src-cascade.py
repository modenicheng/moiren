#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Map both FIR support envelopes through a stamped initial IO/Bus/IO plan.

Three complete, byte-identical archive passes retain only explicit queries and
their bounded dependencies. Imported scalar facts are not native provenance,
an exact cascade phase center, a waveform verdict, or physical latency.
"""
import argparse
from bisect import bisect_left
import hashlib
import importlib.util
import json
from pathlib import Path


SPEC = importlib.util.spec_from_file_location("vad_src_phase", Path(__file__).with_name("vad-src-phase.py"))
PHASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PHASE)
ARCHIVE = PHASE.ARCHIVE
MAX_SUPPORT = 33


def authority(point, route):
    execution = point.get("execution")
    if execution is None:
        return "unknown_execution"
    if execution["plan_revision"] != route["prepared_plan_revision"]:
        return "not_initial_prepared_plan"
    if execution["control_generation"] != 0:
        return "noninitial_controls"
    if point["timeline_start"] + point["engine_frames"] > ARCHIVE.U64_MAX:
        return "timeline_frontier_overflow"
    return None


def track(state, point, kind, route):
    reason = PHASE.track(state, point, kind)
    execution_reason = authority(point, route)
    # A control counter cannot decrease, an actual plan revision cannot be
    # reused, and unknown execution cannot retrospectively prove no control.
    # Keep this trust loss sticky for the owner, including across FIR resets.
    if execution_reason:
        state["execution_lost"] = execution_reason
    execution_reason = state.get("execution_lost")
    if execution_reason:
        # Parameter/plan changes are taint barriers, not just query-local
        # exclusions: the following FIR window may still contain their PCM.
        if reason is None:
            state["barriers"] += 1
            state["segment"] += 1
            state["clean_from"] = None if point["after"] is None else point["after"]["input_frames"]
        reason = execution_reason
    return reason


def support(point, offset):
    lo, hi = PHASE.center_interval(point["before"], point["processing_step"], offset)
    origin = point["before"]["next_input_frame"]
    return (origin + lo.numerator // lo.denominator - 15,
            origin + hi.numerator // hi.denominator + 16)


def refuse(row, reason):
    row.update(status="refused", reason=reason)


def query_rows(request):
    ARCHIVE.shape(request, "schema_version owner_generation queries", "cascade queries")
    ARCHIVE.require(ARCHIVE.integer(request["schema_version"], "query schema") == 1,
                    "unsupported cascade query schema")
    ARCHIVE.integer(request["owner_generation"], "query owner")
    rows = request["queries"]
    ARCHIVE.require(type(rows) is list and 1 <= len(rows) <= PHASE.MAX_QUERIES, "cascade query cap exceeded")
    seen, result = set(), []
    for row in rows:
        ARCHIVE.shape(row, "sequence output_offset", "cascade query")
        seq = ARCHIVE.integer(row["sequence"], "query sequence", 1)
        offset = ARCHIVE.integer(row["output_offset"], "query output offset", 0, PHASE.MAX_FRAMES)
        ARCHIVE.require((seq, offset) not in seen, "duplicate cascade query")
        seen.add((seq, offset))
        result.append({**row, "status": "refused", "reason": "callback_not_observed"})
    return result


class OutputQueries:
    def __init__(self, route, rows):
        self.route, self.rows, self.state = route, {}, PHASE.new_state()
        for row in rows:
            self.rows.setdefault(row["sequence"], []).append(row)

    def observe(self, node, kind, point):
        if node != self.route["sink"] or kind != "virtual_microphone":
            return
        reason = track(self.state, point, kind, self.route)
        for row in self.rows.get(point["sequence"], []):
            local = PHASE.local_result(point, self.state, reason, row["output_offset"])
            if local["status"] != "mapped":
                refuse(row, "microphone_output:" + local["reason"])
                continue
            first, last = support(point, row["output_offset"])
            if not 1 <= last - first + 1 <= MAX_SUPPORT:
                refuse(row, "microphone_support_cap_exceeded")
                continue
            row.update(status="pending", reason=None,
                       microphone_support={"first_input_frame": first, "last_input_frame": last,
                                           "timeline_epoch": point["timeline_epoch"],
                                           "reset_generation": local["reset_generation"],
                                           "segment": local["segment"]},
                       microphone_fir_output_frame=local["fir_output_frame"])


def index_requests(rows, key_frames):
    index = {}
    for row in rows:
        if row["status"] != "pending":
            continue
        key, frames = key_frames(row)
        target = index.setdefault(key, {})
        for frame in frames:
            target.setdefault(frame, {"matches": 0})
    return {key: (sorted(frames), frames) for key, frames in index.items()}


def matching(index, key, start, end):
    if key not in index:
        return
    frames, results = index[key]
    for i in range(bisect_left(frames, start), bisect_left(frames, end)):
        frame = frames[i]
        result = results[frame]
        result["matches"] += 1
        yield frame, result


def mic_scope(value):
    return value["timeline_epoch"], value["reset_generation"], value["segment"]


class PushedInputs:
    def __init__(self, route, rows):
        self.route, self.state = route, PHASE.new_state()
        self.index = index_requests(rows, lambda row: (
            mic_scope(row["microphone_support"]),
            range(row["microphone_support"]["first_input_frame"],
                  row["microphone_support"]["last_input_frame"] + 1)))

    def observe(self, node, kind, point):
        if node != self.route["sink"] or kind != "virtual_microphone":
            return
        reason = track(self.state, point, kind, self.route)
        before, after = point["before"], point["after"]
        if before is None or after is None:
            return
        key = point["timeline_epoch"], before["reset_generation"], self.state["segment"]
        for frame, result in matching(self.index, key, before["input_frames"], after["input_frames"]):
            result.update(reason=reason, sequence=point["sequence"],
                          engine_frame=point["timeline_start"] + frame - before["input_frames"])

    def finish(self, rows):
        for row in rows:
            if row["status"] != "pending":
                continue
            value = row["microphone_support"]
            results = self.index[mic_scope(value)][1]
            dependencies = []
            for frame in range(value["first_input_frame"], value["last_input_frame"] + 1):
                result = results[frame]
                if result["matches"] != 1 or result["reason"]:
                    refuse(row, "microphone_push_missing_conflicting_or_tainted")
                    break
                dependencies.append({"microphone_input_frame": frame, "engine_frame": result["engine_frame"],
                                     "microphone_push_sequence": result["sequence"]})
            if row["status"] == "pending":
                row["engine_dependencies"] = dependencies


class PlaybackOutputs:
    def __init__(self, route, rows):
        self.route, self.state = route, PHASE.new_state()
        self.index = index_requests(rows, lambda row: (
            row["microphone_support"]["timeline_epoch"],
            (dep["engine_frame"] for dep in row["engine_dependencies"])))

    def observe(self, node, kind, point):
        if node != self.route["source"] or kind != "virtual_playback":
            return
        reason = track(self.state, point, kind, self.route)
        start = point["timeline_start"]
        for frame, result in matching(self.index, point["timeline_epoch"], start, start + point["engine_frames"]):
            offset = frame - start
            local = PHASE.local_result(point, self.state, reason, offset)
            result.update(reason=local["reason"])
            if local["status"] == "mapped":
                first, last = support(point, offset)
                result.update(sequence=point["sequence"], output_offset=offset,
                              reset_generation=local["reset_generation"], segment=local["segment"],
                              first_input_frame=first, last_input_frame=last)

    def finish(self, rows):
        for row in rows:
            if row["status"] != "pending":
                continue
            results = self.index[row["microphone_support"]["timeline_epoch"]][1]
            dependencies = [results[dep["engine_frame"]] for dep in row["engine_dependencies"]]
            if any(dep["matches"] != 1 or dep["reason"] for dep in dependencies):
                refuse(row, "playback_output_missing_conflicting_or_tainted")
                continue
            domains = {(dep["reset_generation"], dep["segment"]) for dep in dependencies}
            if len(domains) != 1:
                refuse(row, "playback_support_crosses_reset_or_barrier")
                continue
            for dep, value in zip(row["engine_dependencies"], dependencies):
                dep["playback"] = {key: value[key] for key in ["sequence", "output_offset",
                                   "first_input_frame", "last_input_frame"]}
            reset, segment = next(iter(domains))
            row.update(status="dependency_mapped", reason=None,
                       source_support={"first_input_frame": min(dep["first_input_frame"] for dep in dependencies),
                                       "last_input_frame": max(dep["last_input_frame"] for dep in dependencies),
                                       "reset_generation": reset, "segment": segment,
                                       "coordinate": "playback_successfully_pushed_input"})


def evaluate(summary, chunks_path, request):
    rows = query_rows(request)
    trace, _, _ = ARCHIVE.prepare(summary, ARCHIVE.MAX_BOUNDARIES)
    ARCHIVE.require(trace["owner_generation"] == request["owner_generation"], "cascade owner mismatch")
    route = trace.get("route", {}).get("prepared")
    if route is None or route["schema_version"] != 2:
        for row in rows:
            refuse(row, "unknown_prepared_plan_linkage")
        route = None
    passes = []
    for observer_type in [OutputQueries, PushedInputs, PlaybackOutputs]:
        observer = observer_type(route, rows) if route else None
        with chunks_path.open("rb") as source:
            report = ARCHIVE.verify(summary, source, checkpoint_observer=observer.observe if observer else None)
        ARCHIVE.require(report["scalar_readback_complete"] and report["owner_generation"] == request["owner_generation"],
                        "cascade requires complete scalar readback and the same owner")
        if passes:
            ARCHIVE.require(report["stream"] == passes[0]["stream"], "archive changed between cascade passes")
        passes.append(report)
        if observer and hasattr(observer, "finish"):
            observer.finish(rows)
    # A refusal is not a partial dependency certificate. All three terminal
    # checks must pass before any row is returned or an output is created.
    for row in rows:
        if row["status"] != "dependency_mapped":
            row.pop("engine_dependencies", None)
    return {"schema_version": 1, "mode": "offline_initial_plan_two_fir_dependency_envelopes",
            "qualified": False, "native_execution": False, "physical_delay": "not_evaluated",
            "source_origin": "unknown_not_estimated", "waveform_quality": "not_evaluated",
            "scope": "initial unity IO/Bus/IO execution stamps and full FIR dependency envelopes only; "
                     "no exact cascade center, native packet/source clock, or physical acceptance",
            "owner_generation": request["owner_generation"], "prepared_route": route,
            "all_queries_mapped": all(row["status"] == "dependency_mapped" for row in rows),
            "limits": {"queries": PHASE.MAX_QUERIES, "microphone_support_frames_per_query": MAX_SUPPORT,
                       "archive_passes": 3}, "queries": rows, "archive_passes": passes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["summary", "chunks", "queries", "output"]:
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    try:
        summary_raw = PHASE.read_bounded(args.summary, ARCHIVE.MAX_SUMMARY_BYTES)
        query_raw = PHASE.read_bounded(args.queries, PHASE.MAX_QUERY_BYTES)
        result = evaluate(ARCHIVE.load_json(summary_raw), args.chunks, ARCHIVE.load_json(query_raw))
        result["inputs"] = [{"path": str(path), "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
                            for path, raw in [(args.summary, summary_raw), (args.queries, query_raw)]]
        result["inputs"].append({"path": str(args.chunks), **result["archive_passes"][0]["stream"]})
        with args.output.open("x", encoding="ascii", newline="\n") as output:
            output.write(json.dumps(result, indent=2, allow_nan=False) + "\n")
    except (ValueError, OSError, RecursionError) as error:
        parser.exit(1, f"refused: {error}\n")
    print(f"all_queries_mapped={str(result['all_queries_mapped']).lower()}; qualified=false")
    return 0 if result["all_queries_mapped"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
