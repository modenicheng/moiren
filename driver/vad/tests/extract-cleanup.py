#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Extract real cleanup bodies without importing WDF/ACX into host tests."""

import argparse
from pathlib import Path
import re


FUNCTIONS = (
    ("RevokeFileOwner", r"static\s+void\s+RevokeFileOwner\s*\(\s*WDFDEVICE\s+device\s*,\s*WDFFILEOBJECT\s+file\s*\)\s*\{"),
    ("TraceFileIrp", r"static\s+NTSTATUS\s+TraceFileIrp\s*\(\s*WDFDEVICE\s+device\s*,\s*PIRP\s+irp\s*\)\s*\{"),
    ("VadFileCleanup", r"void\s+VadFileCleanup\s*\(\s*WDFFILEOBJECT\s+file\s*\)\s*\{"),
)


def extract(source: str, source_name: str) -> str:
    # Ignore braces in comments/literals but preserve offsets and line numbers.
    masked = re.sub(
        r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'',
        lambda match: re.sub(r"[^\n]", " ", match.group()),
        source,
    )
    bodies = ["// Generated from Device.cpp; SPDX-License-Identifier: MS-PL\n"]
    for name, pattern in FUNCTIONS:
        matches = list(re.finditer(pattern, masked))
        if len(matches) != 1:
            raise ValueError(f"expected exactly one {name} definition, found {len(matches)}")
        match = matches[0]
        depth = 1
        end = match.end()
        while depth and end < len(masked):
            if masked[end] == "{":
                depth += 1
            elif masked[end] == "}":
                depth -= 1
            end += 1
        if depth:
            raise ValueError(f"unterminated {name} definition")
        line = source.count("\n", 0, match.start()) + 1
        bodies.append(f'#line {line} "{source_name}"\n{source[match.start():end]}\n')
    return "\n".join(bodies)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        generated = extract(args.source.read_text(encoding="utf-8"), args.source.as_posix())
    except (OSError, ValueError) as error:
        parser.exit(1, f"cleanup extraction failed: {error}\n")
    args.output.write_text(generated, encoding="utf-8")


if __name__ == "__main__":
    main()
