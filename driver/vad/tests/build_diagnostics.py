# SPDX-License-Identifier: MIT
"""Exercise diagnostic build isolation without compiling or accessing a device."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import unittest
import xml.etree.ElementTree as ET

PARSER = argparse.ArgumentParser()
PARSER.add_argument("--msbuild", required=True)
ARGS, REMAINING = PARSER.parse_known_args()
ROOT = Path(__file__).resolve().parents[1]
PROJECT = ROOT / "MoirenVad.vcxproj"
PROPERTY = "MoirenVadPlaybackCopyDiagnostic"
NS = {"m": "http://schemas.microsoft.com/developer/msbuild/2003"}


def evaluate(value=None, target=None):
    command = [ARGS.msbuild, str(PROJECT), "/nologo", "/p:Configuration=Debug", "/p:Platform=x64"]
    if value is not None:
        command.append("/p:" + PROPERTY + "=" + value)
    command.append("/t:" + target if target else "-getProperty:" + PROPERTY + ",OutDir,IntDir")
    return subprocess.run(command, text=True, encoding="utf-8", errors="replace",
                          capture_output=True, timeout=60)


class BuildDiagnostics(unittest.TestCase):
    def properties(self, value=None):
        result = evaluate(value)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)["Properties"]

    def test_default_build_disables_content_observation(self):
        values = self.properties()
        self.assertEqual(values[PROPERTY], "0")
        self.assertEqual(values, self.properties("0"))

    def test_enabled_build_has_distinct_object_and_binary_directories(self):
        normal, enabled = self.properties("0"), self.properties("1")
        for key in ("OutDir", "IntDir"):
            self.assertNotEqual(normal[key], enabled[key])
            self.assertIn("playback-copy-1", enabled[key])
        self.assertEqual(enabled[PROPERTY], "1")

    def test_macro_is_forwarded_to_actual_compilation(self):
        project = ET.parse(PROJECT)
        definitions = " ".join(node.text or "" for node in project.findall(".//m:ClCompile/m:PreprocessorDefinitions", NS))
        self.assertIn("MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC=$(" + PROPERTY + ")", definitions)

    def test_invalid_values_are_rejected_before_compile(self):
        for value in ("2", "true", "-1"):
            with self.subTest(value=value):
                result = evaluate(value, "ValidatePlaybackCopyDiagnostic")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(PROPERTY + " must be 0 or 1", result.stdout + result.stderr)

    def test_valid_values_allow_validation_without_building(self):
        for value in ("0", "1"):
            with self.subTest(value=value):
                result = evaluate(value, "ValidatePlaybackCopyDiagnostic")
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0], *REMAINING])
