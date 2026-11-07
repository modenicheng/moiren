"""Categorical overlay and temporary-file CLI tests; no audio or child processes."""
import copy
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "process-owner-ledger.py"
MODULE = None
if SCRIPT.is_file():
    SPEC = importlib.util.spec_from_file_location("process_owner_ledger", SCRIPT)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)


def report(*segments):
    runs, counts, cursor = [], {}, 0
    for width, category in segments:
        runs.append(dict(start_frame=cursor, frames=width, category=category))
        counts[category] = counts.get(category, 0) + width
        cursor += width
    return dict(recorded_frames=cursor, counts=counts, runs=runs)


def fixture():
    return {
        "owner-0001-generation-7": report((3, "unknown_replay"), (4, "fixed_map_compatible"),
                                          (3, "outside_replay")),
        "owner-0002-generation-7": report((5, "outside_replay"), (3, "waveform_residual"),
                                          (2, "unknown_replay")),
    }


class OverlayTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "Missing transition-ledger overlay implementation")

    def test_joint_boundaries_preserve_both_owner_categories_and_full_denominator(self):
        reports = fixture()
        a, b = reports
        result = MODULE.overlay(reports, 10)
        expected = [
            (0, 3, "unknown_replay", "outside_replay"),
            (3, 2, "fixed_map_compatible", "outside_replay"),
            (5, 2, "fixed_map_compatible", "waveform_residual"),
            (7, 1, "outside_replay", "waveform_residual"),
            (8, 2, "outside_replay", "unknown_replay"),
        ]
        self.assertEqual(result["runs"], [dict(start_frame=start, frames=width,
            owners={a: ca, b: cb}) for start, width, ca, cb in expected])
        self.assertEqual(result["partitions"], [dict(owners={a: ca, b: cb}, frames=width)
            for _, width, ca, cb in expected])
        self.assertEqual(result["recorded_frames"], 10)
        self.assertIs(result["qualified"], False)
        self.assertIsNone(result["physical_latency_ms"])
        self.assertNotIn("winner", result)
        self.assertEqual(json.loads(json.dumps(result, allow_nan=False)), result)

    def test_repeated_pair_totals_are_listed_once_without_erasing_runs(self):
        reports = {"A": report((2, "unknown"), (1, "residual"), (2, "unknown")),
                   "B": report((5, "compatible"))}
        result = MODULE.overlay(reports, 5)
        self.assertEqual(result["partitions"], [
            dict(owners={"A": "unknown", "B": "compatible"}, frames=4),
            dict(owners={"A": "residual", "B": "compatible"}, frames=1)])
        self.assertEqual(len(result["runs"]), 3)

    def test_no_input_mutation_or_output_aliases(self):
        reports = fixture()
        before = copy.deepcopy(reports)
        result = MODULE.overlay(reports, 10)
        self.assertEqual(reports, before)
        first_owner = next(iter(reports))
        result["runs"][0]["owners"][first_owner] = "changed"
        self.assertNotEqual(result["partitions"][0]["owners"][first_owner], "changed")
        self.assertEqual(reports, before)

    def test_rejects_gap_overlap_omitted_tail_and_wrong_frontier(self):
        for start in (-1, 1, True, 0.0):
            bad = fixture()
            next(iter(bad.values()))["runs"][0]["start_frame"] = start
            with self.subTest(start=start), self.assertRaises(ValueError):
                MODULE.overlay(bad, 10)
        for change in ((1, "start_frame", 4), (1, "start_frame", 2),
                       (2, "frames", 2), (2, "frames", 4), (0, "frames", 0)):
            bad = fixture()
            next(iter(bad.values()))["runs"][change[0]][change[1]] = change[2]
            with self.subTest(change=change), self.assertRaises(ValueError):
                MODULE.overlay(bad, 10)

    def test_count_mismatches_and_noninteger_counts_are_refused(self):
        for counts in ({"unknown_replay": 10}, {"unknown_replay": -1},
                       {"unknown_replay": True}, {"unknown_replay": 3.0}, {},
                       {"unknown_replay": 3, "fixed_map_compatible": 4, "outside_replay": 2},
                       {"unknown_replay": 3, "fixed_map_compatible": 4, "outside_replay": 3,
                        "invented_nonzero": 1}):
            bad = fixture()
            next(iter(bad.values()))["counts"] = counts
            with self.subTest(counts=counts), self.assertRaises(ValueError):
                MODULE.overlay(bad, 10)

    def test_existing_zero_count_categories_are_allowed_and_labels_not_rewritten(self):
        label = "unknown custom / \u672a\u77e5"
        reports = {"A": report((3, label)), "B": report((3, "fixed_map_compatible"))}
        reports["A"]["counts"]["generation_mismatch"] = 0
        result = MODULE.overlay(reports, 3)
        self.assertEqual(result["runs"][0]["owners"]["A"], label)

    def test_requires_exactly_two_unique_bounded_owner_names(self):
        row = report((1, "unknown"))
        class DuplicateOwners(dict):
            def items(self):
                return [("A", row), ("A", row)]
        for reports in ({}, {"A": row}, {"A": row, "B": row, "C": row},
                        {"": row, "B": row}, {1: row, "B": row},
                        {"A" * 257: row, "B": row}, [("A", row), ("A", row)],
                        DuplicateOwners(A=row, B=row)):
            with self.subTest(kind=type(reports).__name__), self.assertRaises(ValueError):
                MODULE.overlay(reports, 1)

    def test_frame_cap_and_report_denominator_are_strict(self):
        for frames in (0, -1, True, 1.0, 3840001):
            with self.subTest(frames=frames), self.assertRaises(ValueError):
                MODULE.overlay(fixture(), frames)
        for value in (9, True, 10.0, None):
            bad = fixture()
            next(iter(bad.values()))["recorded_frames"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                MODULE.overlay(bad, 10)
        full = report((3840000, "unknown"))
        self.assertEqual(MODULE.overlay({"A": full, "B": full}, 3840000)["recorded_frames"], 3840000)

    def test_run_cap_and_joint_work_bound(self):
        a = report(*[(2, "a%d" % (i % 2)) for i in range(4096)])
        b = report((1, "head"), *[(2, "b%d" % (i % 2)) for i in range(4094)], (3, "tail"))
        result = MODULE.overlay({"A": a, "B": b}, 8192)
        self.assertLessEqual(len(result["runs"]), 8191)
        self.assertEqual(sum(row["frames"] for row in result["partitions"]), 8192)
        too_many = report(*[(1, "unknown") for _ in range(4097)])
        with self.assertRaises(ValueError):
            MODULE.overlay({"A": too_many, "B": too_many}, 4097)

    def test_missing_malformed_categories_and_rows_are_refused(self):
        for category in (None, "", 7, ["unknown"], "x" * 257):
            bad = fixture()
            next(iter(bad.values()))["runs"][0]["category"] = category
            with self.subTest(category=category), self.assertRaises(ValueError):
                MODULE.overlay(bad, 10)
        for field in ("runs", "counts", "recorded_frames"):
            bad = fixture()
            del next(iter(bad.values()))[field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                MODULE.overlay(bad, 10)
        for malformed in (None, [], [None], [{}], iter([])):
            bad = fixture()
            next(iter(bad.values()))["runs"] = malformed
            with self.subTest(kind=type(malformed).__name__), self.assertRaises(ValueError):
                MODULE.overlay(bad, 10)


class CliTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(callable(getattr(MODULE, "main", None)), "Missing bounded CLI")
        temporary = tempfile.TemporaryDirectory(prefix="moiren-owner-ledger-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.paths = [self.root / "first.json", self.root / "second.json"]
        for path, value in zip(self.paths, fixture().values()):
            path.write_text(json.dumps(value), encoding="ascii")
        self.output = self.root / "joint.json"

    def args(self):
        return ["--owner", "owner1", str(self.paths[0]), "--owner", "owner2", str(self.paths[1]),
                "--frames", "10", "--output", str(self.output)]

    def invoke(self, argv=None):
        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
            try:
                return MODULE.main(self.args() if argv is None else argv)
            except SystemExit as error:
                return error.code

    def test_cli_creates_json_overlay_without_mutating_original_inputs(self):
        before = [path.read_bytes() for path in self.paths]
        self.assertEqual(self.invoke(), 0)
        result = json.loads(self.output.read_bytes())
        self.assertEqual(result["recorded_frames"], 10)
        self.assertEqual(sum(row["frames"] for row in result["partitions"]), 10)
        self.assertFalse(result["qualified"])
        self.assertIsNone(result["physical_latency_ms"])
        self.assertEqual(before, [path.read_bytes() for path in self.paths])

    def test_cli_requires_exactly_two_distinct_owner_labels(self):
        args = self.args()
        duplicate = args.copy()
        duplicate[4] = "owner1"
        for argv in (args[3:], args + ["--owner", "owner3", str(self.paths[0])], duplicate):
            with self.subTest(argv=argv):
                self.assertNotEqual(self.invoke(argv), 0)
                self.assertFalse(self.output.exists())

    def test_duplicate_json_keys_are_refused_at_every_depth(self):
        for text in (
            '{"recorded_frames":10,"recorded_frames":10,"counts":{},"runs":[]}',
            '{"recorded_frames":10,"counts":{"unknown":10,"unknown":10},"runs":[]}',
            '{"recorded_frames":10,"counts":{"unknown":10},"runs":[{"start_frame":0,"frames":10,"category":"unknown","category":"other"}]}',
        ):
            self.paths[0].write_text(text, encoding="ascii")
            self.assertNotEqual(self.invoke(), 0)
            self.assertFalse(self.output.exists())

    def test_bool_frames_malformed_json_and_nonfinite_tokens_are_refused(self):
        for text in ('true', '[]', 'null', '{"recorded_frames":true}',
                     '{"recorded_frames":NaN}', '{"recorded_frames":Infinity}', '{'):
            self.paths[0].write_text(text, encoding="ascii")
            with self.subTest(text=text):
                self.assertNotEqual(self.invoke(), 0)
                self.assertFalse(self.output.exists())
        args = self.args()
        args[args.index("--frames") + 1] = "True"
        self.assertNotEqual(self.invoke(args), 0)

    def test_cli_partition_gap_overlap_tail_and_count_mismatch_are_refused(self):
        for field, value in (("start_frame", 1), ("start_frame", -1), ("frames", 2)):
            data = report((10, "unknown"))
            data["runs"][0][field] = value
            self.paths[0].write_text(json.dumps(data), encoding="ascii")
            self.assertNotEqual(self.invoke(), 0)
            self.assertFalse(self.output.exists())
        data = report((10, "unknown"))
        data["counts"]["unknown"] = 9
        self.paths[0].write_text(json.dumps(data), encoding="ascii")
        self.assertNotEqual(self.invoke(), 0)

    def test_input_file_cap_refuses_excess_without_creating_output(self):
        with self.paths[0].open("wb") as stream:
            stream.write(b" " * (8 * 1024 * 1024 + 1))
        self.assertNotEqual(self.invoke(), 0)
        self.assertFalse(self.output.exists())

    def test_exact_input_caps_accept_sixteen_mib_aggregate(self):
        for path in self.paths:
            data = path.read_bytes()
            with path.open("wb") as stream:
                stream.write(data)
                stream.write(b" " * (8 * 1024 * 1024 - len(data)))
        self.assertEqual(self.invoke(), 0)

    def test_existing_output_and_input_as_output_are_never_overwritten(self):
        self.output.write_bytes(b"original")
        self.assertNotEqual(self.invoke(), 0)
        self.assertEqual(self.output.read_bytes(), b"original")
        args = self.args()
        args[-1] = str(self.paths[0])
        before = self.paths[0].read_bytes()
        self.assertNotEqual(self.invoke(args), 0)
        self.assertEqual(self.paths[0].read_bytes(), before)

    def test_output_byte_cap_is_checked_before_creating_new_file(self):
        left = report(*[(2, ("a%d" % i).ljust(256, "x")) for i in range(4096)])
        right = report((1, "head"), *[(2, ("b%d" % i).ljust(256, "y")) for i in range(4094)], (3, "tail"))
        for path, value in zip(self.paths, (left, right)):
            path.write_text(json.dumps(value), encoding="ascii")
            self.assertLess(path.stat().st_size, 8 * 1024 * 1024)
        args = self.args()
        args[1], args[4] = "A" * 256, "B" * 256
        args[args.index("--frames") + 1] = "8192"
        self.assertNotEqual(self.invoke(args), 0)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
