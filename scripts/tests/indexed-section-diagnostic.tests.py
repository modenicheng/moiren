#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Fixed-candidate arithmetic tests, never detector acceptance or native IO."""
from array import array
import copy
import importlib.util
import json
import math
from pathlib import Path
import struct
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "indexed-section-diagnostic.py"
MODULE = None
if SCRIPT.exists():
    SPEC = importlib.util.spec_from_file_location("indexed_section_diagnostic", SCRIPT)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


class SectionDiagnostic(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "missing fixed-candidate section diagnostic")

    def test_exact_bipolar_golden_prefix_and_f32_amplitude(self):
        signs = "--+-+-+-++-++--+++-+-++-------++"
        expected = [f32(0.025) * (1 if sign == "+" else -1) for sign in signs]
        self.assertEqual(MODULE.marker_code(0, 32, 0.025), expected)
        self.assertEqual(len(expected), 32)

    def test_marker_segments_are_contiguous_not_reseeded(self):
        first = MODULE.marker_code(0, 32, 0.1)
        second = MODULE.marker_code(1, 32, 0.1)
        self.assertEqual(first + second, MODULE.marker_code(0, 64, 0.1))
        signs = "+-+-+-+---+--+-+++-++-+-++--++-+"
        self.assertEqual(second, [f32(0.1) * (1 if s == "+" else -1) for s in signs])
        self.assertNotEqual(first, second)

    def test_perfect_gain_dc_and_inversion_use_supplied_polarity(self):
        reference = MODULE.marker_code(3, 1024, 0.025)
        for gain, dc, inverted in [(1, 0, False), (0.4, 0.1, False), (-0.5, 0.1, True)]:
            with self.subTest(gain=gain):
                samples = [f32(gain * value + dc) for value in reference]
                report = MODULE.inspect_fixed(reference, samples, inverted)
                self.assertAlmostEqual(report["full"]["confidence"], 1.0, places=12)
                self.assertAlmostEqual(report["full"]["aligned_correlation"], 1.0, places=12)
                self.assertEqual(report["full"]["signed_correlation"] < 0, inverted)
                self.assertTrue(report["sections_match"])
        wrong = MODULE.inspect_fixed(reference, [-v for v in reference], False)
        self.assertAlmostEqual(wrong["full"]["confidence"], 1.0)
        self.assertFalse(wrong["sections_match"])
        self.assertTrue(all(not s["matches"] for s in wrong["sections"]))

    def test_erased_quarter_high_global_confidence_still_refuses(self):
        reference = [-0.25, 0.25] * 64
        samples = reference[:]
        samples[32:64] = [0.0] * 32
        report = MODULE.inspect_fixed(reference, samples, False)
        self.assertAlmostEqual(report["full"]["confidence"], math.sqrt(0.75))
        self.assertGreater(report["full"]["confidence"], 0.7)
        self.assertFalse(report["sections_match"])
        self.assertIsNone(report["sections"][1]["signed_correlation"])
        self.assertEqual([s["matches"] for s in report["sections"]], [True, False, True, True])

    def test_quarters_retain_global_mean_and_last_takes_remainder(self):
        reference = [-0.5] * 8 + [0.0] * 8 + [0.25] * 8 + [0.5] * 11
        before = copy.deepcopy(reference)
        report = MODULE.inspect_fixed(reference, tuple(reference), False)
        self.assertEqual([(s["start_frame"], s["frames"]) for s in report["sections"]],
                         [(0, 8), (8, 8), (16, 8), (24, 11)])
        mean = 3.5 / 35
        self.assertAlmostEqual(report["sections"][0]["reference_rms"], 0.5 + mean)
        self.assertTrue(report["sections_match"], "local quarter centering would erase all energy")
        self.assertEqual(reference, before)
        for length in (32, 33, 34, 48000):
            with self.subTest(length=length):
                report = MODULE.inspect_fixed(array("f", [-0.25, 0.25] * (length // 2)
                                                     + ([0.25] if length % 2 else [])),
                                              [-0.25, 0.25] * (length // 2)
                                              + ([0.25] if length % 2 else []), False)
                self.assertEqual(sum(s["frames"] for s in report["sections"]), length)
                self.assertTrue(report["sections_match"])

    def test_ordered_f64_means_and_moments_after_f32_input_rounding(self):
        reference = [0.75] + [2.0 ** -30] * 31 + [-0.75]
        samples = [v * 0.25 + (i % 3) * 0.01 for i, v in enumerate(reference)]
        left, right = [f32(v) for v in reference], [f32(v) for v in samples]
        a_total = b_total = 0.0
        for a, b in zip(left, right):
            a_total += a
            b_total += b
        a_mean, b_mean = a_total / len(left), b_total / len(right)
        dot = a_energy = b_energy = 0.0
        for a, b in zip(left, right):
            a, b = a - a_mean, b - b_mean
            dot += a * b
            a_energy += a * a
            b_energy += b * b
        report = MODULE.inspect_fixed(reference, samples, False)
        full = report["full"]
        self.assertEqual(full["signed_correlation"], dot / math.sqrt(a_energy * b_energy))
        self.assertEqual(full["reference_rms"], math.sqrt(a_energy / len(left)))
        self.assertEqual(full["recording_rms"], math.sqrt(b_energy / len(right)))

    def test_zero_energy_is_null_finite_json_never_identity_or_qualification(self):
        for reference, samples in [([0.0] * 32, [0.0] * 32),
                                   ([0.25] * 32, [-0.25, 0.25] * 16)]:
            report = MODULE.inspect_fixed(reference, samples, False)
            self.assertIsNone(report["full"]["confidence"])
            self.assertIsNone(report["full"]["signed_correlation"])
            self.assertIsNone(report["full"]["aligned_correlation"])
            self.assertFalse(report["sections_match"])
            self.assertTrue(all(s["signed_correlation"] is None for s in report["sections"]))
            self.assertEqual(json.loads(json.dumps(report, allow_nan=False)), report)
        perfect = MODULE.inspect_fixed([-0.25, 0.25] * 16, [-0.25, 0.25] * 16, False)
        self.assertEqual(set(perfect), {"full", "sections", "sections_match", "qualified",
                                       "identity_accepted", "scope"})
        self.assertIs(perfect["qualified"], False)
        self.assertIs(perfect["identity_accepted"], False)
        self.assertEqual(perfect["scope"], "fixed_candidate_diagnostic_no_search")

    def test_threshold_rms_floor_and_inclusive_correlation(self):
        reference = [-0.25, 0.25] * 16
        report = MODULE.inspect_fixed(reference, reference, False, 1, 0.25)
        self.assertTrue(report["sections_match"])
        report = MODULE.inspect_fixed(reference, reference, False, 0.7, 0.251)
        self.assertFalse(report["sections_match"])
        self.assertTrue(all(not s["matches"] for s in report["sections"]))

    def test_full_confidence_gate_refuses_even_when_all_quarters_match(self):
        reference = [-0.25, 0.25] * 16
        samples = [-0.75, 0.75] * 4 + [-0.001, 0.001] * 12
        report = MODULE.inspect_fixed(reference, samples, False)
        self.assertLess(report["full"]["confidence"], 0.7)
        self.assertTrue(all(s["matches"] for s in report["sections"]))
        self.assertFalse(report["sections_match"])

    def test_marker_strict_integer_amplitude_and_aggregate_bounds(self):
        for index in (True, 0.0, "0", -1, 4096):
            with self.subTest(index=index), self.assertRaises(ValueError):
                MODULE.marker_code(index, 32, 0.1)
        for frames in (True, 32.0, "32", 0, 31, 48001):
            with self.subTest(frames=frames), self.assertRaises(ValueError):
                MODULE.marker_code(0, frames, 0.1)
        for amplitude in (True, "0.1", 0, -0.1, 1, float("nan"), float("inf"),
                          1e-100, 0.999999999):
            with self.subTest(amplitude=amplitude), self.assertRaises(ValueError):
                MODULE.marker_code(0, 32, amplitude)
        with self.assertRaises(ValueError):
            MODULE.marker_code(4095, 48000, 0.1)
        self.assertEqual(len(MODULE.marker_code(4095, 32, 0.1)), 32)

    def test_inspection_strict_samples_lengths_thresholds_and_bool(self):
        valid = [-0.25, 0.25] * 16
        for bad in (True, "0", None, float("nan"), float("inf"), -float("inf"),
                    1, -1, 0.999999999):
            samples = valid[:]
            samples[0] = bad
            for left, right in ((samples, valid), (valid, samples)):
                with self.subTest(bad=bad), self.assertRaises(ValueError):
                    MODULE.inspect_fixed(left, right, False)
        for samples in (None, "0" * 32, b"0" * 32, iter(valid), valid[:-1], valid + [0],
                        [0.0] * 48001):
            with self.subTest(samples=type(samples)), self.assertRaises(ValueError):
                MODULE.inspect_fixed(valid, samples, False)
        for inverted in (0, 1, "false", None):
            with self.subTest(inverted=inverted), self.assertRaises(ValueError):
                MODULE.inspect_fixed(valid, valid, inverted)
        for key in ("min_correlation", "min_rms"):
            for bad in (True, "0.7", 0, -1, 1.1, float("nan"), float("inf")):
                with self.subTest(key=key, bad=bad), self.assertRaises(ValueError):
                    MODULE.inspect_fixed(valid, valid, False, **{key: bad})


if __name__ == "__main__":
    unittest.main()
