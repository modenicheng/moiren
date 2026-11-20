#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exact model-coordinate spacing diagnostics, never audio acceptance."""
import copy
from fractions import Fraction
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "vad-indexed-spacing.py"
MODULE = None
if SCRIPT.exists():
    SPEC = importlib.util.spec_from_file_location("vad_indexed_spacing", SCRIPT)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)


def fixture(intervals=None, indices=None):
    intervals = intervals or [[1000, 1001], [49000, 49001], [97000, 97001]]
    indices = indices or list(range(len(intervals)))
    count = len(intervals)
    source = {"schema_version": 1, "schedule_id": "spacing-fixture",
              "waveform_version": "moiren-marker-xorshift32-indexed-v2",
              "sample_rate": 48000, "channels": 2, "marker_frames": 1024,
              "marker_amplitude": 0.025, "total_source_frames": count * 48000,
              "starts": [24000 + i * 48000 for i in range(count)]}
    rows = [{"recording_frame_interval": interval, "source_index": identity,
             "status": "Unique", "window_index": window}
            for window, (interval, identity) in enumerate(zip(intervals, indices))]
    reports = [{"correlation_samples": 2000,
                "model": "affine_fir_synthetic_v1_not_exact_cascade",
                "packet_quality": "not_evaluated_no_complete_set_claim",
                "source_origin": "unknown_not_estimated",
                "results": [{"best_score": 0.9, **{k: v for k, v in row.items()
                                               if k != "window_index"}}]}
               for row in rows]
    batch = {"charged_correlation_samples": count * 2000,
             "max_aggregate_correlation_samples": 7000000000,
             "global_identities": rows, "model_identity_set_complete": True,
             "model_source_unchanged": True, "model_window_reports": reports,
             "native_execution": False, "packet_quality": "not_evaluated_preserve_raw_refusal",
             "qualified": False, "scope": "explicit frozen model windows only",
             "source_origin": "unknown_not_estimated"}
    return source, batch


