#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compile the actual hardware-release body against host-only DDI boundaries."""
import importlib.util
from pathlib import Path
import sys

sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location('extract', root / 'tests/extract-cleanup.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
source = root / 'src/audio/CircuitDevice.cpp'
functions = (
    ('DeleteUnpublishedCircuit', r'static\s+void\s+DeleteUnpublishedCircuit\s*\([^;{}]*\)\s*\{'),
    ('Release', r'static\s+NTSTATUS\s+Release\s*\([^;{}]*\)\s*\{'),
)
(output / 'hardware-release-production.inc').write_text(
    module.extract(source.read_text(encoding='utf-8'), source.as_posix(), functions),
    encoding='utf-8',
)
