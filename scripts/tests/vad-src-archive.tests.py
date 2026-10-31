#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Independent scalar archive readback, never native or waveform acceptance."""
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "vad-src-archive.py"
MODULE = None
if SCRIPT.exists():
    SPEC = importlib.util.spec_from_file_location("vad_src_archive", SCRIPT)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)


def cursor(origin=0):
    return {"reset_generation": 0, "input_frames": origin + 64,
            "output_frames": origin, "next_input_frame": origin,
            "fraction": 0.125, "step": 1.0, "saturated": False}


def point(sequence):
    return {"schema_version": 1, "sequence": sequence, "timeline_epoch": 1,
            "timeline_start": (sequence - 1) * 32, "engine_frames": 32,
            "engine_sample_rate": 48000, "boundary_epoch_before": 0,
            "boundary_epoch_after": 0, "requested_epoch_after": 0,
            "before": cursor((sequence - 1) * 32), "after": cursor(sequence * 32),
            "processing_step": 1.0, "carrier_frames": 32, "muted_frames": 0,
            "transferred_frames": 32, "discontinuity": False, "xruns": 0}


def fixture(count=4):
    boundaries = [{"node": "NodeId(1v1)", "kind": "virtual_playback"},
                  {"node": "NodeId(3v1)", "kind": "virtual_microphone"}]
    chunks = [{"record": "chunk", "chunk": {
        "schema_version": 1, "owner_generation": 7, "sequence": index,
        "boundaries": [{**boundary, "checkpoints": [point(index)]}
                       for boundary in boundaries]}}
        for index in range(1, count + 1)]
    trace = {"schema_version": 2, "owner_generation": 7, "checkpoint_limit": 4096,
             "retained_checkpoints": 0, "drained_checkpoints": count * 2,
             "drained_chunks": count, "finalized": True, "history_complete": True,
             "qualified": False, "refusal": None,
             "boundaries": [{**boundary, "total_callbacks": count, "missing_callbacks": 0,
                             "unretained_callbacks": 0, "unavailable_polls": 0,
                             "writer_finished": True, "drained_checkpoints": count,
                             "checkpoints": []} for boundary in boundaries]}
    summary = {"schema_version": 2, "trace": trace,
               "stream": {"complete": True, "chunks": count, "checkpoints": count * 2,
                          "bytes": 0, "failure": None, "failed_chunks": []},
               "chunks_path": "untrusted-path-never-followed.jsonl",
               "history_complete": True, "qualified": False,
               "limits": {"pending_checkpoints": 4096, "queued_chunks": 4,
                          "record_bytes": 8 * 1024 * 1024,
                          "stream_bytes": 32 * 1024 * 1024 * 1024}}
    return summary, chunks


def encode(summary, chunks):
    records = copy.deepcopy(chunks) + [{"record": "final", "summary": summary["trace"]}]
    raw = b"".join((json.dumps(row, allow_nan=False) + "\n").encode() for row in records)
    summary["stream"]["bytes"] = len(raw)
    return raw


class BoundedReader(io.BytesIO):
    def read(self, size=-1):
        if not 0 <= size <= 8193:
            raise AssertionError(f"unbounded read: {size}")
        return super().read(size)

    def readline(self, size=-1):
        if not 0 <= size <= 8193:
            raise AssertionError(f"unbounded readline: {size}")
        return super().readline(size)


