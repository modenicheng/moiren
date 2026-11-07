#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Use the real shared-lock collector and passive trace exporter in host tests."""
import importlib.util
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location('extract', root / 'tests/extract-cleanup.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
parts = []
for relative, functions in (
    ('src/Children.cpp', ('VadRecordRemoval', 'VadAudioRecordRelease')),
    ('src/Control.cpp', ('TracePendingRemovalDiagnostic',)),
):
    source = root / relative
    text = source.read_text(encoding='utf-8')
    if not all(re.search(r'\b' + name + r'\s*\(', text) for name in functions):
        break
    patterns = tuple((name, r'(?:static\s+)?void\s+' + name + r'\s*\([^;{}]*\)\s*\{')
                     for name in functions)
    parts.append(module.extract(text, source.as_posix(), patterns))
else:
    parts.insert(0, '#define HAS_PRODUCTION_REMOVAL_TRACE 1\n')
(output / 'removal-trace-production.inc').write_text('\n'.join(parts), encoding='utf-8')
