#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Reset/fault-aware FIR coordinate intervals, never source-clock fitting."""
import copy
from fractions import Fraction
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    if not path.exists():
        return None
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ARCHIVE_TESTS = load("src_archive_tests", ROOT / "tests/vad-src-archive.tests.py")
PHASE = load("vad_src_phase", ROOT / "vad-src-phase.py")
ARCHIVE = ARCHIVE_TESTS.MODULE


def callback(sequence, *, origin=0, step=1.0, kind="virtual_playback"):
    point = ARCHIVE_TESTS.point(sequence)
    start = origin + (sequence - 1) * 32
    point["before"] = ARCHIVE_TESTS.cursor(start)
    point["after"] = ARCHIVE_TESTS.cursor(start + 32)
    point["before"]["input_frames"] = origin if sequence == 1 else start + 32
    point["after"]["input_frames"] = start + 64
    point["carrier_frames"] = 64 if sequence == 1 else 32
    point["before"]["step"] = step
    point["after"]["step"] = step
    point["processing_step"] = step
    if kind == "virtual_microphone":
        point["carrier_frames"] = 32
    return point


def queries(*pairs, kind="virtual_playback"):
    return {"schema_version": 1, "owner_generation": 7,
            "queries": [{"node": "NodeId(1v1)", "kind": kind, "sequence": seq,
                         "output_offset": offset} for seq, offset in pairs]}


