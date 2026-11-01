#!/usr/bin/env python3
"""Known affine linear-SRC controls; model compatibility is not qualification."""
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest import mock

import numpy as np

sys.dont_write_bytecode = True
PATH = Path(__file__).resolve().parents[1] / "obs-linear-clock.py"
SPEC = importlib.util.spec_from_file_location("obs_linear_clock", PATH) if PATH.exists() else None
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(MODULE)


class LinearClockTests(unittest.TestCase):
    rate = 8000

    def module(self):
        self.assertIsNotNone(MODULE, "missing held-out linear-clock diagnostic")
        return MODULE

    def fixture(self, delay=37.4, ppm=0, gain=1):
        x = np.random.default_rng(811).normal(0, .02, (5 * self.rate, 2))
        n = np.arange(len(x))
        centers = (n - delay) * (1 + ppm * 1e-6)
        y = gain * np.column_stack([np.interp(centers, n, x[:, c], left=0, right=0) for c in range(2)])
        return x, y

    def test_fractional_linear_phase_is_not_misclassified_as_destroyed_pcm(self):
        m = self.module()
        x, y = self.fixture(delay=37.5, gain=-.75)
        row = m.fit_linear_window(x, y, self.rate, self.rate)
        self.assertTrue(row["model_compatible"], row)
        self.assertAlmostEqual(row["relative_offset_frames"], 37.5, delta=.02)
        self.assertLess(row["channels"][0]["raw_correlation"], 0)
        self.assertLess(abs(row["channels"][0]["raw_correlation"]), .95)
        self.assertAlmostEqual(row["channels"][0]["gain"], -.75, delta=.005)
        self.assertFalse(row["physical_qualified"])
        self.assertFalse(row["waveform_continuity_qualified"])

    def test_bounded_positive_negative_drift_and_negative_offset_are_recovered(self):
        m = self.module()
        for ppm, delay in [(700, 37.4), (-700, -100.3), (1800, 130.7), (-1800, 200.2)]:
            with self.subTest(ppm=ppm):
                x, y = self.fixture(delay=delay, ppm=ppm)
                row = m.fit_linear_window(x, y, self.rate, self.rate)
                self.assertTrue(row["model_compatible"], row)
                for channel in row["channels"]:
                    self.assertAlmostEqual(channel["correction_ppm"], ppm, delta=3)
                    self.assertGreater(channel["heldout_correlation"], .99)
                    self.assertLess(channel["heldout_nrmse"], .05)

    def test_out_of_model_rate_cannot_be_hidden_by_a_better_offset(self):
        x, y = self.fixture(ppm=4000)
        self.assertFalse(self.module().fit_linear_window(x, y, self.rate, self.rate)["model_compatible"])

    def test_fit_at_rate_search_boundary_is_reported_and_refused(self):
        x, y = self.fixture(ppm=2000)
        row = self.module().fit_linear_window(x, y, self.rate, self.rate)
        self.assertFalse(row["model_compatible"])
        self.assertTrue(any(p.get("rate_bound_saturated") for p in row["channels"]))

    def test_heldout_only_corruption_does_not_change_estimated_parameters(self):
        m = self.module()
        x, y = self.fixture(ppm=500)
        baseline = m.fit_linear_window(x, y, self.rate, self.rate)
        broken = y.copy()
        # Middle-of-window validation is separate from training and its coarse
        # correspondence seed. Corrupting it may refuse, never retune the fit.
        broken[self.rate + 1500:self.rate + 1700] = 0
        result = m.fit_linear_window(x, broken, self.rate, self.rate)
        self.assertFalse(result["model_compatible"])
        for before, after in zip(baseline["channels"], result["channels"]):
            self.assertAlmostEqual(before["correction_ppm"], after["correction_ppm"], delta=1e-8)
            self.assertAlmostEqual(before["offset_frames"], after["offset_frames"], delta=1e-8)
            self.assertAlmostEqual(before["gain"], after["gain"], delta=1e-10)

    def test_independent_scalar_linear_oracle_preserves_sample_before_advance(self):
        m = self.module()
        x = np.random.default_rng(31).normal(0, .02, (5 * self.rate, 2))
        position = -.25
        y = np.zeros_like(x)
        step = 1 - 333e-6
        for frame in range(len(y)):
            whole = int(np.floor(position))
            fraction = position - whole
            if 0 <= whole < len(x) - 1:
                y[frame] = x[whole] + (x[whole + 1] - x[whole]) * fraction
            position += step
        row = m.fit_linear_window(x, y, self.rate, self.rate)
        self.assertTrue(row["model_compatible"], row)
        self.assertAlmostEqual(row["channels"][0]["correction_ppm"], -333, delta=2)

    def test_piecewise_32_frame_rate_changes_are_not_exact_affine_history(self):
        m = self.module()
        x = np.random.default_rng(32).normal(0, .02, (5 * self.rate, 2))
        y = np.zeros_like(x)
        position = -37.4
        for frame in range(len(y)):
            # Continuous phase, but the bounded rate changes only on the same
            #32-frame cadence as production. This is not an affine oracle.
            step = 1 + (1500 if (frame // 32) % 2 else -1500) * 1e-6
            whole = int(np.floor(position))
            fraction = position - whole
            if 0 <= whole < len(x) - 1:
                y[frame] = x[whole] + (x[whole + 1] - x[whole]) * fraction
            position += step
        row = m.fit_linear_window(x, y, self.rate, self.rate)
        self.assertFalse(row["waveform_continuity_qualified"])
        self.assertTrue(all(p.get("heldout_nrmse", 1) > .001 for p in row["channels"]))

    def test_heldout_energy_cannot_scale_optimizer_training_objective(self):
        m = self.module()
        x, y = self.fixture(ppm=500)
        broken = y.copy()
        broken[self.rate + 1100:self.rate + 2900] *= 10000
        original = m.optimize.least_squares
        captured = []
        def record(function, initial, **kwargs):
            captured.append(function(initial).copy())
            return original(function, initial, **kwargs)
        with mock.patch.object(m.optimize, "least_squares", side_effect=record):
            m.fit_linear_window(x, y, self.rate, self.rate)
            before = captured.copy()
            captured.clear()
            m.fit_linear_window(x, broken, self.rate, self.rate)
        self.assertEqual(len(before), len(captured))
        for a, b in zip(before, captured):
            np.testing.assert_allclose(a, b, rtol=0, atol=0)

    def test_unrelated_silence_and_periodic_signal_never_pass(self):
        m = self.module()
        x, y = self.fixture()
        for other in [np.zeros_like(y), np.random.default_rng(17).normal(0, .02, y.shape)]:
            self.assertFalse(m.fit_linear_window(x, other, self.rate, self.rate)["model_compatible"])
        tone = np.repeat(np.sin(2 * np.pi * 1000 * np.arange(len(x)) / self.rate)[:, None], 2, axis=1)
        self.assertFalse(m.fit_linear_window(tone, tone, self.rate, self.rate)["model_compatible"])

    def test_short_zero_hole_and_corruption_are_not_fitted_away(self):
        m = self.module()
        x, y = self.fixture()
        for kind in ["zero", "noise"]:
            broken = y.copy()
            section = slice(self.rate + 1500, self.rate + 1700)
            broken[section] = 0 if kind == "zero" else np.random.default_rng(18).normal(0, .02, (200, 2))
            row = m.fit_linear_window(x, broken, self.rate, self.rate)
            self.assertFalse(row["model_compatible"], row)

    def test_drop_repeat_and_two_path_mix_are_not_an_affine_resampling_pass(self):
        m = self.module()
        x, y = self.fixture()
        n = np.arange(len(x))
        for step in [-32, 32]:
            centers = n - 37.4 + np.where(n >= self.rate + 2000, step, 0)
            broken = np.column_stack([np.interp(centers, n, x[:, c], left=0, right=0) for c in range(2)])
            self.assertFalse(m.fit_linear_window(x, broken, self.rate, self.rate)["model_compatible"])
        mixed = y.copy()
        mixed[80:] += y[:-80]
        self.assertFalse(m.fit_linear_window(x, mixed, self.rate, self.rate)["model_compatible"])

    def test_stereo_disagreement_is_not_an_accepted_mean(self):
        m = self.module()
        x, y = self.fixture()
        y[40:, 1] = y[:-40, 1]
        row = m.fit_linear_window(x, y, self.rate, self.rate)
        self.assertFalse(row["model_compatible"])

    def test_unknown_epoch_and_truncated_or_nonfinite_pcm_are_refused(self):
        m = self.module()
        x, y = self.fixture()
        self.assertFalse(m.fit_linear_window(x, y[:8100], self.rate, self.rate)["model_compatible"])
        y[self.rate:2 * self.rate] = 0
        self.assertFalse(m.fit_linear_window(x, y, self.rate, self.rate)["model_compatible"])
        y[self.rate, 0] = np.nan
        with self.assertRaises(ValueError):
            m.fit_linear_window(x, y, self.rate, self.rate)


if __name__ == "__main__":
    unittest.main()
