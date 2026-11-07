#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Independent offered/accepted mapping and actual Process SRC replay tests."""

import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import numpy as np


SCRIPT = Path(__file__).resolve().parents[1] / "process-native-pcm.py"
SPEC = (
    importlib.util.spec_from_file_location("process_native_pcm", SCRIPT)
    if SCRIPT.exists()
    else None
)
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if MODULE is not None:
    SPEC.loader.exec_module(MODULE)


def bits(value):
    return str(struct.unpack("<Q", struct.pack("<d", value))[0])


def event(sequence=1, **changes):
    span = dict(
        read_sequence=1,
        output_start_frame=0,
        input_start_frame=0,
        frames=4,
        generation=1,
        phase_bits=bits(0.125),
        step_bits=bits(1.0),
        fade_remaining=0,
    )
    span.update(changes)
    return dict(sequence=sequence, span=span)


def store_json(path, value):
    path.write_text(json.dumps(value, allow_nan=False), encoding="utf-8")


def fixture(root, *, packets=None, events=None, requested=None, reads=None, channels=2):
    """Write the current Rust PCM/trace protocol, not a synthetic alternate schema."""
    root.mkdir()
    pcm = root / "native-pcm"
    pcm.mkdir()
    if packets is None:
        samples = np.column_stack(
            (np.linspace(-0.8, 0.9, 40), np.linspace(0.6, -0.7, 40))
        ).astype("<f4")
        packets = [(samples, 40, 1, 0)]
    if events is None:
        events = [event()]
    requested = (
        requested
        if requested is not None
        else max(
            row["span"]["output_start_frame"] + row["span"]["frames"] for row in events
        )
    )
    reads = (
        reads
        if reads is not None
        else max(row["span"]["read_sequence"] for row in events)
    )
    origin = dict(
        schema_version=1,
        owner_generation=1,
        node="NodeId(1v1)",
        process=dict(pid=123, creation_time_100ns=456, executable_name="source.exe"),
        input_sample_rate=48000,
        consumer_sample_rate=48000,
        prepared_consumer_demand_frames=64,
        admitted_bridge={},
    )
    store_json(root / "origin.json", origin)
    offered = accepted = 0
    rows = []
    with (pcm / "samples.f32le").open("wb") as raw:
        for sequence, (samples, count, generation, flags) in enumerate(packets, 1):
            samples = np.asarray(samples, dtype="<f4")
            raw.write(samples.tobytes())
            rows.append(
                dict(
                    packet_sequence=sequence,
                    offered_before_frames=offered,
                    accepted_before_frames=accepted,
                    packet_frames=len(samples),
                    packet_accepted_frames=count,
                    flags=flags,
                    generation=generation,
                    device_position_frames=1000 + offered,
                    qpc_100ns=100000 + offered,
                    stored_before_frames=offered,
                )
            )
            offered += len(samples)
            accepted += count
    (pcm / "packets.jsonl").write_text(
        "".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8"
    )
    summary = dict(
        capacity_frames=4096,
        sample_rate=48000,
        channels=channels,
        packets=len(rows),
        offered_frames=offered,
        accepted_frames=accepted,
        stored_frames=offered,
        lost_frames=0,
        overflow_packets=0,
        reader_abandoned_packets=0,
        saturated=False,
        writer_finished=True,
    )
    store_json(
        pcm / "stream.json",
        dict(
            schema_version=1,
            sample_format="f32le-bits",
            frame_bytes=8,
            channels=channels,
            sample_rate=48000,
            disk_limit_bytes=512 * 1024 * 1024,
            summary=summary,
            qualified=False,
        ),
    )
    store_json(
        pcm / "final.json",
        dict(
            schema_version=1,
            summary=summary,
            persisted_frames=offered,
            persisted_packets=len(rows),
            persisted_bytes=(pcm / "samples.f32le").stat().st_size
            + (pcm / "packets.jsonl").stat().st_size,
            partial_packet=None,
            pending_frames=0,
            error=None,
            persistence_complete=True,
            qualified=False,
        ),
    )
    if events:
        store_json(
            root / "chunk-00000001.json",
            dict(schema_version=1, sequence=1, events=events),
        )
    chunks = 1 if events else 0
    artifact = dict(
        schema_version=1,
        event_limit=4096,
        retained_events=0,
        drained_events=len(events),
        drained_chunks=chunks,
        total_events=len(events),
        missing_events=0,
        unretained_events=0,
        requested_frames=requested,
        started_reads=reads,
        completed_reads=reads,
        writer_finished=True,
        finalized=True,
        history_complete=True,
        qualified=False,
        refusal=None,
        events=[],
    )
    store_json(
        root / "final.json",
        dict(
            schema_version=1,
            origin=origin,
            artifact=artifact,
            final_history_available=True,
            unavailable_polls=0,
            persisted_events=len(events),
            persisted_chunks=chunks,
            io_error=None,
            native_pcm_enabled=True,
            native_pcm_complete=True,
            persistence_complete=True,
            qualified=False,
        ),
    )
    return root


