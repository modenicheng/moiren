#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Two FIR support dependencies, never a fitted cascade center or clock."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    if not path.exists():
        return None
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


TESTS = load("archive_tests", ROOT / "tests/vad-src-archive.tests.py")
PHASE_TESTS = load("phase_tests", ROOT / "tests/vad-src-phase.tests.py")
MODULE = load("vad_src_cascade", ROOT / "vad-src-cascade.py")


def fixture(count=8):
    summary, chunks = TESTS.fixture(count)
    route = TESTS.prepared_route()
    route["prepared"].update(schema_version=2, prepared_plan_revision=13)
    summary["trace"].update(schema_version=4, route=route)
    for record in chunks:
        sequence = record["chunk"]["sequence"]
        for boundary in record["chunk"]["boundaries"]:
            kind = boundary["kind"]
            point = PHASE_TESTS.callback(sequence, kind=kind)
            if kind == "virtual_microphone":
                # Engine-input coordinates are successful pushed frames, not
                # the playback carrier's extra lookahead/preloaded inventory.
                start = (sequence - 1) * 32
                point["before"]["input_frames"] = start
                point["after"]["input_frames"] = start + 32
                point["before"]["next_input_frame"] = max(0, start - 16)
                point["after"]["next_input_frame"] = start + 16
                if sequence == 1:
                    point["xruns"] = 1
            point.update(schema_version=2, execution={"plan_revision": 13, "control_generation": 0})
            boundary["checkpoints"] = [point]
    return summary, chunks


def request(*queries):
    return {"schema_version": 1, "owner_generation": 7,
            "queries": [{"sequence": seq, "output_offset": offset} for seq, offset in queries]}