class PhaseTests(unittest.TestCase):
    def observer(self, request):
        self.assertIsNotNone(PHASE, "missing reset/fault-aware phase observer")
        return PHASE.PhaseObserver(request)

    def feed(self, observer, point, kind="virtual_playback"):
        ARCHIVE.validate_point(point)
        observer.observe("NodeId(1v1)", kind, point)

    def result(self, observer):
        return observer.report({"scalar_readback_complete": True, "owner_generation": 7})

    def test_first_middle_last_centers_keep_large_integer_origins_split(self):
        observer = self.observer(queries((1, 16), (2, 16), (3, 31)))
        origin = (1 << 54) + 3
        for seq in range(1, 4):
            self.feed(observer, callback(seq, origin=origin))
        result = self.result(observer)
        self.assertFalse(result["qualified"])
        self.assertEqual(result["source_origin"], "unknown_not_estimated")
        for row, expected in zip(result["queries"], [origin + 16, origin + 48, origin + 95]):
            self.assertEqual(row["status"], "mapped", row)
            center = row["center"]
            lo, hi = (Fraction(*bound) + center["input_frame"] for bound in center["relative_interval"])
            self.assertLessEqual(lo, Fraction(expected) + Fraction(1, 8))
            self.assertGreaterEqual(hi, Fraction(expected) + Fraction(1, 8))
            self.assertLess(hi - lo, Fraction(1, 1000000))

    def test_applied_step_not_before_cursor_step_controls_mapping(self):
        observer = self.observer(queries((2, 16)))
        first = callback(1)
        self.feed(observer, first)
        point = callback(2)
        point["before"] = copy.deepcopy(first["after"])
        point["after"]["step"] = point["processing_step"] = 1.001
        point["after"]["fraction"] = 0.125 + 32 * (1.001 - 1)
        self.feed(observer, point)
        row = self.result(observer)["queries"][0]
        self.assertEqual(row["status"], "mapped", row)
        expected = Fraction(1, 8) + 16 * Fraction.from_float(1.001)
        lo, hi = map(lambda pair: Fraction(*pair), row["center"]["relative_interval"])
        self.assertLessEqual(lo, expected)
        self.assertGreaterEqual(hi, expected)

    def test_phase_or_frontier_inconsistency_refuses_without_adaptive_tolerance(self):
        for mutation in ["phase", "input", "output", "carrier", "after_step"]:
            observer = self.observer(queries((1, 20)))
            point = callback(1)
            if mutation == "phase":
                point["after"]["fraction"] += 0.001
            elif mutation == "input":
                point["after"]["input_frames"] -= 1
            elif mutation == "output":
                point["after"]["output_frames"] -= 1
            elif mutation == "carrier":
                point["carrier_frames"] -= 1
            else:
                point["after"]["step"] = 1.001
            self.feed(observer, point)
            with self.subTest(mutation=mutation):
                self.assertEqual(self.result(observer)["queries"][0]["status"], "refused")

    def test_fault_reset_mute_and_unknown_are_barriers_and_do_not_bridge(self):
        for mutation in ["xrun", "mute", "discontinuity", "reset", "epoch", "requested",
                         "unknown", "saturated", "short", "no_step"]:
            observer = self.observer(queries((1, 20)))
            point = callback(1)
            if mutation == "xrun":
                point["xruns"] = 1
            elif mutation == "mute":
                point["muted_frames"] = 1
            elif mutation == "discontinuity":
                point["discontinuity"] = True
            elif mutation == "reset":
                point["after"]["reset_generation"] = 1
            elif mutation == "epoch":
                point["boundary_epoch_after"] = 1
            elif mutation == "requested":
                point["requested_epoch_after"] = 1
            elif mutation == "unknown":
                point["before"] = None
            elif mutation == "saturated":
                point["after"]["saturated"] = True
            elif mutation == "short":
                point["transferred_frames"] -= 1
            else:
                point["processing_step"] = None
            self.feed(observer, point)
            with self.subTest(mutation=mutation):
                self.assertEqual(self.result(observer)["queries"][0]["status"], "refused")

    def test_tainted_fir_support_warms_up_instead_of_immediately_reusing_history(self):
        observer = self.observer(queries((2, 31), (3, 14), (3, 15)))
        first = callback(1)
        first["xruns"] = 1
        self.feed(observer, first)
        self.feed(observer, callback(2))
        self.feed(observer, callback(3))
        rows = self.result(observer)["queries"]
        self.assertEqual([row["status"] for row in rows], ["refused", "refused", "mapped"])
        self.assertEqual(rows[0]["reason"], "tainted_or_startup_support")

    def test_gaps_timeline_and_changed_prior_cursor_split_segments(self):
        for mutation in ["gap", "timeline", "epoch", "cursor"]:
            observer = self.observer(queries((2, 20), (3, 20)))
            first, second = callback(1), callback(2)
            if mutation == "gap":
                first["sequence"] = 0  # Do not feed a schema-invalid point.
            else:
                self.feed(observer, first)
            if mutation == "timeline":
                second["timeline_start"] += 1
            elif mutation == "epoch":
                second["timeline_epoch"] += 1
            elif mutation == "cursor":
                second["before"]["fraction"] += 0.001
            self.feed(observer, second)
            with self.subTest(mutation=mutation):
                self.assertEqual(self.result(observer)["queries"][0]["status"], "refused")

    def test_microphone_uses_fir_outputs_not_engine_input_offsets(self):
        observer = self.observer(queries((1, 15), (1, 16), kind="virtual_microphone"))
        point = callback(1, kind="virtual_microphone")
        point["after"].update(input_frames=32, output_frames=16, next_input_frame=16)
        point["carrier_frames"] = 16
        self.feed(observer, point, "virtual_microphone")
        rows = self.result(observer)["queries"]
        self.assertEqual(rows[0]["status"], "mapped")
        self.assertEqual(rows[1]["reason"], "output_offset_out_of_range")

    def test_query_schema_caps_duplicates_and_owner_are_explicit(self):
        self.assertIsNotNone(PHASE, "missing reset/fault-aware phase observer")
        for change in ["duplicate", "cap", "boolean", "kind", "extra"]:
            request = queries((1, 20))
            if change == "duplicate":
                request["queries"] *= 2
            elif change == "cap":
                request["queries"] *= 257
            elif change == "boolean":
                request["queries"][0]["sequence"] = True
            elif change == "kind":
                request["queries"][0]["kind"] = "process_input"
            else:
                request["extra"] = 0
            with self.subTest(change=change), self.assertRaises(ValueError):
                PHASE.PhaseObserver(request)
        observer = self.observer(queries((1, 20)))
        with self.assertRaises(ValueError):
            observer.report({"scalar_readback_complete": True, "owner_generation": 8})

    def test_unknown_query_and_incomplete_archive_never_release_coordinates(self):
        observer = self.observer(queries((99, 20)))
        self.feed(observer, callback(1))
        self.assertEqual(self.result(observer)["queries"][0]["reason"], "callback_not_observed")
        with self.assertRaises(ValueError):
            observer.report({"scalar_readback_complete": False, "owner_generation": 7})

    def test_rounding_bound_is_fixed_and_applied_to_the_complete_after_cursor(self):
        self.assertIsNotNone(PHASE, "missing reset/fault-aware phase observer")
        self.assertEqual(PHASE.ROUNDING_PER_OUTPUT, Fraction(1, 1 << 32))
        for multiplier, accepted in [(0.5, True), (2, False)]:
            point = callback(1)
            point["after"]["fraction"] += multiplier * 32 / (1 << 32)
            observer = self.observer(queries((1, 20)))
            self.feed(observer, point)
            row = self.result(observer)["queries"][0]
            with self.subTest(multiplier=multiplier):
                self.assertEqual(row["status"] == "mapped", accepted, row)

    def test_callback_work_and_step_range_are_not_silently_extrapolated(self):
        for mutation in ["large_count", "large_engine_block", "large_step", "tiny_step", "lookahead"]:
            observer = self.observer(queries((1, 20)))
            point = callback(1)
            if mutation == "large_count":
                point["after"]["output_frames"] += 1 << 20
            elif mutation == "large_engine_block":
                point["engine_frames"] = point["transferred_frames"] = (1 << 20) + 1
            elif mutation == "large_step":
                point["processing_step"] = point["after"]["step"] = 51.0
            elif mutation == "tiny_step":
                point["processing_step"] = point["after"]["step"] = 0.001
            else:
                point["after"]["input_frames"] = 33
                point["carrier_frames"] = 33
            self.feed(observer, point)
            with self.subTest(mutation=mutation):
                self.assertEqual(self.result(observer)["queries"][0]["status"], "refused")

    def test_faults_on_one_boundary_do_not_change_an_independent_boundary(self):
        request = queries((1, 20))
        request["queries"].append({**request["queries"][0], "node": "other"})
        observer = self.observer(request)
        bad = callback(1)
        bad["xruns"] = 1
        self.feed(observer, bad)
        observer.observe("other", "virtual_playback", callback(1))
        rows = self.result(observer)["queries"]
        self.assertEqual([row["status"] for row in rows], ["refused", "mapped"])

    def test_counter_saturation_never_regains_mapping_even_after_reset(self):
        observer = self.observer(queries((1, 20), (2, 20), (3, 20), (4, 20), (5, 20)))
        for seq in range(1, 6):
            point = callback(seq, origin=1000 if seq >= 2 else 0)
            if seq == 1:
                point["after"]["saturated"] = True
            if seq >= 3:
                point["before"]["reset_generation"] = point["after"]["reset_generation"] = 1
            self.feed(observer, point)
        self.assertTrue(all(row["status"] == "refused" for row in self.result(observer)["queries"]))

    def test_applied_step_must_match_direction_rate_and_correction_authority(self):
        for kind in ["virtual_playback", "virtual_microphone"]:
            observer = self.observer(queries((1, 20), kind=kind))
            point = callback(1, kind=kind)
            point["processing_step"] = point["after"]["step"] = 1.01
            point["after"]["fraction"] += 0.32
            self.feed(observer, point, kind)
            with self.subTest(kind=kind):
                self.assertEqual(self.result(observer)["queries"][0]["status"], "refused")

    def test_strict_archive_observer_is_optional_and_gets_every_valid_checkpoint(self):
        summary, chunks = ARCHIVE_TESTS.fixture()
        raw = ARCHIVE_TESTS.encode(summary, chunks)
        seen = []
        ARCHIVE.verify(summary, io.BytesIO(raw), checkpoint_observer=lambda *args: seen.append(args))
        self.assertEqual(len(seen), 8)
        self.assertEqual([row[2]["sequence"] for row in seen], [1, 1, 2, 2, 3, 3, 4, 4])

    def test_cli_create_new_and_truncated_archive_refuse_without_coordinates(self):
        summary, chunks = ARCHIVE_TESTS.fixture()
        for index, chunk in enumerate(chunks, 1):
            for boundary in chunk["chunk"]["boundaries"]:
                boundary["checkpoints"] = [callback(index)]
        raw = ARCHIVE_TESTS.encode(summary, chunks)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            s, c, q, out = (root / name for name in ["s.json", "c.jsonl", "q.json", "o.json"])
            s.write_text(json.dumps(summary), encoding="ascii")
            c.write_bytes(raw)
            q.write_text(json.dumps(queries((2, 20))), encoding="ascii")
            argv = [sys.executable, str(ROOT / "vad-src-phase.py"), "--summary", str(s),
                    "--chunks", str(c), "--queries", str(q), "--output", str(out)]
            run = subprocess.run(argv, capture_output=True, timeout=20)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(json.loads(out.read_bytes())["queries"][0]["status"], "mapped")
            original = out.read_bytes()
            self.assertNotEqual(subprocess.run(argv, capture_output=True, timeout=20).returncode, 0)
            self.assertEqual(out.read_bytes(), original)
            out.unlink()
            c.write_bytes(raw[:-1])
            self.assertNotEqual(subprocess.run(argv, capture_output=True, timeout=20).returncode, 0)
            self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
