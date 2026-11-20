# SPDX-License-Identifier: MIT
"""Extract actual pause, PCM cancellation and snapshot publication bodies."""
import importlib.util
from pathlib import Path
import sys

sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location("extract", root / "tests/extract-cleanup.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
header = root / "src/Driver.h"
parts = [module.extract(header.read_text(encoding="utf-8"), header.as_posix(), (
    ("VadPcmDrainGuard", r"class\s+VadPcmDrainGuard\s*\{"),
)) + ";\n"]
for relative, functions in (
    ("src/Pcm.cpp", ("DropCountLocked", "DropCount", "FindIndex", "ReserveRequest",
                     "CancelQueued", "VadPcmPurge", "VadPcmSubmit", "VadPcmDiscontinuity")),
    ("src/Children.cpp", ("VadAudioDescribeStream", "VadAudioReleaseStream")),
    ("src/audio/Streams.cpp", ("PacketAddress", "ZeroPackets", "ClearTimerPacket",
                               "VadAudioStopLocked", "VadAudioStreamPause")),
):
    source = root / relative
    text = source.read_text(encoding="utf-8")
    if relative.endswith("Pcm.cpp"):
        parts.append(module.extract(text, source.as_posix(), (
            ("VadPcmDrainGuard constructor", r"VAD_PAGE\s+VadPcmDrainGuard::VadPcmDrainGuard\s*\([^;{}]*\)\s*:[^{}]*\{"),
            ("VadPcmDrainGuard destructor", r"VAD_PAGE\s+VadPcmDrainGuard::~VadPcmDrainGuard\s*\(\s*\)\s*\{"),
            ("VadPcmDrainGuard Purge", r"VAD_PAGE\s+void\s+VadPcmDrainGuard::Purge\s*\([^;{}]*\)\s*\{"),
        )))
    if relative.endswith("Streams.cpp"):
        parts.append(module.extract(text, source.as_posix(), (
            ("LifecycleGuard", r"class\s+LifecycleGuard\s*\{"),
        )) + ";\n")
    patterns = tuple(
        (name, r"(?:static\s+)?(?:VAD_PAGE\s+)?(?:void|NTSTATUS|PVOID|ULONG)\s+" +
         name + r"\s*\([^;{}]*\)\s*\{")
        for name in functions
    )
    parts.append(module.extract(text, source.as_posix(), patterns))
(output / "pause-fence-production.inc").write_text("\n".join(parts), encoding="utf-8")
