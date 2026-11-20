"""Pure overlay of two frozen recorder category ledgers, without attribution.

Each input must already partition the same complete recorder axis. This helper
retains both labels at every original boundary; it does not select a preferred
owner, infer a transition, or establish waveform quality or physical latency.
"""

import argparse
import json
import math
import os
from pathlib import Path
import stat
import sys


MAX_FRAMES = 3840000
MAX_RUNS = 4096
MAX_LABEL_LENGTH = 256
MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_INPUT_BYTES = 16 * 1024 * 1024


def _require(condition, reason):
    if not condition:
        raise ValueError(reason)


def _integer(value, minimum, maximum):
    return type(value) is int and minimum <= value <= maximum


def _label(value):
    return type(value) is str and 1 <= len(value) <= MAX_LABEL_LENGTH


def _validated_runs(report, frames):
    _require(type(report) is dict, "owner report must be a dictionary")
    _require(_integer(report.get("recorded_frames"), frames, frames),
             "owner recorder denominator differs")
    runs, expected = report.get("runs"), report.get("counts")
    _require(type(runs) is list and 1 <= len(runs) <= MAX_RUNS,
             "owner runs must be a bounded nonempty list")
    _require(type(expected) is dict and 1 <= len(expected) <= MAX_RUNS,
             "owner counts must be a bounded nonempty dictionary")
    for category, count in expected.items():
        _require(_label(category) and _integer(count, 0, frames),
                 "owner category/count malformed")
    cursor, actual = 0, {}
    for row in runs:
        _require(type(row) is dict, "owner run must be an object")
        start, width, category = row.get("start_frame"), row.get("frames"), row.get("category")
        _require(_integer(start, cursor, cursor) and _integer(width, 1, frames)
                 and cursor + width <= frames and _label(category),
                 "owner run gap, overlap, invalid category or out-of-bounds frontier")
        cursor += width
        actual[category] = actual.get(category, 0) + width
    _require(cursor == frames, "owner run tail omitted")
    _require(actual == {key: value for key, value in expected.items() if value},
             "owner counts disagree with run partition")
    return runs


def overlay(reports, frames):
    """Return a joint partition and pair totals; never mutate input reports.

    Only ordinary dictionaries are accepted, so a custom mapping cannot emit
    duplicate owners. A duplicate key already lost during JSON decoding cannot
    be recovered here; the caller must reject duplicate keys while decoding.
    Work and output are bounded by the two validated run lists, not audio size.
    """
    _require(_integer(frames, 1, MAX_FRAMES), "recorder frame cap1..3840000")
    _require(type(reports) is dict and len(reports) == 2,
             "exactly two unique owner names required")
    names = list(reports)
    _require(all(_label(name) for name in names) and names[0] != names[1],
             "invalid or duplicate owner name")
    a, b = names
    left, right = (_validated_runs(reports[name], frames) for name in names)
    runs, totals = [], {}
    i = j = cursor = 0
    # Advance at least one input frontier per iteration: at most8191 segments.
    while cursor < frames:
        first, second = left[i], right[j]
        first_end = first["start_frame"] + first["frames"]
        second_end = second["start_frame"] + second["frames"]
        end = min(first_end, second_end)
        pair = first["category"], second["category"]
        width = end - cursor
        runs.append(dict(start_frame=cursor, frames=width, owners={a: pair[0], b: pair[1]}))
        totals[pair] = totals.get(pair, 0) + width
        cursor = end
        if cursor == first_end:
            i += 1
        if cursor == second_end:
            j += 1
    return dict(
        schema_version=1,
        recorded_frames=frames,
        runs=runs,
        partitions=[dict(owners={a: pair[0], b: pair[1]}, frames=count)
                    for pair, count in totals.items()],
        qualified=False,
        waveform_quality_pass=False,
        physical_latency_ms=None,
        scope="Categorical overlay only; original labels retained, no causal or phase attribution",
    )


def _unique_keys(pairs):
    result = {}
    for key, value in pairs:
        _require(key not in result, "duplicate JSON key: " + key)
        result[key] = value
    return result


def _reject_constant(value):
    raise ValueError("nonfinite JSON token: " + value)


def _finite_float(value):
    result = float(value)
    _require(math.isfinite(result), "nonfinite JSON number")
    return result


def _read_report(path, remaining):
    _require(not path.is_symlink() and path.is_file(), "regular JSON input required")
    limit = min(MAX_JSON_BYTES, remaining)
    _require(path.stat().st_size <= limit, "input JSON byte cap exceeded")
    with path.open("rb") as source:
        _require(stat.S_ISREG(os.fstat(source.fileno()).st_mode),
                 "regular JSON input required")
        data = source.read(limit + 1)
    _require(0 < len(data) <= limit, "input JSON empty or grew beyond byte cap")
    report = json.loads(data, object_pairs_hook=_unique_keys,
                        parse_constant=_reject_constant, parse_float=_finite_float)
    return report, len(data)


def _encoded_output(result):
    # Refuse before opening the destination; never leave a cropped JSON result.
    encoded = bytearray()
    encoder = json.JSONEncoder(ensure_ascii=True, allow_nan=False, separators=(",", ":"))
    for part in encoder.iterencode(result):
        block = part.encode("ascii")
        _require(len(encoded) + len(block) <= MAX_JSON_BYTES, "output JSON byte cap exceeded")
        encoded.extend(block)
    return encoded


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner", nargs=2, action="append", required=True,
                        metavar=("LABEL", "FILE"), help="exactly two distinct owner labels and reports")
    parser.add_argument("--frames", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new JSON file; never overwritten")
    args = parser.parse_args(argv)
    try:
        _require(len(args.owner) == 2, "exactly two owner inputs required")
        names = [row[0] for row in args.owner]
        _require(all(_label(name) for name in names) and names[0] != names[1],
                 "two distinct bounded owner labels required")
        _require(_integer(args.frames, 1, MAX_FRAMES), "recorder frame cap1..3840000")
        reports, used = {}, 0
        for name, filename in args.owner:
            report, size = _read_report(Path(filename), MAX_INPUT_BYTES - used)
            used += size
            reports[name] = report
        encoded = _encoded_output(overlay(reports, args.frames))
        with args.output.open("xb") as output:
            output.write(encoded)
        return 0
    except (OSError, ValueError, TypeError, OverflowError, RecursionError) as error:
        print("Refused: " + str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
