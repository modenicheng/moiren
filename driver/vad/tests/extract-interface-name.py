#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reuse the lifecycle extractor to exercise the real interface staging body."""

import importlib.util
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
source, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location(
    "extract", Path(__file__).with_name("extract-cleanup.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
header = source.with_name("InterfaceName.h")
prefix = f'#include "{header.as_posix()}"\n' if header.exists() else ""
text = source.read_text(encoding="utf-8")
reference = source.with_name("EndpointReference.h")
if reference.exists() and re.search(r"\bStageInterfaceName\s*\(", text):
    prefix += f'#include "{reference.as_posix()}"\n'
    # The native suite covers typed-property DDIs. Keep this portable slice
    # focused on the identical bounded REG_SZ staging contract.
    prefix += "#define MOIREN_INTERFACE_NAME_TYPED_STAGE 1\n"
output.write_text(
    prefix + module.extract(
        text, source.as_posix(),
        (("StageInterface", r"static\s+NTSTATUS\s+StageInterface\s*\([^;{}]*\)\s*\{"),),
    ), encoding="utf-8",
)