class IndexedSpacing(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "missing exact model-interval feasibility diagnostic")

    def test_constant_mapping_is_compatible_without_origin_or_quality(self):
        source, batch = fixture()
        before = copy.deepcopy((source, batch))
        result = MODULE.analyze(source, batch)
        self.assertTrue(result["constant_affine_compatible"])
        self.assertFalse(result["qualified"])
        self.assertFalse(result["native_execution"])
        self.assertEqual(result["source_origin"], "unknown_not_estimated")
        self.assertEqual(result["packet_quality"], batch["packet_quality"])
        self.assertEqual(result["slope_bounds"]["lower"]["fraction"], [95999, 96000])
        self.assertEqual(result["slope_bounds"]["upper"]["fraction"], [96001, 96000])
        self.assertEqual((source, batch), before)
        for key in ["delay_frames", "origin", "clock_drift_ppm", "quality_passed"]:
            self.assertNotIn(key, result)

    def test_nonconstant_mapping_retains_disjoint_exact_constraint_witnesses(self):
        source, batch = fixture([[1000, 1001], [48900, 48901], [96900, 96901]])
        result = MODULE.analyze(source, batch)
        self.assertFalse(result["constant_affine_compatible"])
        lower = result["slope_bounds"]["lower"]
        upper = result["slope_bounds"]["upper"]
        self.assertEqual([lower["from_source_index"], lower["to_source_index"]], [1, 2])
        self.assertEqual([upper["from_source_index"], upper["to_source_index"]], [0, 1])
        self.assertEqual(lower["fraction"], [47999, 48000])
        self.assertEqual(upper["fraction"], [15967, 16000])
        self.assertEqual(result["adjacent_spacing"][0]["recording_delta_interval"], [47899, 47901])
        self.assertEqual(result["adjacent_spacing"][0]["spacing_residual_interval"], [-101, -99])

    def test_closed_intervals_allow_exact_feasible_boundary(self):
        source, batch = fixture([[1000, 1000], [49000, 49000], [97000, 97000]])
        result = MODULE.analyze(source, batch)
        self.assertTrue(result["constant_affine_compatible"])
        self.assertEqual(result["slope_bounds"]["lower"]["fraction"], [1, 1])
        self.assertEqual(result["slope_bounds"]["upper"]["fraction"], [1, 1])

    def test_large_recorder_translation_preserves_all_spacing_exactly(self):
        source, batch = fixture()
        before = MODULE.analyze(source, batch)
        for row, report in zip(batch["global_identities"], batch["model_window_reports"]):
            interval = [x + (1 << 63) for x in row["recording_frame_interval"]]
            row["recording_frame_interval"] = interval
            report["results"][0]["recording_frame_interval"] = interval
        after = MODULE.analyze(source, batch)
        self.assertEqual(after["slope_bounds"], before["slope_bounds"])
        self.assertEqual(after["adjacent_spacing"], before["adjacent_spacing"])

    def test_model_window_order_never_assigns_source_identity(self):
        source, batch = fixture([[97000, 97001], [1000, 1001], [49000, 49001]], [2, 0, 1])
        result = MODULE.analyze(source, batch)
        self.assertTrue(result["constant_affine_compatible"])
        self.assertEqual([r["window_index"] for r in result["source_ordered_intervals"]], [1, 2, 0])

    def test_reversed_source_mapping_requires_nonpositive_slope_and_refuses_compatibility(self):
        source, batch = fixture([[97000, 97001], [49000, 49001], [1000, 1001]])
        result = MODULE.analyze(source, batch)
        self.assertFalse(result["constant_affine_compatible"])
        self.assertEqual(result["reason"], "no_positive_constant_affine_mapping")

    def test_missing_duplicate_and_unidentified_codes_never_form_selected_subset(self):
        source, original = fixture()
        for mutation in ["missing", "duplicate", "unidentified", "incomplete"]:
            batch = copy.deepcopy(original)
            if mutation == "missing":
                batch["global_identities"].pop()
            elif mutation == "duplicate":
                batch["global_identities"][1]["source_index"] = 0
            elif mutation == "unidentified":
                batch["global_identities"][1]["status"] = "Unidentified"
            else:
                batch["model_identity_set_complete"] = False
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                MODULE.analyze(source, batch)

    def test_global_and_individual_reports_must_agree(self):
        source, original = fixture()
        for key, value in [("source_index", 2), ("recording_frame_interval", [1002, 1003]),
                           ("recording_frame_interval", [1000.0, 1001]),
                           ("status", "Ambiguous"), ("best_score", 0.69)]:
            batch = copy.deepcopy(original)
            batch["model_window_reports"][0]["results"][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                MODULE.analyze(source, batch)

    def test_malformed_interval_bounds_and_types_refuse(self):
        source, original = fixture()
        for interval in [[1], [2, 1], [-1, 0], [True, 1], [1.0, 2], [0, 3], [0, 1 << 64]]:
            batch = copy.deepcopy(original)
            batch["global_identities"][0]["recording_frame_interval"] = interval
            with self.subTest(interval=interval), self.assertRaises(ValueError):
                MODULE.analyze(source, batch)

    def test_budget_and_schema_refuse_before_interval_analysis(self):
        source, original = fixture()
        for key, value in [("charged_correlation_samples", 1),
                           ("max_aggregate_correlation_samples", 5999),
                           ("qualified", True), ("native_execution", True),
                           ("model_source_unchanged", False),
                           ("source_origin", "fitted"), ("extra", 0)]:
            batch = copy.deepcopy(original)
            batch[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                MODULE.analyze(source, batch)

    def test_source_schedule_is_exact_versioned_nonoverlapping_and_bounded(self):
        original, batch = fixture()
        for key, value in [("schema_version", True), ("sample_rate", 44100),
                           ("channels", 1), ("waveform_version", "v1"),
                           ("starts", [24000, 24000, 120000]),
                           ("starts", [24000, 24001, 120000]),
                           ("starts", [24000, 72000]), ("marker_amplitude", float("nan")),
                           ("total_source_frames", 120000)]:
            source = copy.deepcopy(original)
            source[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                MODULE.analyze(source, batch)

    def test_json_duplicate_keys_nonfinite_and_bounded_reads_refuse(self):
        for raw in ['{"a":0,"a":1}', '{"a":NaN}', '{"a":Infinity}']:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                MODULE.load_json(raw)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input"
            path.write_bytes(b"12345")
            self.assertEqual(MODULE.read_bounded(path, 5), b"12345")
            with self.assertRaises(ValueError):
                MODULE.read_bounded(path, 4)

    def test_exact_feasibility_agrees_with_direct_intercept_intersections(self):
        source, _ = fixture()
        for values in itertools.product(range(4), repeat=3):
            bounds = [[1000 + value, 1001 + value] for value in values]
            _, batch = fixture(bounds)
            # Independent witness check: try rational arrangement vertices
            # and interiors, intersecting intercept intervals directly.
            slopes = {Fraction(0), Fraction(1)}
            for i in range(3):
                for j in range(i + 1, 3):
                    for a, b in itertools.product(bounds[i], bounds[j]):
                        slopes.add(Fraction(b - a, source["starts"][j] - source["starts"][i]))
            vertices = sorted(slopes)
            slopes.update((a + b) / 2 for a, b in zip(vertices, vertices[1:]))
            direct = any(
                slope > 0 and max(Fraction(bounds[i][0]) - slope * source["starts"][i]
                                 for i in range(3)) <=
                min(Fraction(bounds[i][1]) - slope * source["starts"][i] for i in range(3))
                for slope in slopes)
            self.assertEqual(MODULE.analyze(source, batch)["constant_affine_compatible"],
                             direct, values)

    def test_cli_pins_inputs_creates_once_and_incompatibility_is_diagnostic_success(self):
        source, batch = fixture([[1000, 1001], [48900, 48901], [96900, 96901]])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            s, b, out = root / "source.json", root / "batch.json", root / "result.json"
            s.write_text(json.dumps(source), encoding="utf-8")
            b.write_text(json.dumps(batch), encoding="utf-8")
            argv = [sys.executable, str(SCRIPT), "--source", str(s),
                    "--model-report", str(b), "--output", str(out)]
            first = subprocess.run(argv, capture_output=True, timeout=10)
            self.assertEqual(first.returncode, 0, first.stderr)
            result = json.loads(out.read_text(encoding="utf-8"))
            self.assertFalse(result["constant_affine_compatible"])
            self.assertEqual(len(result["inputs"]), 2)
            for path, pin in zip([s, b], result["inputs"]):
                raw = path.read_bytes()
                self.assertEqual(pin, {"path": str(path), "bytes": len(raw),
                                       "sha256": hashlib.sha256(raw).hexdigest()})
            original = out.read_bytes()
            second = subprocess.run(argv, capture_output=True, timeout=10)
            self.assertNotEqual(second.returncode, 0)
            self.assertEqual(out.read_bytes(), original)

    def test_cli_invalid_evidence_creates_no_success_receipt(self):
        source, batch = fixture()
        batch["global_identities"][0]["source_index"] = None
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            s, b, out = root / "source.json", root / "batch.json", root / "result.json"
            s.write_text(json.dumps(source), encoding="utf-8")
            b.write_text(json.dumps(batch), encoding="utf-8")
            run = subprocess.run([sys.executable, str(SCRIPT), "--source", str(s),
                                  "--model-report", str(b), "--output", str(out)],
                                 capture_output=True, timeout=10)
            self.assertNotEqual(run.returncode, 0)
            self.assertFalse(out.exists())

    def test_window_schema_scores_and_scope_refuse_malformed_evidence(self):
        source, original = fixture()
        for key, value in [("model", "other"), ("source_origin", "fitted"),
                           ("packet_quality", "accepted"), ("correlation_samples", True),
                           ("correlation_samples", 256000001), ("extra", 0)]:
            batch = copy.deepcopy(original)
            batch["model_window_reports"][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                MODULE.analyze(source, batch)
        for value in [True, float("nan"), float("inf"), 2, 10 ** 400]:
            batch = copy.deepcopy(original)
            batch["model_window_reports"][0]["results"][0]["best_score"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                MODULE.analyze(source, batch)


if __name__ == "__main__":
    unittest.main()
