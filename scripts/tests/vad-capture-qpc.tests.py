#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Portable regressions for first-copy/nonzero capture timestamp evidence."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "vad-capture-qpc.py"
SPEC = importlib.util.spec_from_file_location("vad_capture_qpc", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def fixture():
    identity = "endpoint=6 session=123456 generation=1"
    stream = "stream=9988"
    lines = [
        f"Capture boundary shape {identity} {stream} kind=0 gate=1 native=0 request=1 "
        "qpc=1000000 packets=2 packet_bytes=1152 native_bytes=1152 ring_offset=2944 packet_offset=0",
        f"Capture boundary prefix {identity} {stream} kind=0 bytes=16 source_nonzero=0 destination_nonzero=0 equal=1",
        f"Capture packet notify {identity} kind=0 gate=1 native=0 completion_qpc=1000000 packets=2 status=STATUS_SUCCESS",
        f"Capture boundary shape {identity} {stream} kind=1 gate=1 native=2 request=3 "
        "qpc=1060000 packets=2 packet_bytes=1152 native_bytes=1152 ring_offset=2944 packet_offset=0",
        f"Capture boundary prefix {identity} {stream} kind=1 bytes=16 source_nonzero=1 destination_nonzero=1 equal=1",
        f"Capture nonzero notify {identity} gate=1 native=2 request=3 index=0 "
        "copy_qpc=1060000 enter_qpc=1060010 return_qpc=1060020 status=STATUS_SUCCESS",
    ]
    packets = [{"packet_sequence": i + 1, "flags": 0,
                "device_position_frames": (i + 1) * 144, "qpc_100ns": 1000000 + i * 30000,
                "frames": 144, "stored_start_frame": i * 144} for i in range(4)]
    provider = {
        "intent": "Microphone", "diagnostic_provenance": "Fresh", "error": None,
        "snapshot": {"state": "Present", "stream_state": "Running", "gate_open": True,
                     "audio_ready": True, "intent": "Microphone", "channels": "Stereo",
                     "sample_format": "Float32", "packet_frames": 144, "stream_generation": 1,
                     "gate_epoch": 1, "stamp": [6, 123456, 6, 1, 1]},
        "telemetry": {"microphone_submission_service": {"qpc_frequency_hz": 10000000}},
    }
    report = {"qualified": False, "mode": "live_vad", "schema_version": 1,
              "native": {"vad_microphone_provider": provider, "capture": None},
              "collection": {"quality_complete": False, "qualified": False,
                             "measurement": {
                                 "info": {"sample_rate": 48000, "channels": 2},
                                 "native_capture": {
                                     "sample_rate": 48000, "channels": 2, "buffer_frames": 144,
                                     "evidence_metadata": {"sample_rate": 48000, "channels": 2,
                                                           "stored_frames": 576, "packets": 4}},
                                 "sink": {"written_frames": 576, "complete_packets": 4}}}}
    return lines, packets, report


def text(lines):
    return "\n".join("[1]0004.1234::10/31/2026-22:00:00.000 [vad]" + line for line in lines)


def analyze(lines=None, packets=None, report=None):
    a, b, c = fixture()
    try:
        return MODULE.analyze(text(a if lines is None else lines),
                              b if packets is None else packets, c if report is None else report)
    except ValueError as error:
        raise AssertionError(f"supported association refused: {error}") from error


def refuses(lines=None, packets=None, report=None):
    a, b, c = fixture()
    return MODULE.analyze(text(a if lines is None else lines),
                          b if packets is None else packets, c if report is None else report)


class CaptureQpcEvidence(unittest.TestCase):
    def test_exact_first_copy_and_nonzero_labels_not_start_or_quality(self):
        result = analyze()
        self.assertFalse(result["qualified"])
        self.assertFalse(result["all_interval_trace_complete"])
        self.assertEqual(result["packet_start_time"], "unknown_not_derived")
        rows = result["associations"]
        self.assertEqual([r["packet_sequence"] for r in rows], [1, 3])
        self.assertEqual([r["native_sequence"] for r in rows], [0, 2])
        self.assertEqual([r["stored_start_frame"] for r in rows], [0, 288])
        self.assertTrue(all(r["raw_qpc_equals_callback"] for r in rows))
        self.assertEqual(rows[1]["notify_call_ticks"], 10)
        self.assertEqual(result["packet_summary"]["qpc_policy_violations"], 0)

    def test_interleaving_and_sibling_endpoint_never_change_association(self):
        lines, _, _ = fixture()
        sibling = [line.replace("endpoint=6", "endpoint=9") for line in lines]
        mixed = ["unrelated message"]
        for i in [4, 2, 0, 5, 3, 1]:
            mixed.extend([sibling[i], lines[i]])
        self.assertEqual(analyze(mixed), analyze())

    def test_every_required_fragment_is_required_and_duplicates_refuse(self):
        lines, _, _ = fixture()
        for i in range(len(lines)):
            with self.subTest(missing=i), self.assertRaises(ValueError):
                refuses(lines[:i] + lines[i + 1:])
            with self.subTest(duplicate=i), self.assertRaises(ValueError):
                refuses(lines + [lines[i]])

    def test_schema_and_integer_bounds_are_strict(self):
        lines, _, _ = fixture()
        for old, new in [("request=1", "request=1 extra=1"), ("native=0", "native=-1"),
                         ("qpc=1000000", "qpc=18446744073709551616"),
                         ("request=1", "request=1 request=1")]:
            with self.subTest(new=new), self.assertRaises(ValueError):
                refuses([line.replace(old, new) for line in lines])

    def test_identity_stream_gate_and_request_must_match(self):
        lines, _, _ = fixture()
        for index, old, new in [(1, "stream=9988", "stream=9989"), (3, "gate=1", "gate=2"),
                                (5, "native=2", "native=3"), (5, "request=3", "request=4"),
                                (2, "session=123456", "session=123457"),
                                (3, "generation=1", "generation=2")]:
            changed = lines.copy()
            changed[index] = changed[index].replace(old, new)
            with self.subTest(index=index, new=new), self.assertRaises(ValueError):
                refuses(changed)

    def test_no_nearest_time_join_or_duplicate_qpc_candidate(self):
        _, packets, _ = fixture()
        for change in (lambda p: p[2].update(qpc_100ns=1060001),
                       lambda p: p[3].update(qpc_100ns=1060000)):
            altered = copy.deepcopy(packets)
            change(altered)
            with self.subTest(change=change), self.assertRaises(ValueError):
                refuses(packets=altered)

    def test_packet_sequence_position_and_storage_cannot_be_inferred(self):
        _, packets, _ = fixture()
        for key in ["packet_sequence", "device_position_frames", "stored_start_frame", "frames"]:
            altered = copy.deepcopy(packets)
            altered[2][key] += 1
            with self.subTest(key=key), self.assertRaises(ValueError):
                refuses(packets=altered)

    def test_flags_prefix_shape_and_notify_failure_refuse_selected_witnesses(self):
        lines, packets, _ = fixture()
        for flag in [1, 2, 4, 8]:
            altered = copy.deepcopy(packets)
            altered[2]["flags"] = flag
            with self.subTest(flag=flag), self.assertRaises(ValueError):
                refuses(packets=altered)
        for old, new in [("equal=1", "equal=0"), ("packets=2", "packets=1"),
                         ("packet_bytes=1152", "packet_bytes=1144"),
                         ("source_nonzero=1", "source_nonzero=0"),
                         ("STATUS_SUCCESS", "STATUS_UNSUCCESSFUL"),
                         ("return_qpc=1060020", "return_qpc=1060009")]:
            with self.subTest(new=new), self.assertRaises(ValueError):
                refuses([line.replace(old, new) for line in lines])

    def test_frequency_or_stale_or_closed_report_refuses_not_converts(self):
        _, _, report = fixture()
        for key, value in [("diagnostic_provenance", "Historical"), ("intent", "Playback")]:
            altered = copy.deepcopy(report)
            altered["native"]["vad_microphone_provider"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                refuses(report=altered)
        for freq in [0, 9999999, True, "10000000"]:
            altered = copy.deepcopy(report)
            altered["native"]["vad_microphone_provider"]["telemetry"]["microphone_submission_service"]["qpc_frequency_hz"] = freq
            with self.subTest(freq=freq), self.assertRaises(ValueError):
                refuses(report=altered)
        for key, value in [("state", "Removed"), ("stream_state", "Closed"), ("gate_open", False),
                           ("packet_frames", 480), ("stream_generation", 2)]:
            altered = copy.deepcopy(report)
            altered["native"]["vad_microphone_provider"]["snapshot"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                refuses(report=altered)

    def test_report_shape_counts_and_strict_numeric_fields(self):
        _, packets, report = fixture()
        altered = copy.deepcopy(report)
        altered["collection"]["measurement"]["sink"]["written_frames"] -= 1
        with self.assertRaises(ValueError):
            refuses(report=altered)
        for key, value in [("flags", True), ("qpc_100ns", 1.0), ("frames", "144"),
                           ("device_position_frames", -1), ("flags", 1 << 32), ("extra", 1)]:
            altered_packets = copy.deepcopy(packets)
            altered_packets[1][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                refuses(packets=altered_packets)
        for bad in [None, [], {}, {"mode": "live_vad"}]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                MODULE.analyze(text(fixture()[0]), packets, bad)

    def test_split_pdo_token_and_endpoint_generation_are_refused(self):
        _, _, report = fixture()
        report["native"]["vad_microphone_provider"]["snapshot"]["stamp"][0] = 333
        with self.assertRaises(ValueError):
            refuses(report=report)

    def test_unselected_packet_faults_remain_explicit(self):
        _, packets, _ = fixture()
        packets[1]["flags"] = 4
        result = analyze(packets=packets)
        self.assertEqual(result["packet_summary"]["timestamp_unknown_packets"], 1)
        self.assertEqual(result["packet_summary"]["flagged_packets"], 1)
        self.assertEqual(result["packet_summary"]["qpc_intervals_checked"], 1)
        self.assertFalse(result["qualified"])

    def test_in_memory_resource_bounds_refuse_before_parsing(self):
        lines, packets, report = fixture()
        for constant, limit in [("MAX_TRACE_BYTES", 8), ("MAX_TRACE_LINES", 2),
                                ("MAX_LINE_BYTES", 16), ("MAX_PACKETS", 2)]:
            with self.subTest(constant=constant), patch.object(MODULE, constant, limit, create=True):
                with self.assertRaises(ValueError):
                    MODULE.analyze(text(lines), packets, report)

    def test_json_and_bounded_read_refuse_duplicates_constants_and_growth(self):
        # File-size checks alone do not bound an input that grows after stat().
        self.assertTrue(callable(getattr(MODULE, "read_bounded", None)), "bounded reader required")
        self.assertTrue(callable(getattr(MODULE, "load_json", None)), "strict JSON decoder required")
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input"
            source.write_bytes(b"123456789")
            with self.assertRaises(ValueError):
                MODULE.read_bounded(source, 8)
        for raw in ['{"a": 1, "a": 2}', '{"a": NaN}', '{"a": Infinity}']:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                MODULE.load_json(raw)

    def test_qpc_policy_failure_is_retained_not_laundered_by_matched_labels(self):
        _, packets, _ = fixture()
        packets[1]["qpc_100ns"] += 1000
        result = analyze(packets=packets)
        self.assertEqual(result["packet_summary"]["qpc_policy_violations"], 2)
        self.assertFalse(result["qualified"])
        self.assertEqual(result["packet_summary"]["position_step_mismatches"], 0)

    def test_qpc_policy_integer_boundary_and_unknown_regressions(self):
        for offset, violations in [(238, 0), (239, 2)]:
            _, packets, _ = fixture()
            packets[1]["qpc_100ns"] += offset
            result = analyze(packets=packets)
            self.assertEqual(result["packet_summary"]["qpc_policy_violations"], violations)
        _, packets, _ = fixture()
        packets[1]["qpc_100ns"] = 999999
        result = analyze(packets=packets)
        self.assertEqual(result["packet_summary"]["qpc_repeat_or_rewind_intervals"], 1)
        self.assertEqual(result["packet_summary"]["qpc_policy_violations"], 1)
        packets[1]["qpc_100ns"] = 0
        result = analyze(packets=packets)
        self.assertEqual(result["packet_summary"]["timestamp_unknown_packets"], 1)
        self.assertEqual(result["packet_summary"]["qpc_intervals_checked"], 1)

    def test_cli_creates_only_new_output_and_refusal_writes_no_success(self):
        lines, packets, report = fixture()
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            trace = base / "trace.txt"
            trace.write_text(text(lines), encoding="utf-8")
            metadata = base / "packets.jsonl"
            metadata.write_text("".join(json.dumps(row) + "\n" for row in packets), encoding="utf-8")
            native = base / "report.json"
            native.write_text(json.dumps(report), encoding="utf-8")
            output = base / "out.json"
            argv = [sys.executable, "-B", str(SCRIPT), str(trace), "--packets", str(metadata),
                    "--report", str(native), "--output", str(output)]
            first = subprocess.run(argv, capture_output=True, timeout=10)
            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertTrue(output.exists(), "successful CLI must create the requested receipt")
            raw = output.read_bytes()
            second = subprocess.run(argv, capture_output=True, timeout=10)
            self.assertEqual(second.returncode, 1)
            self.assertEqual(output.read_bytes(), raw)
            trace.write_text(text(lines[:-1]), encoding="utf-8")
            refused = base / "refused.json"
            argv[-1] = str(refused)
            self.assertEqual(subprocess.run(argv, capture_output=True, timeout=10).returncode, 1)
            self.assertFalse(refused.exists())
            trace.write_text(text(lines), encoding="utf-8")
            metadata.write_text(json.dumps(packets[0]), encoding="utf-8")
            self.assertEqual(subprocess.run(argv, capture_output=True, timeout=10).returncode, 1)
            self.assertFalse(refused.exists())
            for raw in [(json.dumps(packets[0]) + " " * 1024 + "\n").encode(), b"\n" * 100001,
                        b'{"frames": 1, "frames": 2}\n', b"\xff\n"]:
                metadata.write_bytes(raw)
                result = subprocess.run(argv, capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertFalse(refused.exists())


if __name__ == "__main__":
    unittest.main()
