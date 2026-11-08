#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Extract the Policy-to-ACX path; stub only host/kernel DDI boundaries."""

import importlib.util
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location("extract", root / "tests/extract-cleanup.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
source = root / "src/Policy.cpp"
text = source.read_text(encoding="utf-8")
parts = re.findall(r"static const GUID \w+Category\s*=\s*\{[^;]+;", text)
for header in ("InterfaceName.h", "EndpointReference.h", "TerminalPolicy.h"):
    parts.append(f'#include "{(root / "src" / header).as_posix()}"')
functions = (
    "SnapshotPolicy", "CreateKey", "SetValue", "StageInterfaceName",
    "StageInterface", "VadReleasePolicy", "PreparePolicy", "VadPreparePolicy",
)
patterns = tuple((name, r"(?:static\s+)?(?:NTSTATUS|void)\s+" + name +
    r"\s*\([^;{}]*\)\s*\{") for name in functions)
parts.append(module.extract(text, source.as_posix(), patterns))
(output / "interface-name-native-production.inc").write_text("\n".join(parts), encoding="utf-8")

parts = []
for relative, functions in (
    ("src/audio/Circuit.cpp", ("VadAudioBuildCircuit",)),
    ("src/audio/CircuitDevice.cpp", ("DeleteUnpublishedCircuit", "Prepare")),
):
    source = root / relative
    patterns = tuple((name, r"(?:static\s+)?(?:NTSTATUS|void)\s+" + name +
        r"\s*\([^;{}]*\)\s*\{") for name in functions)
    parts.append(module.extract(source.read_text(encoding="utf-8"), source.as_posix(), patterns))
parts.append("""
// Test-only bridge to the ACX counters declared after the shared policy fixture.
static void CheckPdoNameCircuitUnpublished()
{
    Check(C.Allocated == 0 && C.Added == 0 && !C.Ready &&
        C.Failure == Audio.Device->Context.PolicyStatus,
        "failed PDO name never allocates or publishes ACX in actual Prepare");
    Check(VadAudioBuildCircuit(&Audio) == STATUS_INVALID_DEVICE_STATE && C.Allocated == 0,
        "direct builder refuses failed PDO name policy before allocation");
}
""")
(output / "publication-reference-production.inc").write_text("\n".join(parts), encoding="utf-8")

source = root / "src/Control.cpp"
patterns = (("Dispatch", r"static\s+NTSTATUS\s+Dispatch\s*\([^;{}]*\)\s*\{"),)
(output / "identity-control-production.inc").write_text(
    module.extract(source.read_text(encoding="utf-8"), source.as_posix(), patterns),
    encoding="utf-8",
)
