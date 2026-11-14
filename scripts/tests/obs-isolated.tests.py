#!/usr/bin/env python3
"""Synthetic controls for isolated OBS software diagnostics, never physical E2E."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

import numpy as np

sys.dont_write_bytecode = True
PATH = Path(__file__).resolve().parents[1] / "obs-isolated.py"
SPEC = importlib.util.spec_from_file_location("obs_isolated", PATH) if PATH.exists() else None
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(MODULE)


class IsolatedTests(unittest.TestCase):
    def module(self):
        self.assertIsNotNone(MODULE, "missing isolation-control analyzer")
        return MODULE

    def fixture(self):
        rate = 8000
        control = np.random.default_rng(721).normal(0, .02, (2 * rate, 1))
        control = np.repeat(control, 2, axis=1)
        reference = np.zeros((7 * rate, 2))
        reference[rate:3 * rate] = control
        return rate, control, reference, np.zeros_like(reference)

    def test_absent_analyzer_is_not_a_success(self):
        self.module()

    def test_exact_control_and_silent_return_establish_only_bounded_isolation(self):
        rate, control, reference, returned = self.fixture()
        result = self.module().control_isolation(control, reference, returned, rate)
        self.assertEqual(result["verdict"], "bounded_control_clear")
        self.assertTrue(result["established"])
        self.assertEqual(result["control_start_frame"], rate)

    def test_quiet_correlated_direct_leakage_cannot_be_called_isolated(self):
        rate, control, reference, returned = self.fixture()
        returned[rate + 71:3 * rate + 71] = control * .001
        result = self.module().control_isolation(control, reference, returned, rate)
        self.assertFalse(result["established"])
        self.assertEqual(result["verdict"], "correlated_control_leakage")

    def test_unknown_background_is_not_a_clear_isolation_control(self):
        rate, control, reference, returned = self.fixture()
        returned += np.random.default_rng(2).normal(0, .002, returned.shape)
        result = self.module().control_isolation(control, reference, returned, rate)
        self.assertFalse(result["established"])
        self.assertEqual(result["verdict"], "nonquiet_control_return")

    def test_missing_or_unrelated_reference_keeps_control_unknown(self):
        rate, control, reference, returned = self.fixture()
        for invalid in [np.zeros_like(reference), np.random.default_rng(3).normal(size=reference.shape)]:
            result = self.module().control_isolation(control, invalid, returned, rate)
            self.assertFalse(result["established"])
            self.assertEqual(result["verdict"], "control_reference_unresolved")

    def test_truncated_control_return_does_not_pass_vacuously(self):
        rate, control, reference, returned = self.fixture()
        result = self.module().control_isolation(control, reference, returned[:2 * rate], rate)
        self.assertFalse(result["established"])
        self.assertEqual(result["verdict"], "control_return_incomplete")

    def test_signed_one_path_delay_is_recovered_without_physical_claim(self):
        m = self.module()
        rate = 8000
        reference = np.random.default_rng(4).normal(size=(5 * rate, 2))
        for delay in [-160, 37, 848]:
            positions = np.arange(len(reference))
            returned = np.column_stack([np.interp(positions - delay, positions, -reference[:, c], left=0, right=0) for c in range(2)])
            result = m.relative_window(reference, returned, rate, rate)
            self.assertTrue(result["supported"])
            self.assertAlmostEqual(result["offset_frames"], delay, delta=.01)
            self.assertFalse(result["physical_qualified"])
            self.assertFalse(result["waveform_continuity_qualified"])

    def test_silence_unrelated_return_and_tone_are_refused(self):
        m = self.module()
        rate = 8000
        reference = np.random.default_rng(5).normal(size=(5 * rate, 2))
        for returned in [np.zeros_like(reference), np.random.default_rng(6).normal(size=reference.shape)]:
            self.assertFalse(m.relative_window(reference, returned, rate, rate)["supported"])
        tone = np.repeat(np.sin(2 * np.pi * 1000 * np.arange(5 * rate) / rate)[:, None], 2, axis=1)
        self.assertFalse(m.relative_window(tone, tone, rate, rate)["supported"])

    def test_two_similar_paths_and_stereo_disagreement_are_refused(self):
        m = self.module()
        rate = 8000
        reference = np.random.default_rng(7).normal(size=(5 * rate, 2))
        returned = reference.copy()
        returned[80:] += reference[:-80]
        self.assertFalse(m.relative_window(reference, returned, rate, rate)["supported"])
        returned = reference.copy()
        returned[80:, 1] = reference[:-80, 1]
        self.assertFalse(m.relative_window(reference, returned, rate, rate)["supported"])

    def test_drop_repeat_drift_and_epoch_are_never_promoted_to_continuity(self):
        m = self.module()
        rate = 8000
        reference = np.random.default_rng(8).normal(size=(7 * rate, 2))
        positions = np.arange(len(reference))
        for step in [-80, 80]:
            source = positions - 100 - np.where(positions >= 3 * rate, step, 0)
            returned = np.column_stack([np.interp(source, positions, reference[:, c], left=0, right=0) for c in range(2)])
            results = [m.relative_window(reference, returned, t * rate, rate) for t in [1, 4]]
            self.assertTrue(all(row["supported"] for row in results))
            self.assertAlmostEqual(results[1]["offset_frames"] - results[0]["offset_frames"], step, delta=.1)
            self.assertTrue(all(not row["waveform_continuity_qualified"] for row in results))
        source = (positions - 100) / 1.0002
        returned = np.column_stack([np.interp(source, positions, reference[:, c], left=0, right=0) for c in range(2)])
        results = [m.relative_window(reference, returned, t * rate, rate) for t in [1, 5]]
        self.assertTrue(all(not row["waveform_continuity_qualified"] for row in results))
        self.assertTrue(any(not row["supported"] for row in results) or results[1]["offset_frames"] > results[0]["offset_frames"] + 4)
        returned[3 * rate:4 * rate] = 0
        self.assertFalse(m.relative_window(reference, returned, 3 * rate, rate)["supported"])

    def test_nonfinite_and_incomplete_inputs_are_not_valid_windows(self):
        m = self.module()
        reference = np.random.default_rng(9).normal(size=(24000, 2))
        returned = reference.copy()
        returned[8000, 0] = np.nan
        with self.assertRaises(ValueError):
            m.relative_window(reference, returned, 8000, 8000)
        self.assertFalse(m.relative_window(reference, reference[:8100], 8000, 8000)["supported"])

    def test_incomplete_window_still_renders_a_refused_diagnostic(self):
        m = self.module()
        reference = np.random.default_rng(10).normal(size=(24000, 2))
        returned = reference[:8100]
        row = m.relative_window(reference, returned, 8000, 8000)
        row["noise_eligible"] = False
        summary = {"isolation": {"verdict": "control_return_incomplete"}, "windows": [row]}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            m.render(output, [reference, returned], 8000, summary)
            self.assertGreater((output / "overview.png").stat().st_size, 1000)


if __name__ == "__main__":
    unittest.main()
