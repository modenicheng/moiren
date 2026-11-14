#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Validate tracepdb output, including warnings that do not affect exit code."""
import argparse
from pathlib import Path
import re


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("tracepdb_log", type=Path)
    args = parser.parse_args()
    log = args.tracepdb_log.read_text(encoding="utf-8", errors="replace")
    if re.search(r"warning|too complex|omitt|error", log, re.IGNORECASE):
        parser.exit(1, "FAIL: tracepdb reported a warning/error or omitted an event\n")
    formats = "\n".join(path.read_text(encoding="utf-8", errors="replace")
                        for path in args.directory.glob("*.tmf"))
    for label, fields in (
        ("Mic unserved callback", ("endpoint", "session", "generation", "stream", "state_qpc",
                                   "kind", "gate", "flags", "dpc_qpc", "deadline_qpc",
                                   "retrieve", "calls", "freq")),
        ("Mic unserved write_pre", ("endpoint", "session", "generation", "stream", "state_qpc",
                                    "kind", "gate", "has_write", "request", "header_qpc",
                                    "pre_forward_qpc")),
    ):
        if label not in formats:
            parser.exit(1, f"FAIL: missing decodable {label} fragment\n")
        # Inspect the individual event, not an unrelated declaration elsewhere.
        start = formats.rfind("#typev", 0, formats.index(label))
        end = formats.find("}", formats.index(label))
        event = formats[start:end]
        for index, field in enumerate(fields, 10):
            if not re.search(rf"\b{field}=%{index}!", event) or not re.search(
                    rf",\s*Item\w+\s*--\s*{index}\s*$", event, re.MULTILINE):
                parser.exit(1, f"FAIL: {label} has no decoded field/descriptor {field}\n")
        if len(re.findall(r",\s*Item\w+\s*--", event)) != len(fields) or len(fields) > 14:
            parser.exit(1, f"FAIL: {label} payload count is not bounded/exact\n")
    print("PASS: tracepdb warning-free; unserved callback/write_pre TMF with 13/11 decoded fields")


if __name__ == "__main__":
    main()
