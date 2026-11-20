#!/usr/bin/env python3
"""Numerical and refusal regressions for relative OBS two-path diagnostics."""
import importlib.util
from pathlib import Path
import sys
import unittest

import numpy as np


PATH = Path(__file__).resolve().parents[1] / "obs-two-path.py"
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("obs_two_path", PATH) if PATH.exists() else None
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(MODULE)


class TwoPathTests(unittest.TestCase):
    def module(self):
        self.assertIsNotNone(MODULE, "missing validated OBS relative-path analyzer")
        return MODULE

    def test_ncc_matches_direct_centered_windows_and_preserves_sign(self):
        m = self.module()
        rng = np.random.default_rng(11)
        x, y = rng.normal(size=73), rng.normal(size=401)
        actual = m.ncc(x, y)
        centered = x - x.mean()
        expected = []
        for i in range(len(y) - len(x) + 1):
            sample = y[i:i + len(x)] - y[i:i + len(x)].mean()
            expected.append(np.dot(centered, sample) / np.linalg.norm(centered) / np.linalg.norm(sample))
        np.testing.assert_allclose(actual, expected, atol=1e-12)
        self.assertAlmostEqual(m.ncc(x, -x)[0], -1, places=12)

    def test_silence_after_large_energy_cannot_create_impossible_peaks(self):
        m = self.module()
        rng = np.random.default_rng(12)
        x = rng.normal(size=1201)
        y = np.r_[rng.normal(size=90_000), np.zeros(30_000)]
        curve = m.ncc(x, y)
        self.assertTrue(np.isnan(curve[90_000:]).all())
        self.assertLessEqual(np.nanmax(np.abs(curve)), 1)
        with self.assertRaises(ValueError):
            m.ncc(np.zeros(20), y)
        with self.assertRaises(ValueError):
            m.ncc(x, np.full(len(x), np.nan))

    def test_two_signed_paths_are_recovered_at_known_separation(self):
        m = self.module()
        rate = 8000
        x = np.random.default_rng(13).normal(size=rate * 4)
        y = 0.7 * x.copy()
        y[848:] -= 0.6 * x[:-848]
        _, _, direct, delayed = m.correlate_window(x[rate:rate + 4000], y, rate, rate)
        self.assertAlmostEqual(direct["lag_ms"], 0, delta=0.05)
        self.assertAlmostEqual(delayed["lag_ms"], 106, delta=0.05)
        self.assertLess(delayed["correlation"], 0)
        self.assertGreater(direct["peak_ratio"], 1.8)
        self.assertGreater(delayed["peak_ratio"], 1.8)

    def test_sub_ten_ms_paths_with_negative_common_offset_are_not_outside_search(self):
        m = self.module()
        rate = 8000
        x = np.random.default_rng(19).normal(size=4 * rate)
        positions = np.arange(len(x))
        # Direct path is -20 ms relative to the separately timestamped track;
        # the returned path is 5 ms later and deliberately stronger.
        y = 0.5 * np.interp(positions + 160, positions, x, right=0)
        y += 0.8 * np.interp(positions + 120, positions, x, right=0)
        _, _, direct, returned = m.correlate_window(x[rate:rate + 4000], y, rate, rate)
        self.assertIsNotNone(direct)
        self.assertIsNotNone(returned)
        self.assertAlmostEqual(direct["lag_ms"], -20, delta=0.05)
        self.assertAlmostEqual(returned["lag_ms"] - direct["lag_ms"], 5, delta=0.05)

    def test_confidence_gates_keep_strict_thresholds_and_accept_zero_disagreement(self):
        m = self.module()
        def channel(lag=106, strength=0.7, ratio=3):
            return {"direct": {"lag_ms": 0, "abs_correlation": 0.7, "peak_ratio": 3},
                    "delayed": {"lag_ms": lag, "abs_correlation": strength, "peak_ratio": ratio}}
        baseline = [channel(), channel()]
        self.assertTrue(m.gate(baseline, baseline)["confidence_pass"])
        for bad in [
            [channel(strength=0.059), channel()],
            [channel(ratio=1.79), channel()],
            [channel(lag=106.251), channel()],
        ]:
            self.assertFalse(m.gate(bad, baseline)["confidence_pass"])
            self.assertFalse(m.gate(baseline, bad)["confidence_pass"])
        mismatch = [channel(lag=106.351), channel(lag=106.351)]
        self.assertFalse(m.gate(baseline, mismatch)["confidence_pass"])

    def test_invalid_path_order_or_nonfinite_lags_never_pass_confidence(self):
        m = self.module()
        for lag in [float("nan"), float("inf"), -10, 0]:
            channel = {"direct": {"lag_ms": 0, "abs_correlation": 0.7, "peak_ratio": 3},
                       "delayed": {"lag_ms": lag, "abs_correlation": 0.7, "peak_ratio": 3}}
            self.assertFalse(m.gate([channel] * 2, [channel] * 2)["confidence_pass"])

    def test_unrelated_or_silent_output_never_passes_confidence(self):
        m = self.module()
        rng = np.random.default_rng(14)
        x = rng.normal(size=4000)
        for y in [np.zeros(12000), rng.normal(size=12000)]:
            _, _, direct, delayed = m.correlate_window(x, y, 4000, 8000)
            pair = [{"direct": direct, "delayed": delayed}] * 2
            self.assertFalse(m.gate(pair, pair)["confidence_pass"])

    def test_complex_transfer_preserves_negative_phase_not_complex_maximum(self):
        m = self.module()
        x = np.random.default_rng(15).normal(size=65536)
        result = m.transfer(x, -2 * x, 8000, 4096)
        good = result["valid"]
        np.testing.assert_allclose(result["response"][good], -2, atol=1e-12)
        np.testing.assert_allclose(result["coherence"][good], 1, atol=1e-12)
        silent = m.transfer(np.zeros(65536), x, 8000, 4096)
        self.assertFalse(silent["valid"].any())

    def test_dropped_and_repeated_samples_make_delay_track_jumps_visible(self):
        m = self.module()
        rate = 8000
        x = np.random.default_rng(16).normal(size=8 * rate)
        for step in [-80, 80]:
            delays = []
            positions = np.arange(len(x))
            # A step in the returned source cursor skips or repeats 80 samples.
            source_positions = positions - 848 - np.where(positions >= 4 * rate, step, 0)
            returned = np.interp(source_positions, positions, x, left=0, right=0)
            y = 0.7 * x + 0.6 * returned
            for second in [1, 3, 5]:
                start = second * rate
                _, _, direct, delayed = m.correlate_window(x[start:start + 4000], y, start, rate)
                delays.append(delayed["lag_ms"] - direct["lag_ms"])
            flags = m.delay_jumps(delays, 0.35)
            self.assertEqual(flags, [False, False, True])

    def test_constant_tone_cannot_identify_an_unambiguous_delay(self):
        m = self.module()
        rate = 8000
        x = np.sin(2 * np.pi * 2000 * np.arange(4 * rate) / rate)
        y = 0.7 * x.copy()
        y[848:] += 0.6 * x[:-848]
        _, _, direct, delayed = m.correlate_window(x[rate:rate + 4000], y, rate, rate)
        pair = [{"direct": direct, "delayed": delayed}] * 2
        self.assertFalse(m.gate(pair, pair)["confidence_pass"])

    def test_sweep_is_not_a_broadband_noise_window_even_with_two_strong_peaks(self):
        m = self.module()
        rate = 48000
        rng = np.random.default_rng(18)
        noise = rng.normal(size=rate // 2)
        times = np.arange(rate // 2) / rate
        sweep = np.sin(2 * np.pi * (11000 * times + 2000 * times * times))
        self.assertTrue(hasattr(m, "broadband_reference"), "missing noise-versus-sweep discrimination")
        self.assertTrue(m.broadband_reference(noise, rate)["supported"])
        self.assertFalse(m.broadband_reference(sweep, rate)["supported"])
        self.assertFalse(m.broadband_reference(np.zeros(rate // 2), rate)["supported"])

    def test_wide_sweep_does_not_become_noise_when_its_whole_window_is_broad(self):
        m = self.module()
        rate = 48000
        times = np.arange(rate // 2) / rate
        wide_sweep = np.sin(2 * np.pi * (500 * times + 17500 * times * times))
        self.assertFalse(m.broadband_reference(wide_sweep, rate)["supported"])

    def test_unknown_windows_break_adjacency_instead_of_fitting_across_an_epoch(self):
        m = self.module()
        self.assertEqual(m.delay_jumps([106, None, 116, 116], 0.35), [False] * 4)

    def test_clock_drift_is_reported_as_changing_delay_not_corrected_to_zero(self):
        m = self.module()
        rate = 8000
        x = np.random.default_rng(17).normal(size=8 * rate)
        positions = np.arange(len(x))
        delayed = np.interp((positions - 848) / 1.0002, positions, x, left=0, right=0)
        y = 0.7 * x + 0.6 * delayed
        found = []
        for second in [1, 3, 5]:
            start = second * rate
            _, _, direct, returned = m.correlate_window(x[start:start + 2000], y, start, rate)
            found.append(returned["lag_ms"] - direct["lag_ms"])
        self.assertGreater(found[-1] - found[0], 0.5)
        self.assertGreater(min(found), 105)


if __name__ == "__main__":
    unittest.main()