def change_json(path, mutate):
    value = json.loads(path.read_text(encoding="utf-8"))
    mutate(value)
    store_json(path, value)


class NativePcmTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="moiren-process-pcm-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def audit(self, owner, **kwargs):
        self.assertIsNotNone(
            MODULE, "missing independent native PCM audit and actual SRC replay"
        )
        return MODULE.audit_owner(owner, self.root / "audit", make_plot=False, **kwargs)

    def output(self, name, dtype, channels=None):
        result = np.fromfile(self.root / "audit" / name, dtype=dtype)
        return result.reshape((-1, channels)) if channels else result

    def test_actual_recurrence_not_phase_plus_k_step_with_fade_and_exact_bits(self):
        phase, step = 0.231, 1.000123456789
        samples = np.column_stack((np.arange(50) / 60, -np.arange(50) / 75)).astype(
            "<f4"
        )
        owner = fixture(
            self.root / "owner",
            packets=[(samples, 50, 1, 0)],
            events=[
                event(
                    frames=32,
                    phase_bits=bits(phase),
                    step_bits=bits(step),
                    fade_remaining=32,
                )
            ],
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        self.assertFalse(result["qualified"])
        self.assertIsNone(result["physical_latency_ms"])
        expected = []
        coordinates = []
        index = 0
        for offset in range(32):
            while phase >= 1.0:
                phase -= 1.0
                index += 1
            coordinates.append(index + phase)
            expected.append(
                [
                    np.float32(
                        (
                            float(samples[index, ch]) * (1.0 - phase)
                            + float(samples[index + 1, ch]) * phase
                        )
                        * ((offset + 1) / 32)
                    )
                    for ch in range(2)
                ]
            )
            phase += step
        actual = self.output("reconstructed.f32le", "<f4", 2)
        np.testing.assert_array_equal(
            actual.view("<u4"), np.asarray(expected, dtype="<f4").view("<u4")
        )
        coords = self.output("cursor-input.f64le", "<f8")
        np.testing.assert_array_equal(
            coords.view("<u8"), np.asarray(coordinates, dtype="<f8").view("<u8")
        )
        naive = 0.231 + np.arange(32) * step
        self.assertTrue(
            np.any(coords.view("<u8") != naive.view("<u8")),
            "fixture must expose affine shortcut",
        )

    def test_offered_tail_is_not_reindexed_into_accepted_input(self):
        a = np.asarray([[0.1, 0.2], [0.3, 0.4], [999, 999], [888, 888]], dtype="<f4")
        b = np.asarray([[0.5, 0.6], [0.7, 0.8], [0.9, 1.0]], dtype="<f4")
        owner = fixture(
            self.root / "owner",
            packets=[(a, 2, 1, 0), (b, 3, 2, 0)],
            events=[
                event(input_start_frame=2, generation=2, frames=2, phase_bits=bits(0.0))
            ],
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        self.assertEqual(result["native_pcm"]["dropped_tail_frames"], 2)
        np.testing.assert_array_equal(
            self.output("accepted.f32le", "<f4", 2), np.vstack((a[:2], b))
        )
        np.testing.assert_array_equal(
            self.output("reconstructed.f32le", "<f4", 2), b[:2]
        )

    def test_mono_nonfinite_signed_zero_and_silent_normalization(self):
        a = np.asarray(
            [[float("nan"), 0], [float("inf"), 0], [-0.0, 0], [0.75, 0]], dtype="<f4"
        )
        owner = fixture(
            self.root / "owner",
            channels=1,
            packets=[(a, 4, 1, 0), (np.zeros((3, 2)), 3, 1, 2)],
            events=[
                event(frames=3),
                event(
                    2,
                    read_sequence=2,
                    output_start_frame=3,
                    input_start_frame=4,
                    frames=2,
                ),
            ],
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        expected = np.asarray(
            [[0, 0], [0, 0], [-0.0, -0.0], [0.75, 0.75], [0, 0], [0, 0], [0, 0]],
            dtype="<f4",
        )
        np.testing.assert_array_equal(
            self.output("accepted.f32le", "<f4", 2).view("<u4"), expected.view("<u4")
        )
        self.assertEqual(result["native_pcm"]["normalized_nonfinite_samples"], 2)
        self.assertEqual(
            (owner / "native-pcm" / "samples.f32le").read_bytes()[:8], a.tobytes()[:8]
        )

    def test_requested_holes_have_explicit_unknown_mask_never_audio_success(self):
        owner = fixture(
            self.root / "owner",
            events=[event(output_start_frame=3, frames=4, read_sequence=2)],
            requested=12,
            reads=3,
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        self.assertEqual(result["replay"]["unknown_or_priming_frames"], 8)
        self.assertFalse(result["replay"]["all_requested_frames_reconstructed"])
        np.testing.assert_array_equal(
            self.output("reconstructed-valid.u8", "u1"),
            [0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0],
        )
        self.assertTrue(
            np.all(
                self.output("reconstructed.f32le", "<f4", 2)[[0, 1, 2, 7, 8, 9, 10, 11]]
                == 0
            )
        )

    def test_generation_crossing_pair_refuses_even_if_start_matches(self):
        owner = fixture(
            self.root / "owner",
            packets=[(np.ones((2, 2)), 2, 1, 0), (np.ones((4, 2)), 4, 2, 0)],
            events=[event(input_start_frame=1, frames=1)],
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "refused", result)
        self.assertIn("generation", " ".join(result["refusals"]))

    def test_bad_packet_metadata_refuses(self):
        mutations = {
            "sequence": lambda r: r.update(packet_sequence=2),
            "offered": lambda r: r.update(offered_before_frames=1),
            "accepted": lambda r: r.update(accepted_before_frames=1),
            "stored": lambda r: r.update(stored_before_frames=1),
            "length": lambda r: r.update(packet_frames=41),
            "tail": lambda r: r.update(packet_accepted_frames=41),
            "zero_generation": lambda r: r.update(generation=0),
            "bool_index": lambda r: r.update(stored_before_frames=False),
        }
        for name, mutation in mutations.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                owner = fixture(root / "owner")
                path = owner / "native-pcm" / "packets.jsonl"
                row = json.loads(path.read_text())
                mutation(row)
                path.write_text(json.dumps(row) + "\n", encoding="utf-8")
                self.assertIsNotNone(MODULE, "missing native PCM auditor")
                result = MODULE.audit_owner(owner, root / "audit", make_plot=False)
                self.assertEqual(result["status"], "refused", result)

    def test_final_lost_saturated_open_io_cap_partial_mismatches_refuse(self):
        mutations = [
            lambda x: x["summary"].update(lost_frames=1),
            lambda x: x["summary"].update(overflow_packets=1),
            lambda x: x["summary"].update(reader_abandoned_packets=1),
            lambda x: x["summary"].update(saturated=True),
            lambda x: x["summary"].update(writer_finished=False),
            lambda x: x.update(error="IO or disk cap failed"),
            lambda x: x.update(partial_packet={"packet_sequence": 1}),
            lambda x: x.update(pending_frames=1),
            lambda x: x.update(persisted_frames=39),
            lambda x: x.update(persistence_complete=False),
        ]
        for mutation in mutations:
            with (
                self.subTest(mutation=mutation),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                owner = fixture(root / "owner")
                change_json(owner / "native-pcm" / "final.json", mutation)
                self.assertIsNotNone(MODULE, "missing native PCM auditor")
                self.assertEqual(
                    MODULE.audit_owner(owner, root / "audit", make_plot=False)[
                        "status"
                    ],
                    "refused",
                )

    def test_cursor_gap_overlap_invalid_bits_and_bad_indices_refuse(self):
        mutations = [
            lambda e: e[0].update(sequence=2),
            lambda e: e[0]["span"].update(frames=33),
            lambda e: e[0]["span"].update(input_start_frame=40),
            lambda e: e[0]["span"].update(phase_bits=0),
            lambda e: e[0]["span"].update(phase_bits=bits(float("nan"))),
            lambda e: e[0]["span"].update(phase_bits=bits(1.0)),
            lambda e: e[0]["span"].update(step_bits=bits(-1.0)),
            lambda e: e[0]["span"].update(fade_remaining=33),
            lambda e: e[0]["span"].update(generation=2),
            lambda e: e.append(event(2, output_start_frame=2, input_start_frame=4)),
            lambda e: e.append(event(2, output_start_frame=5, input_start_frame=4)),
        ]
        for mutation in mutations:
            with (
                self.subTest(mutation=mutation),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                owner = fixture(root / "owner")
                change_json(
                    owner / "chunk-00000001.json", lambda x: mutation(x["events"])
                )
                self.assertIsNotNone(MODULE, "missing native PCM auditor")
                self.assertEqual(
                    MODULE.audit_owner(owner, root / "audit", make_plot=False)[
                        "status"
                    ],
                    "refused",
                )

    def test_contiguous_same_read_phase_must_follow_repeated_recurrence(self):
        phase, step = 0.231, 1.000123456789
        p = phase
        index = 0
        for _ in range(4):
            while p >= 1:
                p -= 1
                index += 1
            p += step
        while p >= 1:
            p -= 1
            index += 1
        events = [
            event(frames=4, phase_bits=bits(phase), step_bits=bits(step)),
            event(
                2,
                frames=3,
                output_start_frame=4,
                input_start_frame=index,
                phase_bits=bits(p),
                step_bits=bits(step),
            ),
        ]
        owner = fixture(self.root / "owner", events=events)
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        change_json(
            owner / "chunk-00000001.json",
            lambda x: x["events"][1]["span"].update(phase_bits=bits(p + 1e-12)),
        )
        self.assertEqual(
            MODULE.audit_owner(owner, self.root / "refused", make_plot=False)["status"],
            "refused",
        )

    def test_incomplete_cursor_final_refuses(self):
        mutations = [
            lambda x: x["artifact"].update(missing_events=1),
            lambda x: x["artifact"].update(unretained_events=1),
            lambda x: x["artifact"].update(refusal="saturated"),
            lambda x: x["artifact"].update(completed_reads=0),
            lambda x: x["artifact"].update(writer_finished=False),
            lambda x: x["artifact"].update(retained_events=1),
            lambda x: x.update(io_error="disk failure"),
            lambda x: x.update(final_history_available=False),
            lambda x: x.update(native_pcm_complete=False),
            lambda x: x["origin"].update(owner_generation=2),
        ]
        for mutation in mutations:
            with (
                self.subTest(mutation=mutation),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                owner = fixture(root / "owner")
                change_json(owner / "final.json", mutation)
                self.assertIsNotNone(MODULE, "missing native PCM auditor")
                self.assertEqual(
                    MODULE.audit_owner(owner, root / "audit", make_plot=False)[
                        "status"
                    ],
                    "refused",
                )

    def test_partial_raw_and_extra_chunks_refuse_and_preserve_inputs(self):
        owner = fixture(self.root / "owner")
        path = owner / "native-pcm" / "samples.f32le"
        damaged = path.read_bytes()[:-1]
        path.write_bytes(damaged)
        result = self.audit(owner)
        self.assertEqual(result["status"], "refused")
        self.assertEqual(path.read_bytes(), damaged)
        self.assertTrue((self.root / "audit" / "summary.json").exists())

    def test_extra_cursor_chunk_cannot_be_ignored(self):
        owner = fixture(self.root / "owner")
        store_json(owner / "chunk-00000002.json", {"sequence": 2, "events": []})
        self.assertEqual(self.audit(owner)["status"], "refused")

    def test_native_generation_regression_and_bad_mono_silent_bits_refuse(self):
        cases = [
            (1, [(np.ones((3, 2)), 3, 1, 0)]),
            (2, [(np.ones((3, 2)), 3, 1, 2)]),
            (2, [(np.ones((3, 2)), 3, 2, 0), (np.ones((3, 2)), 3, 1, 0)]),
        ]
        for channels, packets in cases:
            with (
                self.subTest(channels=channels, packets=len(packets)),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                owner = fixture(
                    root / "owner",
                    channels=channels,
                    packets=packets,
                    events=[event(frames=1, generation=packets[0][2])],
                )
                self.assertIsNotNone(MODULE, "missing native PCM auditor")
                self.assertEqual(
                    MODULE.audit_owner(owner, root / "audit", make_plot=False)[
                        "status"
                    ],
                    "refused",
                )

    def test_corrupt_partial_duplicate_and_unbounded_metadata_refuse(self):
        for name, content in [
            ("truncated", b'{"packet_sequence":'),
            ("partial_line", b"{}"),
            ("duplicate", b'{"packet_sequence":1,"packet_sequence":1}\n'),
            ("long_line", b" " * 8192 + b"\n"),
        ]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                owner = fixture(root / "owner")
                (owner / "native-pcm" / "packets.jsonl").write_bytes(content)
                change_json(
                    owner / "native-pcm" / "final.json",
                    lambda x: x.update(
                        persisted_bytes=(owner / "native-pcm" / "samples.f32le")
                        .stat()
                        .st_size
                        + len(content)
                    ),
                )
                self.assertIsNotNone(MODULE, "missing native PCM auditor")
                self.assertEqual(
                    MODULE.audit_owner(owner, root / "audit", make_plot=False)[
                        "status"
                    ],
                    "refused",
                )

    def test_contiguous_same_read_fade_mismatch_refuses(self):
        events = [
            event(frames=4, phase_bits=bits(0.0), fade_remaining=32),
            event(
                2,
                frames=3,
                output_start_frame=4,
                input_start_frame=4,
                phase_bits=bits(0.0),
                fade_remaining=27,
            ),
        ]
        owner = fixture(self.root / "owner", events=events)
        result = self.audit(owner)
        self.assertEqual(result["status"], "refused", result)
        self.assertIn("fade", " ".join(result["refusals"]))

    def test_unsupported_huge_step_refuses_without_subtraction_hang(self):
        owner = fixture(self.root / "owner", events=[event(step_bits=bits(1e200))])
        self.assertEqual(self.audit(owner)["status"], "refused")

    def test_root_owners_not_deduplicated_by_reused_generation(self):
        self.assertIsNotNone(MODULE, "missing native PCM auditor")
        archive = self.root / "archive"
        archive.mkdir()
        for ordinal in (1, 2):
            fixture(archive / ("owner-%04d-generation-1" % ordinal))
        store_json(
            archive / "summary.json",
            dict(
                schema_version=1,
                sessions=2,
                expected_owners=2,
                native_pcm_enabled=True,
                error=None,
                qualified=False,
            ),
        )
        result = MODULE.audit_archive(archive, self.root / "audit", make_plot=False)
        self.assertEqual(result["status"], "structurally_complete", result)
        self.assertEqual(len(result["owners"]), 2)
        self.assertTrue(all(row["qualified"] is False for row in result["owners"]))

    def test_root_missing_replacement_owner_refuses(self):
        self.assertIsNotNone(MODULE, "missing native PCM auditor")
        archive = self.root / "archive"
        archive.mkdir()
        fixture(archive / "owner-0001-generation-1")
        store_json(
            archive / "summary.json",
            dict(
                schema_version=1,
                sessions=2,
                expected_owners=2,
                native_pcm_enabled=True,
                error=None,
                qualified=False,
            ),
        )
        result = MODULE.audit_archive(archive, self.root / "audit", make_plot=False)
        self.assertEqual(result["status"], "refused", result)

    def test_refused_cli_returns_nonzero_and_preserves_failure_png(self):
        self.assertIsNotNone(MODULE, "missing native PCM auditor")
        owner = fixture(self.root / "owner")
        change_json(
            owner / "native-pcm" / "final.json",
            lambda x: x["summary"].update(lost_frames=12),
        )
        output = self.root / "cli-refused"
        run = subprocess.run(
            [
                sys.executable,
                "-B",
                str(SCRIPT),
                "--archive-directory",
                str(owner),
                "--output-directory",
                str(output),
            ],
            capture_output=True,
            text=True,
            timeout=45,
        )
        self.assertEqual(run.returncode, 2, run.stderr + run.stdout)
        result = json.loads((output / "summary.json").read_text())
        self.assertEqual(result["status"], "refused")
        self.assertFalse(result["qualified"])
        self.assertEqual(
            (output / "overview.png").read_bytes()[:8], b"\x89PNG\r\n\x1a\n"
        )

    def test_status_remains_refused_if_replay_dies_after_valid_prefix(self):
        events = [
            event(frames=4),
            event(
                2,
                frames=2,
                output_start_frame=4,
                input_start_frame=4,
                phase_bits=bits(0.125),
                generation=2,
            ),
        ]
        owner = fixture(self.root / "owner", events=events)
        result = self.audit(owner)
        self.assertEqual(result["status"], "refused", result)
        self.assertTrue(result["replay"]["partial_outputs_untrusted"])
        np.testing.assert_array_equal(
            self.output("reconstructed-valid.u8", "u1"), [1, 1, 1, 1, 0, 0]
        )

    def test_downsampling_normalizes_before_each_sample(self):
        samples = np.column_stack((np.arange(40) / 50, np.arange(40) / -60)).astype(
            "<f4"
        )
        owner = fixture(
            self.root / "owner",
            packets=[(samples, 40, 1, 0)],
            events=[event(frames=12, phase_bits=bits(0.8), step_bits=bits(2.3))],
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        phase, index, expected = 0.8, 0, []
        for _ in range(12):
            while phase >= 1:
                phase -= 1
                index += 1
            expected.append(index + phase)
            phase += 2.3
        np.testing.assert_array_equal(
            self.output("cursor-input.f64le", "<f8").view("<u8"),
            np.asarray(expected, dtype="<f8").view("<u8"),
        )

    def test_same_read_downsampling_can_skip_generation_boundary_without_reset(self):
        events = [
            event(frames=1, phase_bits=bits(0.0), step_bits=bits(2.0)),
            event(
                2,
                output_start_frame=1,
                input_start_frame=2,
                frames=1,
                generation=2,
                phase_bits=bits(0.0),
                step_bits=bits(2.0),
            ),
        ]
        owner = fixture(
            self.root / "owner",
            packets=[(np.ones((2, 2)), 2, 1, 0), (np.full((4, 2), 0.75), 4, 2, 0)],
            events=events,
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        np.testing.assert_array_equal(
            self.output("reconstructed.f32le", "<f4", 2), [[1, 1], [0.75, 0.75]]
        )

    def test_all_zero_output_reads_remain_entirely_unknown_not_audio_success(self):
        owner = fixture(self.root / "owner", events=[], requested=64, reads=4)
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        self.assertEqual(result["replay"]["reconstructed_frames"], 0)
        self.assertEqual(result["replay"]["unknown_or_priming_frames"], 64)
        self.assertFalse(result["replay"]["all_requested_frames_reconstructed"])
        self.assertFalse(result["qualified"])
        self.assertTrue(np.all(self.output("reconstructed-valid.u8", "u1") == 0))
        self.assertTrue(np.all(np.isnan(self.output("cursor-input.f64le", "<f8"))))

    def test_intermediate_zero_output_reads_and_terminal_tail_remain_unknown(self):
        events = [
            event(frames=2, read_sequence=1),
            event(
                2,
                frames=3,
                read_sequence=3,
                output_start_frame=10,
                input_start_frame=4,
                fade_remaining=32,
            ),
        ]
        owner = fixture(self.root / "owner", events=events, requested=20, reads=4)
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        mask = self.output("reconstructed-valid.u8", "u1")
        np.testing.assert_array_equal(mask, [1, 1] + [0] * 8 + [1] * 3 + [0] * 7)
        self.assertEqual(result["replay"]["unknown_or_priming_frames"], 15)

    def test_root_overview_lists_both_owners_not_blank_axes(self):
        self.assertIsNotNone(MODULE, "missing native PCM auditor")
        archive = self.root / "archive"
        archive.mkdir()
        fixture(archive / "owner-0001-generation-1")
        fixture(archive / "owner-0002-generation-1", events=[], requested=8, reads=1)
        store_json(
            archive / "summary.json",
            dict(
                schema_version=1,
                sessions=2,
                expected_owners=2,
                native_pcm_enabled=True,
                error=None,
                qualified=False,
            ),
        )
        result = MODULE.audit_archive(archive, self.root / "audit", make_plot=True)
        self.assertEqual(result["status"], "structurally_complete", result)
        self.assertEqual(result["owners"][1]["replay"]["unknown_or_priming_frames"], 8)
        self.assertEqual(result.get("overview_kind"), "per_owner_coverage_ledger")

    def test_between_read_recovery_may_reset_phase_and_fade_at_next_frontier(self):
        events = [
            event(frames=1, phase_bits=bits(0.25), step_bits=bits(1.0)),
            event(
                2,
                read_sequence=2,
                output_start_frame=1,
                input_start_frame=1,
                frames=2,
                phase_bits=bits(0.0),
                step_bits=bits(1.0),
                fade_remaining=32,
            ),
        ]
        owner = fixture(self.root / "owner", events=events)
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)

    def test_all_native_packets_unaccepted_still_no_invented_replay(self):
        owner = fixture(
            self.root / "owner",
            packets=[(np.ones((4, 2)), 0, 1, 0)],
            events=[],
            requested=16,
            reads=1,
        )
        result = self.audit(owner)
        self.assertEqual(result["status"], "structurally_complete", result)
        self.assertEqual(result["native_pcm"]["dropped_tail_frames"], 4)
        self.assertEqual(result["outputs"]["accepted.f32le"]["bytes"], 0)
        self.assertTrue(np.all(self.output("reconstructed-valid.u8", "u1") == 0))

    def test_missing_required_error_receipt_is_not_successful_closure(self):
        owner = fixture(self.root / "owner")
        change_json(owner / "native-pcm" / "final.json", lambda x: x.pop("error"))
        result = self.audit(owner)
        self.assertEqual(result["status"], "refused", result)

    def test_report_stays_compact_with_large_untrusted_origin_policy(self):
        owner = fixture(self.root / "owner")
        change_json(
            owner / "origin.json",
            lambda x: x["admitted_bridge"].update(unknown="x" * 100000),
        )
        change_json(
            owner / "final.json",
            lambda x: x["origin"]["admitted_bridge"].update(unknown="x" * 100000),
        )
        self.audit(owner)
        self.assertLess((self.root / "audit" / "summary.json").stat().st_size, 65536)

    def test_input_cap_and_requested_output_bomb_refuse_before_allocation(self):
        owner = fixture(self.root / "owner")
        self.assertEqual(self.audit(owner, input_limit_bytes=16)["status"], "refused")
        change_json(
            owner / "final.json",
            lambda x: x["artifact"].update(requested_frames=(1 << 63)),
        )
        self.assertEqual(
            MODULE.audit_owner(owner, self.root / "bomb", make_plot=False)["status"],
            "refused",
        )
        self.assertFalse((self.root / "bomb" / "reconstructed.f32le").exists())

    def test_existing_output_never_overwritten(self):
        owner = fixture(self.root / "owner")
        self.audit(owner)
        self.assertRaises(
            FileExistsError,
            MODULE.audit_owner,
            owner,
            self.root / "audit",
            make_plot=False,
        )

    def test_array_flush_failure_closes_all_maps_before_publishing_a_refusal(self):
        owner = fixture(self.root / "owner")
        output = self.root / "audit"
        original_flush = MODULE.np.memmap.flush
        original_create = MODULE.Arrays.create
        original_read = MODULE.Arrays.read
        maps = []

        def close_test_maps():
            for value in maps:
                if not value._mmap.closed:
                    value._mmap.close()

        self.addCleanup(close_test_maps)

        def track(original):
            def call(arrays, *args, **kwargs):
                value = original(arrays, *args, **kwargs)
                if isinstance(value, MODULE.np.memmap):
                    maps.append(value)
                return value

            return call

        def flush(value):
            if Path(value.filename).name == "reconstructed-valid.u8":
                raise OSError("synthetic output flush failure")
            return original_flush(value)

        with (
            patch.object(MODULE.Arrays, "create", track(original_create)),
            patch.object(MODULE.Arrays, "read", track(original_read)),
            patch.object(MODULE.np.memmap, "flush", flush),
        ):
            code = MODULE.main(
                [
                    "--archive-directory",
                    str(owner),
                    "--output-directory",
                    str(output),
                    "--no-plot",
                ]
            )
        self.assertEqual(code, 2)
        result = json.loads((output / "summary.json").read_text())
        self.assertEqual(result["status"], "refused")
        self.assertTrue(result["replay"]["partial_outputs_untrusted"])
        self.assertTrue(
            any("synthetic output flush failure" in reason for reason in result["refusals"])
        )
        self.assertFalse(result["qualified"])
        self.assertIsNone(result["physical_latency_ms"])
        self.assertTrue(maps)
        self.assertTrue(all(value._mmap.closed for value in maps))

    def test_overflowed_json_number_preserves_a_valid_refusal_receipt(self):
        owner = fixture(self.root / "owner")
        for name in ("origin.json", "final.json"):
            path = owner / name
            path.write_text(
                path.read_text().replace(
                    '"admitted_bridge": {}',
                    '"admitted_bridge": {"age_limit_100ns": 1e400}',
                ),
                encoding="utf-8",
            )
        try:
            self.audit(owner)
        except ValueError:
            pass
        try:
            result = json.loads((self.root / "audit" / "summary.json").read_text())
        except json.JSONDecodeError as error:
            self.fail("refusal receipt must remain valid JSON: " + str(error))
        self.assertEqual(result["status"], "refused")
        self.assertTrue(result["refusals"])
        self.assertFalse(result["qualified"])
        self.assertIsNone(result["physical_latency_ms"])

    def test_root_retains_failed_persistence_and_continues_other_owner_audits(self):
        archive = self.root / "archive"
        archive.mkdir()
        names = ["owner-0001-generation-1", "owner-0002-generation-1"]
        for name in names:
            fixture(archive / name)
        store_json(
            archive / "summary.json",
            dict(
                schema_version=1,
                sessions=2,
                expected_owners=2,
                native_pcm_enabled=True,
                error=None,
                qualified=False,
            ),
        )
        original_flush = MODULE.np.memmap.flush

        def flush(value):
            path = Path(value.filename)
            if path.name == "accepted.f32le" and path.parent.name == names[0]:
                raise OSError("synthetic first owner flush failure")
            return original_flush(value)

        with patch.object(MODULE.np.memmap, "flush", flush):
            result = MODULE.audit_archive(archive, self.root / "audit", make_plot=False)
        self.assertEqual(result["status"], "refused")
        self.assertEqual(
            [item["status"] for item in result["owners"]],
            ["refused", "structurally_complete"],
        )
        persisted = json.loads((self.root / "audit" / "summary.json").read_text())
        self.assertEqual(persisted, result)
        for name, status in zip(names, ["refused", "structurally_complete"]):
            owner = json.loads((self.root / "audit" / name / "summary.json").read_text())
            self.assertEqual(owner["status"], status)
            self.assertFalse(owner["qualified"])

    def test_cli_creates_compact_json_and_png_without_physical_claim(self):
        self.assertIsNotNone(MODULE, "missing native PCM auditor")
        owner = fixture(self.root / "owner")
        output = self.root / "cli"
        run = subprocess.run(
            [
                sys.executable,
                "-B",
                str(SCRIPT),
                "--archive-directory",
                str(owner),
                "--output-directory",
                str(output),
            ],
            capture_output=True,
            text=True,
            timeout=45,
        )
        self.assertEqual(run.returncode, 0, run.stderr + run.stdout)
        result = json.loads((output / "summary.json").read_text())
        self.assertFalse(result["qualified"])
        self.assertLess((output / "summary.json").stat().st_size, 65536)
        self.assertEqual(
            (output / "overview.png").read_bytes()[:8], b"\x89PNG\r\n\x1a\n"
        )


if __name__ == "__main__":
    unittest.main()
