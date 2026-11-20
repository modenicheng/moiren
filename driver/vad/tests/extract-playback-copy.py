# SPDX-License-Identifier: MIT
"""Extract the actual accepted PCM transfer and passive playback exporter."""
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
source = root / "src/Control.cpp"
text = source.read_text(encoding="utf-8")
name = "TracePendingPlaybackCopyDiagnostic"
generated = ""
if re.search(r"\b" + name + r"\s*\(", text):
    generated = module.extract(text, source.as_posix(), (
        (name, r"static\s+void\s+" + name + r"\s*\([^;{}]*\)\s*\{"),
    ))
(output / "playback-copy-production.inc").write_text(generated, encoding="utf-8")
source = root / "src/Children.cpp"
text = source.read_text(encoding="utf-8")
start = text.index("    const int reserved = vad::Reserve(")
end = text.index("    KeReleaseSpinLock(&shared->Lock, irql);", start)
reserve = text[start:end]
assert reserve.count("PlaybackCopyDiagnostics[reserved].Reset()") == 1
assert text.count("PlaybackCopyDiagnostics[") == 1, "Only CREATE may reset diagnostic"
(output / "playback-copy-reserve.inc").write_text(
    "// Actual CREATE reserve block, called under the fixture Shared lock.\n"
    "static int ReservePlaybackFixture(VAD_SHARED* shared, const MOIREN_VAD_CREATE_REQUEST& request, "
    "uint64_t session, uint64_t& nextId) {\n"
    "struct Root { uint64_t Session; uint64_t& NextId; } owner = {session, nextId};\n"
    "const auto root = &owner;\n" + reserve + "return reserved;\n}\n",
    encoding="utf-8",
)
