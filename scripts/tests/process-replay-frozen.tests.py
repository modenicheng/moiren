#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Headless frozen-integer association regressions; no audio or file inputs."""

import importlib.util
import copy
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


class FrozenRecordingAudit(unittest.TestCase):
    def case(self):
        replay, recorded, valid, generation, windows = fixture()
        owners = {"owner": (replay, valid, generation)}
        association = MODULE.associate(owners, recorded, windows)
        return owners, recorded, association

    def audit(self, owners, recorded, association, **kwargs):
        audit = getattr(MODULE, "audit_recording", None)
        self.assertTrue(callable(audit), "missing full-recording frozen-map audit behavior")
        return audit(owners, recorded, association, **kwargs)

    def partition(self, result, frames):
        self.assertEqual(result["recorded_frames"], frames)
        self.assertEqual(sum(result["counts"].values()), frames)
        self.assertLessEqual(len(result["runs"]), 4096)
        end = 0
        for run in result["runs"]:
            self.assertEqual(set(run), {"start_frame", "frames", "category"})
            self.assertEqual(run["start_frame"], end)
            self.assertGreater(run["frames"], 0)
            end += run["frames"]
        self.assertEqual(end, frames)
        self.assertEqual(json.loads(json.dumps(result, allow_nan=False)), result)

    def test_entire_recording_fixed_map_keeps_training_and_tail_in_denominator(self):
        owners, recorded, association = self.case()
        result = self.audit(owners, recorded, association)
        self.partition(result, len(recorded))
        self.assertEqual(result["anchor"], association["anchor"])
        self.assertEqual(result["counts"].get("anchor_training", 0), 256)
        self.assertEqual(result["counts"]["partial_block_tail"], 44)
        self.assertGreater(result["counts"]["fixed_map_compatible"], 2900)
        self.assertEqual(result["blocks"]["frames"], 64)
        self.assertLess(result["fixed_gain_dc_error"]["channels"][0]["max_abs"], 1e-12)
        for key in ("qualified", "waveform_continuity_qualified", "identity_accepted"):
            self.assertIs(result[key], False)
        self.assertIsNone(result["physical_latency_ms"])

    def test_audit_never_searches_refits_or_changes_inputs_or_anchor(self):
        owners, recorded, association = self.case()
        before = [x.copy() for x in owners["owner"]] + [recorded.copy()]
        saved = copy.deepcopy(association)
        original_train, original_associate = MODULE.train_channel, MODULE.associate
        def forbidden(*args, **kwargs):
            raise AssertionError("audit attempted search or reanchor")
        MODULE.train_channel = MODULE.associate = forbidden
        try:
            recorded[2000:2064] = owners["owner"][0][2024:2088] * [0.7, 0.6] + [0.01, -0.02]
            before[-1] = recorded.copy()
            result = self.audit(owners, recorded, association)
        finally:
            MODULE.train_channel, MODULE.associate = original_train, original_associate
        self.assertEqual(result["anchor"], saved["anchor"])
        self.assertEqual(association, saved)
        self.assertGreater(result["counts"].get("waveform_residual", 0), 0)
        for value, expected in zip((*owners["owner"], recorded), before):
            np.testing.assert_array_equal(value, expected)
        result["anchor"]["channels"][0]["gain"] = 99
        self.assertEqual(association, saved)

    def test_outside_unknown_generation_nonfinite_silence_each_retained(self):
        owners, recorded, association = self.case()
        replay, valid, generation = owners["owner"]
        recorded = np.vstack([recorded, np.zeros((800, 2))])
        valid[1000:1064] = 0
        generation[1000:1064] = 0
        generation[1100:1164] = 8
        recorded[1200:1264] = np.nan
        replay[1400:1464] = np.inf
        recorded[1600:1728] = [0.01, -0.02]
        replay[1623:1751] = 0
        result = self.audit(owners, recorded, association)
        self.partition(result, len(recorded))
        for category in ("outside_replay", "unknown_replay", "generation_mismatch",
                         "nonfinite_recording", "nonfinite_replay", "uninformative_signal"):
            self.assertGreater(result["counts"].get(category, 0), 0, category)
        self.assertEqual(result["counts"]["outside_replay"], 227)
        self.assertEqual(result["counts"]["unknown_replay"], 64)
        self.assertEqual(result["counts"]["generation_mismatch"], 64)
        self.assertEqual(result["counts"]["nonfinite_recording"], 64)
        self.assertEqual(result["counts"]["nonfinite_replay"], 64)

    def test_silence_and_training_never_count_as_informative_pass(self):
        owners, recorded, association = self.case()
        owners["owner"][0][:] = 0
        recorded[:] = [0.01, -0.02]
        result = self.audit(owners, recorded, association)
        self.partition(result, len(recorded))
        self.assertEqual(result["counts"].get("fixed_map_compatible", 0), 0)
        self.assertEqual(result["blocks"]["informative_pass"], 0)
        self.assertEqual(result["counts"]["uninformative_signal"], len(recorded))

    def test_raw_bits_stereo_signed_zero_and_nontraining_counts(self):
        owners, recorded, association = self.case()
        replay = owners["owner"][0]
        recorded = replay[23:3523].copy()
        association["anchor"]["channels"] = [{"gain": 1.0, "dc": 0.0} for _ in range(2)]
        replay[2023, 0], recorded[2000, 0] = 0.0, -0.0
        result = self.audit(owners, recorded, association)
        raw = result["raw_bits"]
        self.assertTrue(raw["sample_encoding_match"])
        self.assertEqual(raw["support_frames"], 3500)
        self.assertEqual(raw["equal_stereo_frames"], 3499)
        self.assertEqual(raw["unequal_stereo_frames"], 1)
        self.assertEqual(raw["nontraining"]["support_frames"], 3244)
        self.assertEqual(raw["nontraining"]["equal_stereo_frames"], 3243)
        self.assertEqual(result["fixed_gain_dc_error"]["channels"][0]["max_abs"], 0.0)

    def test_unchanged_correlation_and_nrmse_gates_reject_constant_offset(self):
        owners, recorded, association = self.case()
        gates = dict(MODULE.GATES)
        recorded[1920:1984] += 0.02
        result = self.audit(owners, recorded, association)
        self.assertEqual(MODULE.GATES, gates)
        self.assertEqual(result["gates"], gates)
        self.assertGreaterEqual(result["counts"].get("waveform_residual", 0), 64)
        self.assertGreater(result["fixed_gain_dc_error"]["channels"][0]["max_abs"], 0.019)

    def test_nonfinite_training_and_tail_cannot_disappear(self):
        owners, recorded, association = self.case()
        recorded[300:428] = np.nan
        recorded[-1] = np.inf
        result = self.audit(owners, recorded, association)
        self.partition(result, len(recorded))
        self.assertEqual(result["counts"]["nonfinite_recording"], 129)
        self.assertLess(result["counts"].get("anchor_training", 0), 256)

    def test_refuses_missing_anchor_types_generation_windows_and_caps(self):
        owners, recorded, association = self.case()
        bad_reports = []
        for path, value in (("anchor", None), ("gates", {}), ("qualified", True)):
            bad = copy.deepcopy(association); bad[path] = value; bad_reports.append(bad)
        for key, value in (("owner", "absent"), ("window_index", True),
                           ("recorder_to_replay_shift_frames", 23.0),
                           ("generation", None), ("generation", 0), ("generation", True)):
            bad = copy.deepcopy(association); bad["anchor"][key] = value; bad_reports.append(bad)
        for key, value in (("gain", True), ("gain", float("inf")), ("dc", float("nan"))):
            bad = copy.deepcopy(association); bad["anchor"]["channels"][0][key] = value; bad_reports.append(bad)
        bad = copy.deepcopy(association); bad["windows"][0]["window"]["frames"] = True; bad_reports.append(bad)
        for index, bad in enumerate(bad_reports):
            with self.subTest(mutation=index):
                with self.assertRaises(ValueError): self.audit(owners, recorded, bad)
        for value in ((owners["owner"][0], owners["owner"][1]),
                      (*owners["owner"][:2], owners["owner"][2].astype(float)),
                      (*owners["owner"][:2], owners["owner"][2][:-1])):
            with self.assertRaises(ValueError): self.audit({"owner": value}, recorded, association)
        for rate in (True, 48000.0, 44100):
            with self.assertRaises(ValueError): self.audit(owners, recorded, association, rate=rate)
        old = MODULE.MAX_FRAMES
        MODULE.MAX_FRAMES = 100
        try:
            with self.assertRaises(ValueError): self.audit(owners, recorded, association)
        finally: MODULE.MAX_FRAMES = old

    def test_detail_cap_refuses_instead_of_truncating_denominator(self):
        owners, recorded, association = self.case()
        owners["owner"][1][::2] = 0
        old = getattr(MODULE, "MAX_AUDIT_RUNS", None)
        MODULE.MAX_AUDIT_RUNS = 8
        try:
            with self.assertRaisesRegex(ValueError, "run cap"):
                self.audit(owners, recorded, association)
        finally:
            if old is None: del MODULE.MAX_AUDIT_RUNS
            else: MODULE.MAX_AUDIT_RUNS = old

    def test_raw_encoding_mismatch_is_not_silently_rounded_into_bit_equality(self):
        owners, recorded, association = self.case()
        result = self.audit(owners, recorded, association)
        self.assertFalse(result["raw_bits"]["sample_encoding_match"])
        self.assertEqual(result["raw_bits"]["incomparable_frames"], len(recorded))
        self.assertEqual(result["raw_bits"]["equal_stereo_frames"], 0)

    def test_negative_shift_outside_head_and_chunk_boundary_keep_exact_partition(self):
        owners, _, association = self.case()
        rng = np.random.default_rng(955)
        replay = rng.uniform(-0.2, 0.2, (66000, 2)).astype("<f4")
        recorded = np.zeros((66064, 2))
        recorded[23:66023] = replay.astype(float)*[0.7, 0.6]+[0.01, -0.02]
        owners = {"owner": (replay, np.ones(66000, dtype="u1"),
                            np.full(66000, 7, dtype="u8"))}
        association["anchor"]["recorder_to_replay_shift_frames"] = -23
        owners["owner"][1][65534:65538] = 0
        result = self.audit(owners, recorded, association)
        self.partition(result, len(recorded))
        self.assertEqual(result["counts"]["outside_replay"], 64)
        self.assertEqual(result["counts"]["unknown_replay"], 4)

    def test_training_attempts_before_frozen_anchor_are_not_independent_coverage(self):
        owners, recorded, association = self.case()
        association["anchor"]["window_index"] = 1
        association["training_attempts"] = 2
        result = self.audit(owners, recorded, association)
        self.assertEqual(result["counts"]["anchor_training"], 512)
        self.assertEqual(len(result["training_intervals"]), 4)

    def test_actual_4096_run_cap_refuses_without_partial_success(self):
        owners, recorded, association = self.case()
        replay = np.tile(owners["owner"][0], (2, 1))
        valid = np.ones(len(replay), dtype="u1")
        valid[::2] = 0
        owners = {"owner": (replay, valid, np.full(len(replay), 7, dtype="u8"))}
        recorded = np.tile(recorded, (2, 1))
        with self.assertRaisesRegex(ValueError, "run cap"):
            self.audit(owners, recorded, association)

    def test_generation_zero_on_valid_samples_and_float_validity_refuse(self):
        owners, recorded, association = self.case()
        replay, valid, generation = owners["owner"]
        for owner in ((replay, valid.astype(float), generation),
                      (replay, valid, np.zeros_like(generation)),
                      (replay, valid, generation.astype(np.int64)*-1)):
            with self.assertRaises(ValueError):
                self.audit({"owner": owner}, recorded, association)

    def test_anchor_generation_must_exist_at_its_frozen_origin(self):
        owners, recorded, association = self.case()
        association["anchor"]["generation"] = 999
        with self.assertRaisesRegex(ValueError, "anchor generation"):
            self.audit(owners, recorded, association)

    def test_mixed_training_blocks_do_not_count_neighbors_as_informative_pass(self):
        owners, recorded, association = self.case()
        result = self.audit(owners, recorded, association)
        def category(frame):
            return next(r["category"] for r in result["runs"]
                        if r["start_frame"] <= frame < r["start_frame"]+r["frames"])
        self.assertEqual(category(300), "anchor_training")
        self.assertEqual(category(299), "unsupported_block_context")
        self.assertEqual(category(428), "unsupported_block_context")

    def test_oversized_python_anchor_scalar_refuses_with_value_error(self):
        owners, recorded, association = self.case()
        association["anchor"]["channels"][0]["gain"] = 10**400
        with self.assertRaises(ValueError):
            self.audit(owners, recorded, association)


if __name__ == "__main__":
    unittest.main()
