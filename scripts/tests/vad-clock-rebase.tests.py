#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Host-only regression tests for matched native rebase/no-copy evidence."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "vad-clock-rebase.py"
SPEC = importlib.util.spec_from_file_location("vad_clock_rebase", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
analyze = MODULE.analyze


def fixture(endpoint=26, stream=46173801378424):
    clock = (f"endpoint={endpoint} session=1266750799707 generation=1 "
             f"stream={stream} callback_qpc=1266827998581")
    capture = (f"endpoint={endpoint} session=1266750799707 generation=1 "
               "gate=2 native=2138")
    return [
        f"Clock rebase cause {clock} reason=1 deadline_frame=308160 "
        "deadline_qpc=1266827985574 late_ticks=13007 frames=144 packets=2 "
        "completed=2139 presented=0",
        f"Clock rebase before {clock} slot_gen=1 gate=1 open=1 state=2 "
        "freq=10000000 epoch=1266763785574 base=0 limit=308016 running=1",
        f"Clock rebase after {clock} slot_gen=1 gate=2 open=1 state=2 "
        "freq=10000000 epoch=1266827998581 base=308016 limit=308160 running=1",
        f"Capture no-copy state {capture} state=1 stream_state=2 gate_open=1 "
        "audio_ready=1 format=2 channels=2 frames=144",
        f"Capture no-copy timing {capture} callback_qpc=1266827998581 "
        "frequency=10000000 silence_before=0 silence_after=1",
        f"Capture no-copy event {capture} callback_qpc=1266827998581 "
        "event_gate=2 packet_qpc=1266827998581 packets=2 index=0 "
        "packet_bytes=1152 native_bytes=1152 offset=2944",
        f"Capture no-copy notify {capture} callback_qpc=1266827998581 "
        "event_gate=2 packets=2 index=0 status=STATUS_SUCCESS",
    ]


def text(lines):
    return "\n".join(f"[1]0004.4238::10/31/2026-21:02:13.764 [vad]{line}"
                     for line in lines)


class ClockRebaseEvidence(unittest.TestCase):
    def test_matches_native_tuple_and_separates_two_deadlines(self):
        report = analyze(text(fixture()))
        self.assertFalse(report["qualified"])
        self.assertFalse(report["all_interval_trace_complete"])
        item, = report["associations"]
        self.assertEqual(item["endpoint"], 26)
        self.assertEqual(item["next_deadline_lateness_ticks"], 13007)
        self.assertEqual(item["serviced_deadline_qpc_derived"], 1266827955574)
        self.assertEqual(item["serviced_deadline_lateness_ticks_derived"], 43007)
        self.assertEqual(item["gate_before"], 1)
        self.assertEqual(item["gate_after"], 2)
        self.assertEqual(item["silence_before"], 0)
        self.assertEqual(item["silence_after"], 1)

    def test_joins_interleaved_out_of_order_fragments_by_identity(self):
        first, second = fixture(), fixture(endpoint=31, stream=111)
        lines = ["unrelated trace line"]
        for index in (3, 6, 1, 0, 5, 2, 4):
            lines.extend((first[index], second[index]))
        self.assertEqual(len(analyze(text(lines))["associations"]), 2)

    def test_rejects_every_incomplete_fragment_set(self):
        for index in range(7):
            with self.subTest(fragment=index), self.assertRaises(ValueError):
                analyze(text(fixture()[:index] + fixture()[index + 1:]))

    def test_rejects_duplicate_fragments_even_when_equal(self):
        for line in fixture():
            with self.subTest(fragment=line.split()[2]), self.assertRaises(ValueError):
                analyze(text(fixture() + [line]))

    def test_rejects_mismatched_join_keys(self):
        for index, old, new in (
            (1, "stream=46173801378424", "stream=111"),
            (2, "callback_qpc=1266827998581", "callback_qpc=1266827998582"),
            (3, "generation=1", "generation=2"),
            (4, "session=1266750799707", "session=1266750799708"),
            (5, "native=2138", "native=2139"),
            (6, "gate=2", "gate=3"),
        ):
            lines = fixture()
            lines[index] = lines[index].replace(old, new)
            with self.subTest(fragment=index), self.assertRaises(ValueError):
                analyze(text(lines))

    def test_rejects_ambiguous_stream_for_same_no_copy_callback(self):
        lines = fixture() + fixture(stream=112)[:3]
        with self.assertRaises(ValueError):
            analyze(text(lines))

    def test_rejects_invalid_cause_transition_and_transfer_semantics(self):
        for index, old, new in (
            (0, "reason=1", "reason=2"),
            (0, "deadline_frame=308160", "deadline_frame=308161"),
            (0, "deadline_qpc=1266827985574", "deadline_qpc=1266827985575"),
            (0, "late_ticks=13007", "late_ticks=13008"),
            (0, "completed=2139", "completed=0"),
            (1, "slot_gen=1", "slot_gen=2"),
            (1, "open=1", "open=0"),
            (1, "freq=10000000", "freq=0"),
            (1, "running=1", "running=0"),
            (2, "gate=2", "gate=1"),
            (2, "base=308016", "base=308015"),
            (2, "epoch=1266827998581", "epoch=1266827998582"),
            (3, "stream_state=2", "stream_state=1"),
            (3, "format=2", "format=1"),
            (4, "silence_after=1", "silence_after=0"),
            (5, "event_gate=2", "event_gate=3"),
            (5, "packet_qpc=1266827998581", "packet_qpc=1266827998580"),
            (5, "index=0", "index=1"),
            (5, "native_bytes=1152", "native_bytes=1144"),
            (6, "status=STATUS_SUCCESS", "status=STATUS_CANCELLED"),
        ):
            lines = fixture()
            lines[index] = lines[index].replace(old, new)
            with self.subTest(index=index, mutation=new), self.assertRaises(ValueError):
                analyze(text(lines))

    def test_rejects_malformed_or_unknown_fields_without_skipping(self):
        for suffix in (" frames=144", " unknown=1"):
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                analyze(text([fixture()[0] + suffix, *fixture()[1:]]))
        for replacement in ("frames=-1", "frames=0", "frames=18446744073709551616"):
            lines = fixture()
            lines[0] = lines[0].replace("frames=144", replacement)
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                analyze(text(lines))

    def test_does_not_infer_an_observation_from_empty_or_unrelated_trace(self):
        for value in ("", "[vad]device start", "Clock rebase cause endpoint=26"):
            with self.subTest(text=value), self.assertRaises(ValueError):
                analyze(value)

    def test_cli_creates_pinned_receipt_once_and_preserves_existing_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            trace, output = (Path(temporary) / name for name in ("trace.txt", "report.json"))
            trace.write_text(text(fixture()), encoding="utf-8")
            command = [sys.executable, str(SCRIPT), str(trace), "--output", str(output)]
            result = subprocess.run(command, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            retained = output.read_bytes()
            self.assertEqual(json.loads(retained)["decoded_trace_sha256"],
                             MODULE.hashlib.sha256(trace.read_bytes()).hexdigest().upper())
            self.assertFalse(json.loads(retained)["qualified"])
            duplicate = subprocess.run(command, capture_output=True, timeout=10)
            self.assertEqual(duplicate.returncode, 1)
            self.assertEqual(output.read_bytes(), retained)

    def test_cli_refusal_does_not_write_a_success_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            trace, output = (Path(temporary) / name for name in ("trace.txt", "report.json"))
            trace.write_text(text(fixture()[:-1]), encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(SCRIPT), str(trace), "--output", str(output)],
                capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 1)
            self.assertFalse(output.exists())
            self.assertIn(b"REFUSED:", result.stderr)


if __name__ == "__main__":
    unittest.main()
