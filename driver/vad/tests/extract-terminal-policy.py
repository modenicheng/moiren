#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compile the production ACX builder against host-only DDI boundaries."""

import importlib.util
from pathlib import Path
import sys

sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location("extract", root / "tests/extract-cleanup.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
source = root / "src/audio/Circuit.cpp"
header = root / "src/TerminalPolicy.h"
prefix = f'#include "{header.as_posix()}"\n' if header.exists() else ""
(output / "terminal-policy-production.inc").write_text(
    prefix + module.extract(source.read_text(encoding="utf-8"), source.as_posix(), (
        ("VadAudioBuildCircuit", r"NTSTATUS\s+VadAudioBuildCircuit\s*\([^;{}]*\)\s*\{"),
    )), encoding="utf-8",
)
