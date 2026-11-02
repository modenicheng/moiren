#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Component planning tests; no detector, PCM, or native execution."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "indexed-support-plan.py"
MODULE = None
if SCRIPT.exists():
    SPEC = importlib.util.spec_from_file_location("indexed_support_plan", SCRIPT)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)


def arguments(windows=None, **overrides):
    result = dict(windows=windows if windows is not None else [dict(start_frame=0, frames=1152)],
                  marker_frames=1024, code_count=39, max_window_frames=96000,
                  max_windows=64, per_child_cap=800000000, aggregate_cap=7000000000)
    result.update(overrides)
    return result


class SupportPlan(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "missing indexed support component planner")

    def test_mixed_short_first_preserves_all_twelve_original_windows(self):
        windows = [dict(start_frame=1344, frames=673)] + [
            dict(start_frame=48871 + i * 48000, frames=1183 + i % 2) for i in range(11)]
        before = copy.deepcopy(windows)
        result = MODULE.plan(**arguments(windows))
        self.assertEqual(result["refused_count"], 1)
        self.assertEqual(result["analyzed_pending_count"], 11)
        self.assertEqual([c["component_index"] for c in result["components"]], list(range(12)))
        self.assertEqual([c["window"] for c in result["components"]], windows)
        self.assertEqual(result["components"][0]["refusal"], "too_short")
        self.assertEqual([i for b in result["batches"] for i in b["component_indices"]],
                         list(range(1, 12)))
        self.assertEqual(windows, before)
        result["components"][1]["window"]["frames"] = 1
        self.assertEqual(windows, before)

    def test_all_refused_retains_short_long_and_work_cap_reasons(self):
        windows = [dict(start_frame=0, frames=673), dict(start_frame=1000, frames=96001),
                   dict(start_frame=100000, frames=1152)]
        result = MODULE.plan(**arguments(windows, per_child_cap=1))
        self.assertEqual([c["refusal"] for c in result["components"]],
                         ["too_short", "too_long", "per_child_work_cap"])
        self.assertTrue(all(c["status"] == "refused" for c in result["components"]))
        self.assertEqual((result["refused_count"], result["analyzed_pending_count"]), (3, 0))
        self.assertEqual(result["batches"], [])
        self.assertEqual(result["charged_correlation_samples"], 0)

    def test_work_formula_batch_boundary_and_aggregate_equality(self):
        windows = [dict(start_frame=i * 10, frames=5) for i in range(3)]
        result = MODULE.plan(**arguments(windows, marker_frames=4, code_count=3,
                                        per_child_cap=48, aggregate_cap=72))
        self.assertEqual([b["component_indices"] for b in result["batches"]], [[0, 1], [2]])
        self.assertEqual([b["charged_correlation_samples"] for b in result["batches"]],
                         [48, 24])
        self.assertEqual([w for b in result["batches"] for w in b["windows"]], windows)
        self.assertEqual(result["charged_correlation_samples"], 72)
        self.assertTrue(all(c["status"] == "pending" and c["refusal"] is None
                            for c in result["components"]))

    def test_aggregate_overflow_refuses_entire_plan_without_skipping_windows(self):
        windows = [dict(start_frame=0, frames=1), dict(start_frame=10, frames=5),
                   dict(start_frame=20, frames=5)]
        result = MODULE.plan(**arguments(windows, marker_frames=4, code_count=3,
                                        per_child_cap=24, aggregate_cap=47))
        self.assertEqual(result["refusal"], "aggregate_work_cap")
        self.assertEqual([c["window"] for c in result["components"]], windows)
        self.assertEqual([c["refusal"] for c in result["components"]],
                         ["too_short", "aggregate_work_cap", "aggregate_work_cap"])
        self.assertEqual((result["refused_count"], result["analyzed_pending_count"]), (3, 0))
        self.assertEqual(result["batches"], [])
        self.assertEqual(result["charged_correlation_samples"], 0)

    def test_order_overlap_and_count_are_validated_before_local_refusal(self):
        for windows in [[dict(start_frame=20, frames=1), dict(start_frame=10, frames=1)],
                        [dict(start_frame=0, frames=673), dict(start_frame=672, frames=1)]]:
            with self.subTest(windows=windows), self.assertRaises(ValueError):
                MODULE.plan(**arguments(windows))
        with self.assertRaises(ValueError):
            MODULE.plan(**arguments([dict(start_frame=i, frames=1) for i in range(65)]))
        result = MODULE.plan(**arguments([dict(start_frame=0, frames=1024),
                                         dict(start_frame=1024, frames=1024)]))
        self.assertEqual(result["analyzed_pending_count"], 2)

    def test_strict_integer_and_window_schema_validation(self):
        for key in ("marker_frames", "code_count", "max_window_frames", "max_windows",
                    "per_child_cap", "aggregate_cap"):
            for bad in (True, False, 1.0, "1", 0, -1):
                with self.subTest(key=key, bad=bad), self.assertRaises(ValueError):
                    MODULE.plan(**arguments(**{key: bad}))
        for key in ("start_frame", "frames"):
            for bad in (True, False, 1.0, "1", -1, None):
                window = dict(start_frame=0, frames=1152)
                window[key] = bad
                with self.subTest(window=window), self.assertRaises(ValueError):
                    MODULE.plan(**arguments([window]))
        for windows in ([dict(start_frame=0, frames=0)], [dict(start_frame=0)],
                        [dict(start_frame=0, frames=1152, extra=1)], {}, None, [None]):
            with self.subTest(windows=windows), self.assertRaises(ValueError):
                MODULE.plan(**dict(arguments(), windows=windows))

    def test_no_pass_or_completed_analysis_claim_even_when_all_eligible(self):
        for windows in ([], [dict(start_frame=0, frames=1152)]):
            result = MODULE.plan(**arguments(windows))
            self.assertIs(result["qualified"], False)
            self.assertIs(result["partial_can_pass"], False)
            self.assertNotIn("analyzed_count", result)
            self.assertEqual(len(result["components"]), len(windows))

    def test_cli_fresh_output_only_and_input_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            source, output = Path(directory) / "input.json", Path(directory) / "plan.json"
            raw = json.dumps(arguments()).encode()
            source.write_bytes(raw)
            def run(destination):
                return subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--input",
                                       str(source), "--output", str(destination)],
                                      capture_output=True, timeout=10)
            self.assertEqual(run(output).returncode, 0)
            original_output = output.read_bytes()
            self.assertFalse(json.loads(original_output)["partial_can_pass"])
            self.assertNotEqual(run(output).returncode, 0)
            self.assertEqual(output.read_bytes(), original_output)
            self.assertNotEqual(run(source).returncode, 0)
            self.assertEqual(source.read_bytes(), raw)

    def test_cli_bounded_strict_json_refuses_before_creating_output(self):
        valid = json.dumps(arguments())
        for raw in (b" " * (1024 * 1024 + 1), b'{"windows": [], "windows": []}',
                    json.dumps(arguments(extra=1)).encode(),
                    valid.replace('"code_count": 39', '"code_count": 39.0').encode(),
                    valid.replace('"code_count": 39', '"code_count": true').encode()):
            with self.subTest(raw=raw[:70]), tempfile.TemporaryDirectory() as directory:
                source, output = Path(directory) / "input.json", Path(directory) / "plan.json"
                source.write_bytes(raw)
                result = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--input",
                                         str(source), "--output", str(output)],
                                        capture_output=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
