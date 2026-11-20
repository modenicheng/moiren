#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Plan unchanged full-codebook detector work; never execute or qualify it."""
import argparse
import json
from pathlib import Path


MAX_INPUT_BYTES = 1024 * 1024
FIELDS = {"windows", "marker_frames", "code_count", "max_window_frames",
          "max_windows", "per_child_cap", "aggregate_cap"}


def integer(value, name, minimum=1):
    if type(value) is not int or value < minimum:
        raise ValueError(f"{name} must be an integer >= {minimum}, not bool/float")
    return value


def plan(windows, marker_frames, code_count, max_window_frames, max_windows,
         per_child_cap, aggregate_cap):
    """Keep every original component; batches describe pending analysis only.

    Malformed input raises ValueError before planning. Local size/work refusals
    remain components. Aggregate refusal invalidates the entire dispatch plan.
    """
    for name, value in (("marker_frames", marker_frames), ("code_count", code_count),
                        ("max_window_frames", max_window_frames), ("max_windows", max_windows),
                        ("per_child_cap", per_child_cap), ("aggregate_cap", aggregate_cap)):
        integer(value, name)
    if type(windows) is not list or len(windows) > max_windows:
        raise ValueError("windows must be a list within max_windows")
    previous_end = 0
    validated = []
    # Validate the complete list before allowing a local refusal to hide bad input.
    for window in windows:
        if type(window) is not dict or set(window) != {"start_frame", "frames"}:
            raise ValueError("window requires exactly start_frame and frames")
        start = integer(window["start_frame"], "start_frame", 0)
        frames = integer(window["frames"], "frames")
        if start < previous_end:
            raise ValueError("windows must be ordered and nonoverlapping")
        previous_end = start + frames
        validated.append(dict(start_frame=start, frames=frames))

    components, eligible = [], []
    total = 0
    for index, window in enumerate(validated):
        frames = window["frames"]
        charge = 0
        if frames < marker_frames:
            refusal = "too_short"
        elif frames > max_window_frames:
            refusal = "too_long"
        else:
            charge = (frames - marker_frames + 1) * marker_frames * code_count
            refusal = "per_child_work_cap" if charge > per_child_cap else None
        components.append(dict(component_index=index, window=window,
                               status="refused" if refusal else "pending", refusal=refusal))
        if refusal is None:
            eligible.append((index, charge))
            total += charge

    refusal = "aggregate_work_cap" if total > aggregate_cap else None
    batches = []
    if refusal:
        for index, _ in eligible:
            components[index].update(status="refused", refusal=refusal)
        total = 0
    else:
        for index, charge in eligible:
            if not batches or batches[-1]["charged_correlation_samples"] + charge > per_child_cap:
                batches.append(dict(component_indices=[], windows=[], charged_correlation_samples=0))
            batch = batches[-1]
            batch["component_indices"].append(index)
            batch["windows"].append(dict(validated[index]))
            batch["charged_correlation_samples"] += charge
    refused_count = sum(c["status"] == "refused" for c in components)
    return dict(components=components, batches=batches, charged_correlation_samples=total,
                refused_count=refused_count, analyzed_pending_count=len(components) - refused_count,
                qualified=False, partial_can_pass=False, refusal=refusal)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def invalid_number(value):
    raise ValueError(f"float/noninteger JSON number refused: {value}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        if args.input.resolve() == args.output.resolve():
            raise ValueError("input and output must differ")
        if not args.input.is_file() or args.input.stat().st_size > MAX_INPUT_BYTES:
            raise ValueError("input must be a regular file <= 1 MiB")
        with args.input.open("rb") as source:
            raw = source.read(MAX_INPUT_BYTES + 1)
        if len(raw) > MAX_INPUT_BYTES:
            raise ValueError("input grew beyond 1 MiB")
        parameters = json.loads(raw, object_pairs_hook=unique_object,
                                parse_float=invalid_number, parse_constant=invalid_number)
        if type(parameters) is not dict or set(parameters) != FIELDS:
            raise ValueError("input requires exactly the seven plan arguments")
        result = plan(**parameters)
        # Exclusive creation also refuses existing symlinks and hard-link aliases.
        with args.output.open("x", encoding="utf-8", newline="\n") as output:
            json.dump(result, output, indent=2, allow_nan=False)
            output.write("\n")
    except (ValueError, OSError) as error:
        parser.exit(1, f"refused: {error}\n")


if __name__ == "__main__":
    main()
