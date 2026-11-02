#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Headless frozen-integer association regressions; no audio or file inputs."""

import importlib.util
import json
from pathlib import Path
import unittest

import numpy as np


SCRIPT = Path(__file__).resolve().parents[1] / "process-replay-frozen.py"
MODULE = None
if SCRIPT.exists():
    SPEC = importlib.util.spec_from_file_location("process_replay_frozen", SCRIPT)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)


def fixture():
    rng = np.random.default_rng(917)
    replay = rng.uniform(-0.2, 0.2, (4096, 2)).astype("<f4")
    recorded = replay[23:3523].astype(np.float64) * [0.7, 0.6] + [0.01, -0.02]
    valid = np.ones(len(replay), dtype=np.uint8)
    generations = np.full(len(replay), 7, dtype=np.uint64)
    windows = [{"start_frame": start, "frames": 512} for start in (300, 1300, 2300)]
    return replay, recorded, valid, generations, windows


class FrozenReplay(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "missing frozen integer replay helper")

    def test_integer_gain_dc_and_immutable_inputs(self):
        replay, recorded, valid, generation, windows = fixture()
        before = [value.copy() for value in (replay, recorded, valid, generation)]
        report = MODULE.associate(
            {"owner-0002-generation-1": (replay, valid, generation)}, recorded, windows
        )
        anchor = report["anchor"]
        self.assertEqual(anchor["window_index"], 0)
        self.assertEqual(anchor["owner"], "owner-0002-generation-1")
        self.assertEqual(anchor["recorder_to_replay_shift_frames"], 23)
        self.assertTrue(all(row["association_compatible"] for row in report["windows"]))
        self.assertIs(report["qualified"], False)
        self.assertIs(report["identity_accepted"], False)
        self.assertEqual(json.loads(json.dumps(report, allow_nan=False)), report)
        for original, saved in zip((replay, recorded, valid, generation), before):
            np.testing.assert_array_equal(original, saved)

    def test_heldout_damage_freezes_anchor_and_never_reanchors(self):
        replay, recorded, valid, generation, windows = fixture()
        recorded[450:650] *= -1
        report = MODULE.associate(
            {"owner": (replay, valid, generation)}, recorded, windows
        )
        self.assertEqual(report["anchor"]["window_index"], 0)
        self.assertFalse(report["windows"][0]["association_compatible"])
        self.assertTrue(report["windows"][1]["association_compatible"])
        self.assertTrue(report["windows"][2]["association_compatible"])
        self.assertEqual(report["training_attempts"], 1)

    def test_false_first_anchor_is_not_replaced_by_later_mapping(self):
        replay, recorded, valid, generation, windows = fixture()
        recorded[300:812] = replay[900:1412] * [0.7, 0.6] + [0.01, -0.02]
        recorded[450:650] = 0
        report = MODULE.associate(
            {"owner": (replay, valid, generation)}, recorded, windows
        )
        self.assertEqual(report["anchor"]["recorder_to_replay_shift_frames"], 600)
        self.assertEqual(report["anchor"]["window_index"], 0)
        self.assertTrue(
            all(not row["association_compatible"] for row in report["windows"])
        )

    def test_duplicate_owner_and_repeated_template_refuse(self):
        replay, recorded, valid, _, windows = fixture()
        report = MODULE.associate(
            {"a": (replay, valid), "b": (replay, valid)}, recorded, windows[:1]
        )
        self.assertIsNone(report["anchor"])
        replay[2000:2512] = replay[323:835]
        report = MODULE.associate({"a": (replay, valid)}, recorded, windows[:1])
        self.assertIsNone(report["anchor"])

    def test_unknown_or_generation_crossing_refuses_complete_window(self):
        for fault in ("unknown", "generation"):
            replay, recorded, valid, generation, windows = fixture()
            if fault == "unknown":
                valid[500] = 0
                generation[500] = 0
            else:
                generation[500:] = 8
            report = MODULE.associate(
                {"a": (replay, valid, generation)}, recorded, windows[:1]
            )
            self.assertIsNone(report["anchor"], fault)
            self.assertFalse(report["windows"][0]["association_compatible"])

    def test_ambiguous_other_owner_still_competes_with_unique_owner(self):
        replay, recorded, valid, _, windows = fixture()
        other = replay.copy()
        other[2000:2512] = replay[323:835]
        report = MODULE.associate(
            {"unique": (replay, valid), "repeated": (other, valid)},
            recorded,
            windows[:1],
        )
        self.assertIsNone(report["anchor"])

    def test_sparse_1024_marker_1184_window_preserves_training_and_heldout(self):
        rng = np.random.default_rng(214)
        replay = np.zeros((20000, 2), dtype="<f4")
        for start in (1000, 6000, 11000):
            replay[start : start + 1024] = rng.choice([-0.025, 0.025], (1024, 2))
        recorded = replay[101:18101].astype(float) * 0.75
        valid = np.ones(len(replay), dtype="u1")
        windows = [
            {"start_frame": start - 101 - 80, "frames": 1184}
            for start in (1000, 6000, 11000)
        ]
        report = MODULE.associate({"owner": (replay, valid)}, recorded, windows)
        self.assertEqual(report["anchor"]["recorder_to_replay_shift_frames"], 101)
        self.assertTrue(all(row["association_compatible"] for row in report["windows"]))

    def test_later_unknown_or_generation_change_does_not_reanchor(self):
        for fault in ("unknown", "generation"):
            replay, recorded, valid, generation, windows = fixture()
            if fault == "unknown":
                valid[1500] = 0
                generation[1500] = 0
            else:
                generation[1323:1835] = 8
            report = MODULE.associate(
                {"a": (replay, valid, generation)}, recorded, windows
            )
            self.assertEqual(report["anchor"]["window_index"], 0)
            self.assertFalse(report["windows"][1]["association_compatible"])
            self.assertEqual(report["training_attempts"], 1)

    def test_drift_later_refuses_without_refitting(self):
        replay, recorded, valid, _, windows = fixture()
        recorded[1300:1812] = replay[1324:1836] * [0.7, 0.6] + [0.01, -0.02]
        report = MODULE.associate({"a": (replay, valid)}, recorded, windows)
        self.assertEqual(report["anchor"]["recorder_to_replay_shift_frames"], 23)
        self.assertFalse(report["windows"][1]["association_compatible"])

    def test_fractional_mapping_refuses(self):
        replay, recorded, valid, _, windows = fixture()
        recorded[:] = (0.5 * replay[23:3523] + 0.5 * replay[24:3524]) * 0.7
        report = MODULE.associate({"a": (replay, valid)}, recorded, windows)
        self.assertIsNone(report["anchor"])

    def test_silent_training_and_heldout_refuse(self):
        replay, recorded, valid, _, windows = fixture()
        recorded[:] = 0
        self.assertIsNone(
            MODULE.associate({"a": (replay, valid)}, recorded, windows)["anchor"]
        )
        replay, recorded, valid, _, windows = fixture()
        recorded[428:684] = 0
        report = MODULE.associate({"a": (replay, valid)}, recorded, windows[:1])
        self.assertIsNotNone(report["anchor"])
        self.assertFalse(report["windows"][0]["association_compatible"])

    def test_bounds_types_and_nonfinite_refuse(self):
        replay, recorded, valid, generation, windows = fixture()
        for bad in (
            [{"start_frame": -1, "frames": 512}],
            [{"start_frame": 0, "frames": True}],
            [{"start_frame": 3200, "frames": 512}],
            [windows[1], windows[0]],
            windows * 30,
            [{"start_frame": 0, "frames": 32}],
            [{"start_frame": 0, "frames": 512, "identity": 1}],
        ):
            with self.assertRaises(ValueError):
                MODULE.associate({"a": (replay, valid)}, recorded, bad)
        for owners in (
            {},
            {"a": (replay, valid[:-1])},
            {"a": (replay, valid, generation[:-1])},
            {"a": (replay, valid, generation.astype(float))},
        ):
            with self.assertRaises(ValueError):
                MODULE.associate(owners, recorded, windows)
        for rate in (True, 48000.0, 44100):
            with self.assertRaises(ValueError):
                MODULE.associate({"a": (replay, valid)}, recorded, windows, rate)
        replay[0, 0] = np.nan
        with self.assertRaises(ValueError):
            MODULE.associate({"a": (replay, valid)}, recorded, windows)


if __name__ == "__main__":
    unittest.main()
