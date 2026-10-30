# SPDX-License-Identifier: MIT
"""Compile the actual PCM, ring-copy, position and passive trace bodies on host."""
import importlib.util
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location("extract", root / "tests/extract-cleanup.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
pcm = root / "src/Pcm.cpp"
(output / "pcm-production.inc").write_text(
    module.extract(pcm.read_text(encoding="utf-8"), pcm.as_posix(), module.PCM_FUNCTIONS),
    encoding="utf-8",
)
parts = []
for relative, functions in (
    ("src/audio/Streams.cpp", ("RecordEventCaptureCopy", "TransferTimerSegment", "VadAudioGetPosition")),
    ("src/Control.cpp", ("TracePendingCaptureDiagnostic",)),
):
    source = root / relative
    text = source.read_text(encoding="utf-8")
    patterns = tuple(
        (name, r"(?:static\s+)?(?:void|NTSTATUS)\s+" + name + r"\s*\([^;{}]*\)\s*\{")
        for name in functions if re.search(r"\b" + name + r"\s*\(", text)
    )
    parts.append(module.extract(text, source.as_posix(), patterns))
(output / "capture-boundary-production.inc").write_text("\n".join(parts), encoding="utf-8")
