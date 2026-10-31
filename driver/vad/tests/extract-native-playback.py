# SPDX-License-Identifier: MIT
"""Extract actual timer/event consumption, passive export and CREATE reserve."""
import importlib.util
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
spec = importlib.util.spec_from_file_location("extract", root / "tests/extract-cleanup.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
source = root / "src/Pcm.cpp"
(output / "pcm-production.inc").write_text(
    module.extract(source.read_text(encoding="utf-8"), source.as_posix(), module.PCM_FUNCTIONS),
    encoding="utf-8",
)
parts = []
for relative, names in (
    ("src/audio/Streams.cpp", ("PacketAddress", "NativePlaybackLayoutFor",
        "RecordNativePlaybackLocked", "RecordEventNativePlayback", "TransferTimerSegment")),
    ("src/Control.cpp", ("TracePendingNativePlaybackDiagnostic",)),
):
    source = root / relative
    text = source.read_text(encoding="utf-8")
    patterns = tuple((name, r"(?:static\s+)?(?:void|PVOID|vad::NativePlaybackLayout)\s+" +
        name + r"\s*\([^;{}]*\)\s*\{") for name in names
        if re.search(r"\b" + name + r"\s*\(", text))
    for name, pattern in patterns:
        extracted = module.extract(text, source.as_posix(), ((name, pattern),))
        if name in ("NativePlaybackLayoutFor", "RecordNativePlaybackLocked",
                    "RecordEventNativePlayback", "TracePendingNativePlaybackDiagnostic"):
            extracted = "#if MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC\n" + extracted + "\n#endif\n"
        parts.append(extracted)
    if relative.endswith("Streams.cpp"):
        # The exact event branch includes the real hook, transfer and clearing;
        # no second model of the consumed-buffer ordering is used by the test.
        start = text.index("    auto buffer = PacketAddress(stream, index);")
        end = text.index("    (void)WdfTimerStart(stream->Timer, schedule.Due);", start)
        parts.append("static void ActualEventTransfer(VAD_AUDIO_STREAM* stream, ULONG index, "
            "uint64_t packet, uint64_t now, VAD_PCM_BATCH& batch) {\n"
            "vad::audio::CaptureNotifyObservation notification = {};\n" + text[start:end] +
            "(void)notification;\n}\n")
        # Preserve the real default-off wrapper's rundown/identity/lock behavior.
        pcm = root / "src/Pcm.cpp"
        parts.insert(0, module.extract(pcm.read_text(encoding="utf-8"), pcm.as_posix(), (
            ("VadPcmTransfer", r"void\s+VadPcmTransfer\s*\([^;{}]*\)\s*\{"),
        )))
(output / "native-playback-production.inc").write_text("\n".join(parts), encoding="utf-8")
source = root / "src/Children.cpp"
text = source.read_text(encoding="utf-8")
if "NativePlaybackDiagnostics[reserved].Reset()" in text:
    assert text.count("NativePlaybackDiagnostics[") == 1, "Only CREATE may reset native diagnostic"
start = text.index("    const int reserved = vad::Reserve(")
end = text.index("    KeReleaseSpinLock(&shared->Lock, irql);", start)
(output / "native-playback-reserve.inc").write_text(
    "static int ActualReserve(VAD_SHARED* shared, const MOIREN_VAD_CREATE_REQUEST& request, "
    "uint64_t session, uint64_t& nextId) {\n"
    "struct Root { uint64_t Session; uint64_t& NextId; } owner = {session, nextId};\n"
    "const auto root = &owner;\n" + text[start:end] + "return reserved;\n}\n",
    encoding="utf-8",
)