class CascadeTests(unittest.TestCase):
    def evaluate(self, summary, chunks, queries=None):
        self.assertIsNotNone(MODULE, "missing bounded cascade support mapper")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "chunks.jsonl"
            path.write_bytes(TESTS.encode(summary, chunks))
            return MODULE.evaluate(summary, path, queries or request((5, 16)))

    def test_support_propagates_all_microphone_taps_not_sum_of_centers(self):
        summary, chunks = fixture()
        report = self.evaluate(summary, chunks)
        row = report["queries"][0]
        self.assertEqual(row["status"], "dependency_mapped", row)
        self.assertFalse(report["qualified"])
        self.assertFalse(report["native_execution"])
        self.assertEqual(report["physical_delay"], "not_evaluated")
        self.assertNotIn("cascade_center", row)
        dependencies = row["engine_dependencies"]
        self.assertEqual([d["microphone_input_frame"] for d in dependencies], list(range(113, 145)))
        self.assertEqual([d["engine_frame"] for d in dependencies], list(range(113, 145)))
        self.assertEqual(row["source_support"]["first_input_frame"], 98)
        self.assertEqual(row["source_support"]["last_input_frame"], 160)
        self.assertEqual(len(report["archive_passes"]), 3)
        self.assertEqual(len({p["stream"]["sha256"] for p in report["archive_passes"]}), 1)

    def test_every_dependency_requires_upstream_clean_support_and_execution(self):
        for mutation in ["xrun", "mute", "unknown_execution", "other_plan", "control", "unknown_cursor"]:
            summary, chunks = fixture()
            point = chunks[3]["chunk"]["boundaries"][0]["checkpoints"][0]
            if mutation == "xrun":
                point["xruns"] = 1
            elif mutation == "mute":
                point["muted_frames"] = 1
            elif mutation == "unknown_execution":
                point["execution"] = None
            elif mutation == "other_plan":
                point["execution"]["plan_revision"] = 14
            elif mutation == "control":
                point["execution"]["control_generation"] = 1
            else:
                point["before"] = None
            with self.subTest(mutation=mutation):
                row = self.evaluate(summary, chunks)["queries"][0]
                self.assertEqual(row["status"], "refused", row)

    def test_downstream_push_callbacks_need_authority_before_queried_output(self):
        for key, value in [("plan_revision", 14), ("control_generation", 1)]:
            summary, chunks = fixture()
            chunks[3]["chunk"]["boundaries"][1]["checkpoints"][0]["execution"][key] = value
            row = self.evaluate(summary, chunks)["queries"][0]
            self.assertEqual(row["status"], "refused", row)

    def test_unknown_or_noninitial_execution_cannot_regain_initial_authority(self):
        for kind in [0, 1]:
            for value in [None, {"plan_revision": 13, "control_generation": 1},
                          {"plan_revision": 14, "control_generation": 0}]:
                summary, chunks = fixture()
                chunks[1]["chunk"]["boundaries"][kind]["checkpoints"][0]["execution"] = value
                with self.subTest(kind=kind, execution=value):
                    row = self.evaluate(summary, chunks, request((8, 16)))["queries"][0]
                    self.assertEqual(row["status"], "refused", row)

    def test_prepared_route_invalidation_does_not_rewrite_earlier_stamped_plan(self):
        summary, chunks = fixture()
        summary["trace"]["route"]["invalidated"] = True
        for boundary in chunks[6]["chunk"]["boundaries"]:
            boundary["checkpoints"][0]["execution"]["plan_revision"] = 20
        rows = self.evaluate(summary, chunks, request((5, 16), (7, 16)))["queries"]
        self.assertEqual(rows[0]["status"], "dependency_mapped", rows)
        self.assertEqual(rows[1]["status"], "refused", rows)

    def test_unknown_old_route_or_execution_is_never_upgraded(self):
        summary, chunks = fixture()
        summary["trace"]["schema_version"] = 3
        summary["trace"]["route"]["prepared"].pop("prepared_plan_revision")
        summary["trace"]["route"]["prepared"]["schema_version"] = 1
        self.assertEqual(self.evaluate(summary, chunks)["queries"][0]["status"], "refused")
        summary["trace"].update(schema_version=4, route={"prepared": None, "invalidated": False})
        self.assertEqual(self.evaluate(summary, chunks)["queries"][0]["status"], "refused")

    def test_callback_segmentation_and_independent_chunk_order_do_not_create_pairing(self):
        summary, chunks = fixture()
        separate = []
        for boundary_index in [1, 0]:
            for record in chunks:
                separate.append({"record": "chunk", "chunk": {**record["chunk"],
                                 "sequence": len(separate) + 1,
                                 "boundaries": [record["chunk"]["boundaries"][boundary_index]]}})
        summary["trace"]["drained_chunks"] = summary["stream"]["chunks"] = len(separate)
        self.assertEqual(self.evaluate(summary, separate)["queries"][0]["status"], "dependency_mapped")

    def test_distinct_callback_segmentation_joins_engine_frames_not_sequences(self):
        summary, chunks = fixture()
        microphone = [chunks[0]["chunk"]["boundaries"][1]["checkpoints"][0]]
        for record in chunks[1:]:
            point = record["chunk"]["boundaries"][1]["checkpoints"][0]
            middle = copy.deepcopy(point["before"])
            for key in ["input_frames", "output_frames", "next_input_frame"]:
                middle[key] += 16
            first, second = copy.deepcopy(point), copy.deepcopy(point)
            first.update(after=middle, engine_frames=16, carrier_frames=16, transferred_frames=16)
            second.update(before=middle, engine_frames=16, carrier_frames=16, transferred_frames=16,
                          timeline_start=point["timeline_start"] + 16)
            microphone.extend([first, second])
        for sequence, point in enumerate(microphone, 1):
            point["sequence"] = sequence
        chunks[0]["chunk"]["boundaries"][1]["checkpoints"] = microphone
        for record in chunks[1:]:
            record["chunk"]["boundaries"].pop()
        boundary = summary["trace"]["boundaries"][1]
        boundary["total_callbacks"] = boundary["drained_checkpoints"] = len(microphone)
        summary["trace"]["drained_checkpoints"] = summary["stream"]["checkpoints"] = 8 + len(microphone)
        row = self.evaluate(summary, chunks, request((9, 0)))["queries"][0]
        self.assertEqual(row["status"], "dependency_mapped", row)
        self.assertEqual(row["source_support"]["first_input_frame"], 98)
        self.assertEqual(row["source_support"]["last_input_frame"], 160)

    def test_resets_gaps_epochs_saturation_and_unqualified_support_never_bridge(self):
        for kind in [0, 1]:
            for mutation in ["reset", "epoch", "requested", "discontinuity", "short", "saturated", "gap"]:
                summary, chunks = fixture()
                point = chunks[3]["chunk"]["boundaries"][kind]["checkpoints"][0]
                if mutation == "reset":
                    point["after"]["reset_generation"] += 1
                elif mutation == "epoch":
                    point["timeline_epoch"] += 1
                elif mutation == "requested":
                    point["requested_epoch_after"] += 1
                elif mutation == "discontinuity":
                    point["discontinuity"] = True
                elif mutation == "short":
                    point["transferred_frames"] -= 1
                elif mutation == "saturated":
                    point["before"]["saturated"] = True
                else:
                    point["timeline_start"] += 1
                with self.subTest(kind=kind, mutation=mutation):
                    row = self.evaluate(summary, chunks)["queries"][0]
                    self.assertEqual(row["status"], "refused", row)
                    self.assertNotIn("engine_dependencies", row)
                    self.assertNotIn("source_support", row)

    def test_large_integer_source_origins_remain_exact(self):
        summary, chunks = fixture()
        origin = (1 << 54) + 3
        for record in chunks:
            for cursor in [record["chunk"]["boundaries"][0]["checkpoints"][0][key] for key in ["before", "after"]]:
                for key in ["input_frames", "output_frames", "next_input_frame"]:
                    cursor[key] += origin
        row = self.evaluate(summary, chunks)["queries"][0]
        self.assertEqual(row["status"], "dependency_mapped", row)
        self.assertEqual(row["source_support"]["first_input_frame"], origin + 98)
        self.assertEqual(row["source_support"]["last_input_frame"], origin + 160)

    def test_integer_phase_uses_conservative_33_frame_support_without_fitting(self):
        summary, chunks = fixture()
        for record in chunks:
            for boundary in record["chunk"]["boundaries"]:
                point = boundary["checkpoints"][0]
                point["before"]["fraction"] = point["after"]["fraction"] = 0.0
        row = self.evaluate(summary, chunks)["queries"][0]
        self.assertEqual(row["status"], "dependency_mapped", row)
        self.assertEqual(len(row["engine_dependencies"]), 33)
        self.assertEqual(row["microphone_support"]["first_input_frame"], 112)
        self.assertEqual(row["microphone_support"]["last_input_frame"], 144)
        self.assertEqual(row["source_support"]["first_input_frame"], 96)
        self.assertEqual(row["source_support"]["last_input_frame"], 160)

    def test_incomplete_coverage_is_not_a_partial_certificate(self):
        summary, chunks = fixture()
        chunks[2]["chunk"]["boundaries"].pop(0)
        boundary = summary["trace"]["boundaries"][0]
        boundary.update(drained_checkpoints=7, missing_callbacks=1)
        summary["trace"].update(drained_checkpoints=15, history_complete=False, refusal="missing_callbacks")
        summary["stream"]["checkpoints"] = 15
        summary["history_complete"] = False
        with self.assertRaisesRegex(ValueError, "complete scalar readback"):
            self.evaluate(summary, chunks)

    def test_archive_changes_between_passes_release_no_results(self):
        summary, chunks = fixture()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "chunks.jsonl"
            path.write_bytes(TESTS.encode(summary, chunks))
            verify, passes = MODULE.ARCHIVE.verify, []

            def change_after_read(*args, **kwargs):
                report = verify(*args, **kwargs)
                passes.append(report)
                if len(passes) == 1:
                    changed = copy.deepcopy(chunks)
                    # Same valid length/shape/counts, different consumed bytes.
                    changed[-1]["chunk"]["boundaries"][0]["checkpoints"][0]["execution"]["plan_revision"] = 14
                    path.write_bytes(TESTS.encode(summary, changed))
                return report

            with patch.object(MODULE.ARCHIVE, "verify", side_effect=change_after_read), self.assertRaisesRegex(
                    ValueError, "changed between cascade passes"):
                MODULE.evaluate(summary, path, request((5, 16)))
            self.assertEqual(len(passes), 2)

    def test_maximum_queries_bound_retained_dependency_tables(self):
        summary, chunks = fixture()
        queries = request(*[(seq, offset) for seq in range(1, 9) for offset in range(32)])
        report = self.evaluate(summary, chunks, queries)
        self.assertEqual(len(report["queries"]), 256)
        self.assertTrue(any(row["status"] == "dependency_mapped" for row in report["queries"]))
        self.assertLessEqual(sum(len(row.get("engine_dependencies", [])) for row in report["queries"]), 256 * 33)

    def test_query_schema_caps_duplicates_and_owner_are_strict(self):
        summary, chunks = fixture()
        for queries in [request(), request((5, 16), (5, 16)),
                        {**request((5, 16)), "owner_generation": 8},
                        request(*[(i + 1, 0) for i in range(257)]),
                        request((True, 0)), request((5, -1))]:
            with self.subTest(queries=str(queries)[:90]), self.assertRaises(ValueError):
                self.evaluate(summary, chunks, queries)

    def test_cli_is_create_new_and_truncation_cannot_release_results(self):
        summary, chunks = fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, stream, queries, output = [root / name for name in ["summary.json", "chunks.jsonl", "queries.json", "out.json"]]
            raw = TESTS.encode(summary, chunks)
            source.write_text(json.dumps(summary), encoding="ascii")
            stream.write_bytes(raw)
            queries.write_text(json.dumps(request((5, 16))), encoding="ascii")
            argv = [sys.executable, str(ROOT / "vad-src-cascade.py"), "--summary", str(source),
                    "--chunks", str(stream), "--queries", str(queries), "--output", str(output)]
            run = subprocess.run(argv, capture_output=True, timeout=20)
            self.assertEqual(run.returncode, 0, run.stderr)
            first = output.read_bytes()
            self.assertNotEqual(subprocess.run(argv, capture_output=True, timeout=20).returncode, 0)
            self.assertEqual(output.read_bytes(), first)
            output.unlink()
            stream.write_bytes(raw[:-1])
            self.assertNotEqual(subprocess.run(argv, capture_output=True, timeout=20).returncode, 0)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
