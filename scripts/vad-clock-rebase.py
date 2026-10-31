#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Strictly associate decoded EventDeadline rebase and first no-copy witnesses.

Offline diagnostic only: refusal is not absence of a fault, and successful
association is neither complete trace coverage nor accepted audio quality.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re


U64_MAX = (1 << 64) - 1
CLOCK_KEY = "endpoint session generation stream callback_qpc".split()
CAPTURE_KEY = "endpoint session generation gate native".split()
FIELDS = {
    "Clock rebase cause": CLOCK_KEY +
    "reason deadline_frame deadline_qpc late_ticks frames packets completed presented".split(),
    "Clock rebase before": CLOCK_KEY +
    "slot_gen gate open state freq epoch base limit running".split(),
    "Clock rebase after": CLOCK_KEY +
    "slot_gen gate open state freq epoch base limit running".split(),
    "Capture no-copy state": CAPTURE_KEY +
    "state stream_state gate_open audio_ready format channels frames".split(),
    "Capture no-copy timing": CAPTURE_KEY +
    "callback_qpc frequency silence_before silence_after".split(),
    "Capture no-copy event": CAPTURE_KEY +
    "callback_qpc event_gate packet_qpc packets index packet_bytes native_bytes offset".split(),
    "Capture no-copy notify": CAPTURE_KEY +
    "callback_qpc event_gate packets index status".split(),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def frame_qpc(clock, frame):
    delta = max(frame - clock["base"], 0)
    return clock["epoch"] + (delta * clock["freq"] + 47999) // 48000


def fragments(text):
    groups = ({}, {})
    for line_number, line in enumerate(text.splitlines(), 1):
        message = line.split("[vad]", 1)[-1].strip()
        if not message.startswith(("Clock rebase ", "Capture no-copy ")):
            continue
        label = next((name for name in FIELDS if message.startswith(name + " ")), None)
        require(label is not None, f"line {line_number}: unknown witness fragment")
        fields = {}
        for token in message[len(label):].split():
            require(token.count("=") == 1, f"line {line_number}: malformed field")
            name, value = token.split("=", 1)
            require(name not in fields, f"line {line_number}: duplicate field {name}")
            if name != "status":
                require(re.fullmatch(r"[0-9]+", value) is not None,
                        f"line {line_number}: invalid integer {name}")
                value = int(value)
                require(value <= U64_MAX, f"line {line_number}: overflow {name}")
            fields[name] = value
        require(set(fields) == set(FIELDS[label]), f"line {line_number}: unexpected schema")
        is_capture = label.startswith("Capture")
        key_names = CAPTURE_KEY if is_capture else CLOCK_KEY
        key = tuple(fields[name] for name in key_names)
        group = groups[is_capture].setdefault(key, {})
        part = label.rsplit(" ", 1)[-1]
        require(part not in group, f"line {line_number}: duplicate fragment {label}")
        group[part] = fields
    require(groups[0] and groups[1], "missing rebase/no-copy witnesses")
    for group in groups[0].values():
        require(set(group) == {"cause", "before", "after"}, "incomplete rebase triplet")
    for group in groups[1].values():
        require(set(group) == {"state", "timing", "event", "notify"},
                "incomplete no-copy quartet")
    return groups


def associate(clock, capture):
    cause, before, after = (clock[part] for part in ("cause", "before", "after"))
    state, timing, event, notify = (capture[part] for part in
                                  ("state", "timing", "event", "notify"))
    for name in ("endpoint", "session", "generation", "stream", "callback_qpc"):
        require(cause[name] > 0, f"missing positive identity {name}")
    frames, completed, callback = cause["frames"], cause["completed"], cause["callback_qpc"]
    require(cause["reason"] == 1 and cause["packets"] == 2 and
            48 <= frames <= 4800 and completed > 0,
            "requires an EventDeadline event-mode packet shape")
    boundary, next_boundary = completed * frames, (completed + 1) * frames
    require(next_boundary <= U64_MAX and cause["deadline_frame"] == next_boundary,
            "next deadline frame does not match completion")
    for item in (before, after):
        require(item["slot_gen"] == cause["generation"] and item["open"] == 1 and
                item["state"] == 2 and item["running"] == 1 and
                48000 <= item["freq"] <= 100000000000 and
                item["epoch"] > 0 and item["base"] <= item["limit"],
                "requires same-generation Running/open clocks")
    require(after["freq"] == before["freq"] and before["gate"] > 0 and
            after["gate"] == before["gate"] + 1 and after["gate"] < U64_MAX and
            before["limit"] == boundary and after["base"] == boundary and
            after["limit"] == next_boundary and after["epoch"] == callback,
            "invalid atomic rebase transition")
    deadline = frame_qpc(before, next_boundary)
    serviced = frame_qpc(before, boundary)
    require(deadline <= callback and deadline == cause["deadline_qpc"] and
            cause["late_ticks"] == callback - deadline,
            "next deadline or lateness does not match old clock")
    require(state["gate"] == after["gate"] and state["native"] == completed - 1 and
            state["state"] == 1 and state["stream_state"] == 2 and
            state["gate_open"] == state["audio_ready"] == 1 and
            state["format"] == 2 and state["channels"] == 2 and state["frames"] == frames,
            "no-copy does not match rebase packet/Running admission")
    require(timing["frequency"] == before["freq"] and timing["silence_before"] == 0 and
            timing["silence_after"] == 1, "not the first native silence increment")
    for item in (timing, event, notify):
        require(item["callback_qpc"] == callback, "different no-copy callback")
    for item in (event, notify):
        require(item["event_gate"] == after["gate"] and item["packets"] == 2 and
                item["index"] == state["native"] % 2, "different event/notify packet")
    require(event["packet_qpc"] == callback and
            event["packet_bytes"] == event["native_bytes"] == frames * 2 * 4 and
            notify["status"] == "STATUS_SUCCESS", "invalid replacement packet/Notify")
    return {
        **{name: cause[name] for name in CLOCK_KEY},
        "frames": frames, "native_sequence": state["native"],
        "frequency": before["freq"], "gate_before": before["gate"],
        "gate_after": after["gate"], "next_deadline_qpc": deadline,
        "next_deadline_lateness_ticks": callback - deadline,
        "serviced_deadline_qpc_derived": serviced,
        "serviced_deadline_lateness_ticks_derived": callback - serviced,
        "silence_before": timing["silence_before"], "silence_after": timing["silence_after"],
    }


def analyze(text):
    clocks, captures = fragments(text)
    associations = []
    used = set()
    for clock_key, clock in sorted(clocks.items()):
        matches = [(key, group) for key, group in captures.items()
                   if key[:3] == clock_key[:3] and
                   group["timing"]["callback_qpc"] == clock_key[4]]
        require(len(matches) == 1, "missing or ambiguous same-callback no-copy")
        capture_key, capture = matches[0]
        require(capture_key not in used, "ambiguous stream for same no-copy callback")
        associations.append(associate(clock, capture))
        used.add(capture_key)
    require(len(used) == len(captures), "unmatched no-copy witnesses")
    return {"schema_version": 1, "qualified": False, "all_interval_trace_complete": False,
            "meaning": "same-callback native EventDeadline rebase and first silence; "
                       "not scheduler cause, full trace coverage or audio/physical acceptance",
            "associations": associations}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("decoded_trace", type=Path)
    parser.add_argument("--output", type=Path, help="create-new diagnostic JSON receipt")
    args = parser.parse_args()
    try:
        raw = args.decoded_trace.read_bytes()
        report = analyze(raw.decode("utf-8-sig"))
        report["decoded_trace"] = str(args.decoded_trace.resolve())
        report["decoded_trace_sha256"] = hashlib.sha256(raw).hexdigest().upper()
        serialized = json.dumps(report, indent=2) + "\n"
        if args.output:
            with args.output.open("x", encoding="utf-8", newline="\n") as output:
                output.write(serialized)
        print(serialized, end="")
    except (ValueError, OSError, UnicodeError) as error:
        parser.exit(1, f"REFUSED: {error}\n")


if __name__ == "__main__":
    main()
