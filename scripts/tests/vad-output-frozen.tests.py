#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Closed output witnesses do not qualify native delivery or physical clocks."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import numpy as np


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("vad_output_frozen", ROOT / "vad-output-frozen.py")
MODULE = None
if SPEC is not None and SPEC.loader is not None and Path(SPEC.origin).exists():
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)
ENDPOINT = "00112233445566778899aabbccddeeff"


def fixture(frames=16384, fmt=1):
    rng = np.random.default_rng(925)
    samples = rng.integers(-1024, 1024, size=(frames, 2), dtype=np.int16)
    values = samples.astype("<f4") / np.float32(32768)
    raw = samples.astype("<i2").tobytes() if fmt == 1 else values.tobytes()
    width = 4 if fmt == 1 else 8
    packets = []
    for start in range(0, frames, 256):
        packets.append(dict(byte_offset=start*width, byte_count=256*width,
                            actual_frames=256, silence_frames=0, submission="accepted",
                            identity_current_after_submit=True, completion="completed",
                            completion_bytes=256*width, sequence=len(packets)+1,
                            host_submission_qpc=1000+len(packets), error_kind=None,
                            raw_hresult=None))
    meta = dict(schema_version=1, qualified=False, requested_frames=480000,
                stored_frames=frames, raw_bytes=len(raw), worker_join="joined",
                worker_outcome="completed", seal_reason="identity_changed",
                excluded_epoch_packets=0, untracked_completion_observations=0,
                identity=dict(stable_id=list(bytes.fromhex(ENDPOINT)), native_stamp=[1, 2, 3, 4, 5],
                              reset_epoch=0, admission_ticket=2, active_revision=1,
                              format=fmt, channels=2, packet_frames=256), packets=packets)
    rows = [dict(packet_sequence=i+1, frames=256, stored_start_frame=i*256,
                 flags=1 if i == 0 else 0, device_position_frames=1000+i*256,
                 qpc_100ns=10000+i*53333) for i in range(frames//256)]
    return meta, raw, values, rows


def envelope():
    meta, raw, recorded, rows = fixture()
    prepared = dict(stable_endpoint_id=bytes.fromhex(ENDPOINT)[::-1].hex(),
                    mmdevice_id="exact-capture-mmdevice", adapter_instance="exact-adapter",
                    root_instance="exact-control-root")
    route = dict(qualified=False, route="microphone", boundary="internal", bus=True,
                 microphone_stable_endpoint_id=ENDPOINT, recorder_identity_recheck=dict(status="matched"),
                 recorder_endpoint_id="exact-capture-mmdevice", prepared_recorder_endpoint_id="exact-capture-mmdevice",
                 recorder_prepared_microphone=prepared, resolved_microphone=copy.deepcopy(prepared))
    collection = dict(qualified=False, quality_passed=False, collection_complete=False,
                      cleanup_complete=True, failure="original flags retained",
                      graph=dict(route_evidence=route), measurement=dict(
                          info=dict(channels=2, sample_rate=48000, input_endpoint_id="exact-capture-mmdevice"),
                          sink=dict(written_frames=len(recorded), written_pcm_bytes=recorded.nbytes)))
    report = dict(qualified=False, status="unqualified", collection=collection,
                  native=dict(error=None, vad_output_reference=dict(qualified=False,
                              metadata=meta, status="retained_not_delivery_proof", failure=None)))
    data = {"report.json": json.dumps(report).encode(), "collection.json": json.dumps(collection).encode(),
            "vad-output.metadata.json": json.dumps(meta).encode(), "vad-output.raw.bin": raw,
            "measurement.f32le": recorded.tobytes(),
            "measurement.packets.jsonl": b"\n".join(json.dumps(r).encode() for r in rows)}
    return data, meta, report, collection


class WitnessTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "missing actual-output frozen recording adapter")

    def test_pcm16_decode_uses_exact_divisor_and_current_completed_support(self):
        meta, raw, expected, _ = fixture()
        values, valid, generation, receipt = MODULE.decode_output(meta, raw, ENDPOINT)
        np.testing.assert_array_equal(values.view("u4"), expected.view("u4"))
        self.assertTrue(valid.all())
        self.assertTrue((generation == 1).all())
        self.assertEqual(receipt["supported_frames"], len(expected))
        self.assertFalse(receipt["qualified"])

    def test_float32_keeps_signed_zero(self):
        meta, raw, values, _ = fixture(fmt=2)
        values[0, 0] = np.float32(-0.0)
        actual, _, _, _ = MODULE.decode_output(meta, values.tobytes(), ENDPOINT)
        self.assertEqual(actual[0, 0].view("u4"), 0x80000000)

    def test_unresolved_rejected_stale_and_failed_packets_are_unknown_not_zeros(self):
        meta, raw, expected, _ = fixture()
        meta["packets"][1]["completion"] = "retired_completion_unobserved"
        meta["packets"][2]["submission"] = "quota_rejected"
        meta["packets"][2]["completion"] = "not_submitted"
        meta["packets"][2]["completion_bytes"] = None
        meta["packets"][3]["identity_current_after_submit"] = False
        meta["packets"][4]["raw_hresult"] = -1
        actual, valid, _, receipt = MODULE.decode_output(meta, raw, ENDPOINT)
        np.testing.assert_array_equal(actual, expected)
        self.assertFalse(valid[256:1280].any())
        self.assertEqual(receipt["unsupported_frames"], 1024)

    def test_failed_worker_preserves_bytes_but_does_not_supply_known_support(self):
        meta, raw, expected, _ = fixture()
        meta["worker_outcome"] = "failed"
        values, valid, _, receipt = MODULE.decode_output(meta, raw, ENDPOINT)
        np.testing.assert_array_equal(values, expected)
        self.assertFalse(valid.any())
        self.assertEqual(receipt["worker_outcome"], "failed")

    def test_missing_terminal_stamp_is_unknown_even_when_completion_label_says_completed(self):
        for key in ("sequence", "host_submission_qpc"):
            meta, raw, expected, _ = fixture()
            meta["packets"][0][key] = None
            actual, valid, _, receipt = MODULE.decode_output(meta, raw, ENDPOINT)
            np.testing.assert_array_equal(actual, expected)
            self.assertFalse(valid[:256].any(), key)
            self.assertEqual(receipt["unsupported_frames"], 256)

    def test_unjoined_and_wrong_endpoint_refuse(self):
        for key, value in (("worker_join", "unknown"), ("qualified", True)):
            meta, raw, _, _ = fixture()
            meta[key] = value
            with self.assertRaises(ValueError):
                MODULE.decode_output(meta, raw, ENDPOINT)
        meta, raw, _, _ = fixture()
        with self.assertRaises(ValueError):
            MODULE.decode_output(meta, raw, "11"*16)

    def test_boolean_schema_does_not_alias_version_one(self):
        meta, raw, _, _ = fixture()
        meta["schema_version"] = True
        with self.assertRaises(ValueError):
            MODULE.decode_output(meta, raw, ENDPOINT)

    def test_packet_offsets_counts_identity_and_width_cannot_be_forged(self):
        for mutate in (
            lambda m: m["packets"][1].update(byte_offset=0),
            lambda m: m["packets"][1].update(byte_count=100),
            lambda m: m.update(stored_frames=1),
            lambda m: m["identity"].update(channels=1),
            lambda m: m["identity"].update(format=3),
            lambda m: m["packets"][1].update(actual_frames=True),
        ):
            meta, raw, _, _ = fixture()
            mutate(meta)
            with self.assertRaises(ValueError):
                MODULE.decode_output(meta, raw, ENDPOINT)

    def test_zero_fill_and_submission_counters_are_not_hidden(self):
        meta, raw, _, _ = fixture()
        meta["packets"][0]["silence_frames"] = 32
        meta["excluded_epoch_packets"] = 2
        _, _, _, receipt = MODULE.decode_output(meta, raw, ENDPOINT)
        self.assertEqual(receipt["bridge_zero_fill_frames"], 32)
        self.assertEqual(receipt["excluded_epoch_packets"], 2)

    def test_uniform_windows_cover_every_frame_without_content_selection(self):
        for frames in (256, 8192, 8193, 16384, 480000):
            windows = MODULE.windows(frames)
            self.assertEqual(sum(w["frames"] for w in windows), frames)
            end = 0
            for window in windows:
                self.assertEqual(window["start_frame"], end)
                self.assertTrue(256 <= window["frames"] <= 8192)
                end += window["frames"]
            self.assertLessEqual(len(windows), 64)

    def test_packet_flags_and_position_gaps_are_retained_on_stored_axis(self):
        _, _, recorded, rows = fixture()
        rows[1]["device_position_frames"] += 480
        report = MODULE.recorder_packets(rows, len(recorded))
        self.assertEqual(len(report["flagged_packets"]), 1)
        self.assertEqual([r["delta_frames"] for r in report["position_gaps"]], [480, -480])
        self.assertFalse(report["qualified"])

    def test_packet_tail_sequence_and_clock_corruption_refuse(self):
        for mutate in (
            lambda rows: rows.pop(),
            lambda rows: rows[1].update(stored_start_frame=0),
            lambda rows: rows[1].update(packet_sequence=1),
            lambda rows: rows[1].update(qpc_100ns=0),
        ):
            _, _, recorded, rows = fixture()
            mutate(rows)
            with self.assertRaises(ValueError):
                MODULE.recorder_packets(rows, len(recorded))

    def test_continuous_signal_has_informative_heldout_support_without_qualification(self):
        meta, raw, recorded, rows = fixture()
        result = MODULE.analyze(meta, raw, recorded, rows, ENDPOINT)
        self.assertEqual(result["association"]["anchor"]["recorder_to_replay_shift_frames"], 0)
        audit = result["full_recording_audit"]
        self.assertGreater(audit["counts"]["fixed_map_compatible"], 10000)
        self.assertEqual(audit["raw_bits"]["equal_stereo_frames"], len(recorded))
        self.assertEqual(sum(audit["counts"].values()), len(recorded))
        self.assertFalse(result["qualified"])
        self.assertIsNone(result["physical_latency_ms"])

    def test_later_corruption_is_residual_not_refitted_or_dropped(self):
        meta, raw, recorded, rows = fixture()
        recorded[12000:13000] *= -1
        result = MODULE.analyze(meta, raw, recorded, rows, ENDPOINT)
        self.assertEqual(result["association"]["training_attempts"], 1)
        self.assertGreater(result["full_recording_audit"]["counts"]["waveform_residual"], 0)
        self.assertEqual(result["full_recording_audit"]["raw_bits"]["unequal_stereo_frames"], 1000)

    def test_unknown_output_packets_survive_full_recording_denominator(self):
        meta, raw, recorded, rows = fixture()
        meta["packets"][-1]["completion"] = "retired_completion_unobserved"
        result = MODULE.analyze(meta, raw, recorded, rows, ENDPOINT)
        self.assertEqual(result["full_recording_audit"]["counts"]["unknown_replay"], 256)

    def test_nonfinite_recording_retains_denominator_and_explicit_refusal(self):
        meta, raw, recorded, rows = fixture()
        recorded[10000, 0] = np.nan
        result = MODULE.analyze(meta, raw, recorded, rows, ENDPOINT)
        self.assertIsNone(result["association"])
        self.assertIsNone(result["full_recording_audit"])
        self.assertEqual(result["unassessed_frames"], len(recorded))
        self.assertEqual(result["recording_nonfinite_samples"], 1)
        self.assertTrue(result["analysis_refusal"])

    def test_no_anchor_keeps_all_frames_unassessed(self):
        meta, raw, recorded, rows = fixture()
        result = MODULE.analyze(meta, raw, np.zeros_like(recorded), rows, ENDPOINT)
        self.assertIsNone(result["association"]["anchor"])
        self.assertIsNone(result["full_recording_audit"])
        self.assertEqual(result["unassessed_frames"], len(recorded))


class PinTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "missing actual-output frozen recording adapter")

    def test_pinned_reads_require_exact_files_hashes_and_no_duplicate_or_traversal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            names = MODULE.INPUT_NAMES
            pins = []
            for name in names:
                (root / name).write_bytes(b"1234")
                pins.append(dict(name=name, bytes=4, sha256=hashlib.sha256(b"1234").hexdigest()))
            manifest = dict(schema_version=1, files=pins)
            result = MODULE.pinned_inputs(root, manifest)
            self.assertEqual(set(result), set(names))
            for mutate in (
                lambda m: m["files"].append(copy.deepcopy(m["files"][0])),
                lambda m: m["files"][0].update(name="../report.json"),
                lambda m: m["files"][0].update(sha256="0"*64),
                lambda m: m["files"][0].update(bytes=True),
            ):
                bad = copy.deepcopy(manifest)
                mutate(bad)
                with self.assertRaises(ValueError):
                    MODULE.pinned_inputs(root, bad)
            (root / names[0]).write_bytes(b"12345")
            with self.assertRaises(ValueError):
                MODULE.pinned_inputs(root, manifest)


class EnvelopeTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, "missing actual-output frozen recording adapter")

    def test_original_refusal_is_preserved_when_imported_bindings_agree(self):
        data, meta, _, _ = envelope()
        result = MODULE.validate_envelope(data, meta, ENDPOINT, 16384)
        self.assertFalse(result["original_quality_passed"])
        self.assertEqual(result["original_collection_failure"], "original flags retained")

    def test_claimed_matched_status_does_not_override_sibling_prepared_identity(self):
        for key, wrong in (("stable_endpoint_id", "11"*16), ("mmdevice_id", "sibling"),
                           ("adapter_instance", "sibling"), ("root_instance", "sibling")):
            data, meta, report, collection = envelope()
            collection["graph"]["route_evidence"]["resolved_microphone"][key] = wrong
            data["report.json"] = json.dumps(report).encode()
            data["collection.json"] = json.dumps(collection).encode()
            with self.assertRaises(ValueError, msg=key):
                MODULE.validate_envelope(data, meta, ENDPOINT, 16384)

    def test_detached_collection_or_embedded_metadata_difference_refuses(self):
        data, meta, report, _ = envelope()
        report["collection"] = dict(report["collection"], quality_passed=True)
        data["report.json"] = json.dumps(report).encode()
        with self.assertRaises(ValueError):
            MODULE.validate_envelope(data, meta, ENDPOINT, 16384)
        data, meta, report, _ = envelope()
        report["native"]["vad_output_reference"]["metadata"] = dict(meta, stored_frames=1)
        data["report.json"] = json.dumps(report).encode()
        with self.assertRaises(ValueError):
            MODULE.validate_envelope(data, meta, ENDPOINT, 16384)

    def test_duplicate_json_keys_are_not_silently_overwritten(self):
        with self.assertRaises(ValueError):
            MODULE.parse_json(b'{"qualified":true,"qualified":false}')

    def test_cli_fresh_outputs_pin_recheck_and_no_native_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            inputs = root / "inputs"
            inputs.mkdir()
            data, _, _, _ = envelope()
            for name, raw in data.items():
                (inputs / name).write_bytes(raw)
            pins = root / "pins.json"
            pins.write_text(json.dumps(dict(schema_version=1, files=[dict(
                name=n, bytes=len(b), sha256=hashlib.sha256(b).hexdigest()) for n, b in data.items()])))
            output = root / "output"
            args = ["--input-dir", str(inputs), "--pins", str(pins), "--output", str(output),
                    "--microphone-id", ENDPOINT]
            with mock.patch.object(MODULE, "plot") as plot:
                MODULE.main(args)
                plot.assert_called_once()
                result = json.loads((output / "summary.json").read_bytes())
                self.assertEqual(result["recorded_frames"], 16384)
                self.assertFalse(result["qualified"])
                with self.assertRaises(FileExistsError):
                    MODULE.main(args)
            output2 = root / "output2"
            args[args.index(str(output))] = str(output2)
            def mutate(*_):
                (inputs / "measurement.f32le").write_bytes(b"changed")
            with mock.patch.object(MODULE, "plot", side_effect=mutate):
                with self.assertRaises(ValueError):
                    MODULE.main(args)

    def test_plot_categories_have_distinct_colors(self):
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.axes
        import matplotlib.figure
        # pyplot copies Figure.savefig metadata during import. Initialize it
        # before replacing that method with a mock lacking function metadata.
        import matplotlib.pyplot
        meta, raw, recorded, rows = fixture()
        recorded[:512] = 0
        result = MODULE.analyze(meta, raw, recorded, rows, ENDPOINT)
        self.assertIsNotNone(result["full_recording_audit"])
        original = matplotlib.axes.Axes.broken_barh
        colors = []
        def capture(axis, *args, **kwargs):
            colors.append(kwargs.get("facecolors"))
            return original(axis, *args, **kwargs)
        with mock.patch.object(matplotlib.axes.Axes, "broken_barh", new=capture), \
                mock.patch.object(matplotlib.figure.Figure, "savefig"):
            MODULE.plot(result, recorded, Path("not-written.png"))
        self.assertGreater(len(colors), 1)
        self.assertEqual(len(set(colors)), len(colors))


if __name__ == "__main__":
    unittest.main()