class ArchiveTests(unittest.TestCase):
    def verify(self, summary, raw, **limits):
        self.assertIsNotNone(MODULE, "missing independent SRC archive verifier")
        return MODULE.verify(summary, BoundedReader(raw), record_limit=8192, **limits)

    def test_complete_readback_checks_all_boundaries_and_hashes(self):
        summary, chunks = fixture()
        raw = encode(summary, chunks)
        result = self.verify(summary, raw)
        self.assertTrue(result["scalar_readback_complete"])
        self.assertFalse(result["qualified"])
        self.assertFalse(result["native_execution"])
        self.assertEqual(result["source_origin"], "unknown_not_estimated")
        self.assertEqual(result["stream"]["sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(result["stream"]["bytes"], len(raw))
        self.assertEqual([row["observed_checkpoints"] for row in result["boundaries"]], [4, 4])

    def test_gap_and_original_refusal_are_not_upgraded(self):
        summary, chunks = fixture()
        del chunks[1]["chunk"]["boundaries"][0]
        summary["trace"]["boundaries"][0]["drained_checkpoints"] -= 1
        summary["trace"]["boundaries"][0]["missing_callbacks"] = 1
        summary["trace"]["drained_checkpoints"] -= 1
        summary["stream"]["checkpoints"] -= 1
        summary["trace"]["refusal"] = "missing_callbacks"
        summary["trace"]["history_complete"] = summary["history_complete"] = False
        result = self.verify(summary, encode(summary, chunks))
        self.assertFalse(result["scalar_readback_complete"])
        self.assertEqual(result["trace_refusal"], "missing_callbacks")
        self.assertEqual(result["boundaries"][0]["unobserved_callbacks"], 1)

    def test_scalar_completeness_does_not_erase_faults_or_prove_phase_mapping(self):
        summary, chunks = fixture()
        first = chunks[0]["chunk"]["boundaries"][0]["checkpoints"][0]
        first.update(xruns=3, discontinuity=True, muted_frames=16, processing_step=None)
        first["before"] = None
        first["after"]["saturated"] = True
        result = self.verify(summary, encode(summary, chunks))
        self.assertTrue(result["scalar_readback_complete"])
        self.assertEqual(result["boundaries"][0]["callbacks_with_xruns"], 1)
        self.assertEqual(result["boundaries"][0]["callbacks_with_discontinuity"], 1)
        self.assertEqual(result["boundaries"][0]["callbacks_with_mute"], 1)
        self.assertEqual(result["boundaries"][0]["callbacks_with_unknown_or_saturated_cursor"], 1)
        self.assertEqual(result["phase_mapping"], "not_evaluated")

    def test_terminal_must_exist_match_and_be_the_last_record(self):
        summary, chunks = fixture()
        good = encode(summary, chunks)
        terminal = {"record": "final", "summary": copy.deepcopy(summary["trace"])}
        terminal["summary"]["boundaries"][0]["total_callbacks"] += 1
        prefix = b"".join((json.dumps(row) + "\n").encode() for row in chunks)
        for raw in [prefix, good[:-1], good + b"\n", good + good,
                    prefix + (json.dumps(terminal) + "\n").encode()]:
            with self.subTest(tail=raw[-40:]), self.assertRaises(ValueError):
                self.verify(summary, raw)

    def test_terminal_numeric_equality_does_not_hide_type_changes(self):
        summary, chunks = fixture()
        prefix = b"".join((json.dumps(row) + "\n").encode() for row in chunks)
        for key, value in [("qualified", 0), ("finalized", 1), ("owner_generation", 7.0)]:
            terminal = {"record": "final", "summary": copy.deepcopy(summary["trace"])}
            terminal["summary"][key] = value
            raw = prefix + (json.dumps(terminal) + "\n").encode()
            summary["stream"]["bytes"] = len(raw)
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "terminal"):
                self.verify(summary, raw)

    def test_chunks_need_not_contain_simultaneous_boundary_observations(self):
        summary, chunks = fixture()
        separate = []
        for record in chunks:
            for boundary in record["chunk"]["boundaries"]:
                separate.append({"record": "chunk", "chunk": {
                    **record["chunk"], "sequence": len(separate) + 1, "boundaries": [boundary]}})
        summary["trace"]["drained_chunks"] = summary["stream"]["chunks"] = len(separate)
        self.assertTrue(self.verify(summary, encode(summary, separate))["scalar_readback_complete"])

    def test_limits_include_newline_and_allow_exact_stream_byte_budget(self):
        summary, chunks = fixture()
        raw = encode(summary, chunks)
        self.assertTrue(self.verify(summary, raw, stream_limit=len(raw))["scalar_readback_complete"])
        first_line = raw.splitlines(keepends=True)[0]
        oversized = first_line[:-1] + b" " * (8192 - len(first_line) + 1) + b"\n"
        with self.assertRaisesRegex(ValueError, "record.*cap"):
            self.verify(summary, oversized + raw[len(first_line):])

    def test_report_exposes_original_collection_counters_without_dropping_unknown_polls(self):
        summary, chunks = fixture()
        summary["trace"]["boundaries"][0]["unavailable_polls"] = 5
        result = self.verify(summary, encode(summary, chunks))
        for key, expected in [("total_callbacks", 4), ("drained_checkpoints", 4),
                              ("missing_callbacks", 0), ("unretained_callbacks", 0),
                              ("unavailable_polls", 5)]:
            with self.subTest(key=key):
                self.assertEqual(result["boundaries"][0].get(key), expected)

    def test_chunk_owner_sequence_duplicate_and_unknown_boundary_refuse(self):
        for key, value in [("owner_generation", 8), ("sequence", 1),
                           ("schema_version", 2), ("extra", 0)]:
            summary, chunks = fixture()
            chunks[1]["chunk"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.verify(summary, encode(summary, chunks))
        for mutation in ["duplicate", "unknown", "changed_kind", "empty", "regress"]:
            summary, chunks = fixture()
            boundary = chunks[1]["chunk"]["boundaries"][0]
            if mutation == "duplicate":
                chunks[1]["chunk"]["boundaries"].append(copy.deepcopy(boundary))
            elif mutation == "unknown":
                boundary["node"] = "other"
            elif mutation == "changed_kind":
                boundary["kind"] = "process_input"
            elif mutation == "empty":
                boundary["checkpoints"] = []
            else:
                boundary["checkpoints"][0]["sequence"] = 1
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.verify(summary, encode(summary, chunks))

    def test_summary_flags_cannot_override_counts_closure_or_refusal(self):
        for section, key, value in [
            ("stream", "checkpoints", 7), ("stream", "chunks", 3),
            ("stream", "bytes", 1), ("stream", "complete", False),
            ("stream", "failure", "failed"), ("stream", "failed_chunks", [{}]),
            ("trace", "drained_checkpoints", 7), ("trace", "drained_chunks", 3),
            ("trace", "retained_checkpoints", 1), ("trace", "finalized", False),
            ("trace", "refusal", "invalid_history"), ("trace", "qualified", True),
            ("boundary", "writer_finished", False), ("boundary", "drained_checkpoints", 3),
            ("boundary", "total_callbacks", 5), ("boundary", "missing_callbacks", 1)]:
            summary, chunks = fixture()
            raw = encode(summary, chunks)
            target = summary["trace"]["boundaries"][0] if section == "boundary" else summary[section]
            target[key] = value
            if section != "stream":
                raw = encode(summary, chunks)
            with self.subTest(section=section, key=key), self.assertRaises(ValueError):
                self.verify(summary, raw)

    def test_json_and_scalar_types_refuse_ambiguity(self):
        self.assertIsNotNone(MODULE, "missing independent SRC archive verifier")
        for raw in [b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":Infinity}', b'{"x":1e999}', b'\xff']:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                MODULE.load_json(raw)
        for field, value in [("sequence", True), ("sequence", -1),
                             ("engine_frames", 0), ("engine_frames", 1 << 64),
                             ("xruns", "0"), ("discontinuity", 1), ("processing_step", 0),
                             ("processing_step", True), ("processing_step", 10 ** 400)]:
            summary, chunks = fixture()
            chunks[0]["chunk"]["boundaries"][0]["checkpoints"][0][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.verify(summary, encode(summary, chunks))

    def test_split_cursor_preserves_large_integers_without_phase_fitting(self):
        summary, chunks = fixture()
        large = (1 << 54) + 3
        for boundary in chunks[0]["chunk"]["boundaries"]:
            boundary["checkpoints"][0]["before"] = cursor(large)
        self.assertTrue(self.verify(summary, encode(summary, chunks))["scalar_readback_complete"])
        for field, value in [("fraction", 1.0), ("fraction", -0.1),
                             ("next_input_frame", float(large)), ("saturated", 0), ("step", -1)]:
            broken = copy.deepcopy(chunks)
            broken[0]["chunk"]["boundaries"][0]["checkpoints"][0]["before"][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.verify(summary, encode(summary, broken))

    def test_record_stream_and_boundary_caps_are_enforced(self):
        summary, chunks = fixture()
        raw = encode(summary, chunks)
        with self.assertRaisesRegex(ValueError, "record.*cap"):
            self.verify(summary, b" " * 8193 + raw)
        with self.assertRaisesRegex(ValueError, "stream.*cap"):
            self.verify(summary, raw, stream_limit=len(raw) - 1)
        with self.assertRaisesRegex(ValueError, "boundar"):
            self.verify(summary, raw, boundary_limit=1)

    def test_jsonl_rejects_duplicate_keys_unknown_record_and_invalid_nesting(self):
        summary, chunks = fixture()
        raw = encode(summary, chunks)
        for prefix in [b'{"record":"chunk","record":"final"}\n',
                       b'{"record":"other"}\n', b'[]\n', b'\xff\n',
                       b'[' * 2000 + b']' * 2000 + b'\n']:
            with self.subTest(prefix=prefix[:60]), self.assertRaises(ValueError):
                self.verify(summary, prefix + raw)

    def test_final_metadata_preserves_unavailable_polls_and_zero_callback_refusal(self):
        summary, chunks = fixture()
        summary["trace"]["boundaries"][0]["unavailable_polls"] = 5
        self.assertTrue(self.verify(summary, encode(summary, chunks))["scalar_readback_complete"])
        summary, chunks = fixture(0)
        summary["trace"]["refusal"] = "no_callbacks"
        summary["trace"]["history_complete"] = summary["history_complete"] = False
        result = self.verify(summary, encode(summary, chunks))
        self.assertFalse(result["scalar_readback_complete"])
        self.assertEqual(result["trace_refusal"], "no_callbacks")

    def test_schema_unknowns_and_boolean_metadata_cannot_masquerade_as_integers(self):
        for section, key, value in [
            ("summary", "schema_version", True), ("summary", "extra", 0),
            ("summary", "qualified", 0), ("summary", "history_complete", 1),
            ("trace", "schema_version", 3), ("trace", "owner_generation", True),
            ("trace", "refusal", []), ("stream", "bytes", True),
            ("limits", "queued_chunks", 5), ("limits", "record_bytes", 0),
            ("boundary", "total_callbacks", True), ("boundary", "kind", []),
            ("boundary", "unavailable_polls", -1), ("boundary", "checkpoints", [{}])]:
            summary, chunks = fixture()
            raw = encode(summary, chunks)
            target = (summary if section == "summary" else summary["trace"]["boundaries"][0]
                      if section == "boundary" else summary[section])
            target[key] = value
            if section != "stream":
                raw = encode(summary, chunks)
            with self.subTest(section=section, key=key), self.assertRaises(ValueError):
                self.verify(summary, raw)

    def test_cli_retains_incomplete_scalar_receipt_with_nonzero_exit(self):
        summary, chunks = fixture()
        summary["trace"]["refusal"] = "history_unavailable"
        summary["trace"]["history_complete"] = summary["history_complete"] = False
        raw = encode(summary, chunks)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, stream, output = root / "summary.json", root / "chunks.jsonl", root / "out.json"
            source.write_text(json.dumps(summary), encoding="ascii")
            stream.write_bytes(raw)
            run = subprocess.run([sys.executable, str(SCRIPT), "--summary", str(source),
                                  "--chunks", str(stream), "--output", str(output)],
                                 capture_output=True, timeout=20)
            self.assertNotEqual(run.returncode, 0)
            result = json.loads(output.read_bytes())
            self.assertFalse(result["scalar_readback_complete"])
            self.assertEqual(result["trace_refusal"], "history_unavailable")

    def test_cli_hashes_explicit_inputs_and_never_overwrites(self):
        summary, chunks = fixture()
        raw = encode(summary, chunks)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            summary_path, chunks_path, output = root / "summary.json", root / "chunks.jsonl", root / "out.json"
            summary_raw = json.dumps(summary).encode()
            summary_path.write_bytes(summary_raw)
            chunks_path.write_bytes(raw)
            argv = [sys.executable, str(SCRIPT), "--summary", str(summary_path),
                    "--chunks", str(chunks_path), "--output", str(output)]
            run = subprocess.run(argv, capture_output=True, timeout=20)
            self.assertEqual(run.returncode, 0, run.stderr)
            result = json.loads(output.read_bytes())
            self.assertEqual(result["inputs"][0]["sha256"], hashlib.sha256(summary_raw).hexdigest())
            first = output.read_bytes()
            run = subprocess.run(argv, capture_output=True, timeout=20)
            self.assertNotEqual(run.returncode, 0)
            self.assertEqual(output.read_bytes(), first)
            output.unlink()
            chunks_path.write_bytes(raw[:-1])
            run = subprocess.run(argv, capture_output=True, timeout=20)
            self.assertNotEqual(run.returncode, 0)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
