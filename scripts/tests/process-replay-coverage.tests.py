#!/usr/bin/env python3
"""Fixed recorder maps must not repair away interruptions or missing coverage."""

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import numpy as np


sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().parents[1] / "process-replay-coverage.py"
SPEC = (
    importlib.util.spec_from_file_location("replay_coverage", SCRIPT)
    if SCRIPT.exists()
    else None
)
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(MODULE)


class CoverageTests(unittest.TestCase):
    rate = 8000

    def module(self):
        self.assertIsNotNone(MODULE, "missing fixed-map continuous coverage analyzer")
        return MODULE

    def fixture(self, *, ppm=0, offset=160.25):
        replay = np.random.default_rng(188).normal(0, 0.03, (24000, 2)).astype("<f4")
        positions = (np.arange(16000) - offset) * (1 + ppm * 1e-6)
        recorded = np.column_stack(
            [
                np.interp(
                    positions, np.arange(len(replay)), replay[:, c], left=0, right=0
                )
                for c in range(2)
            ]
        )
        valid = np.ones(len(replay), dtype="u1")
        anchor = dict(
            owner="owner-1",
            recording_start_frame=4000,
            channels=[
                dict(
                    recording_center_frame=5999.5,
                    replay_frame_at_recording_center=(5999.5 - offset)
                    * (1 + ppm * 1e-6),
                    correction_ppm=ppm,
                    gain=1.0,
                    dc=0.0,
                )
                for _ in range(2)
            ],
        )
        return replay, valid, recorded, anchor

    def scan(self, replay, valid, recorded, anchor):
        return self.module().scan_coverage(
            {"owner-1": (replay, valid)}, recorded, [anchor], self.rate
        )

    def test_known_fractional_delay_and_drift_use_one_frozen_map(self):
        replay, valid, recorded, anchor = self.fixture(ppm=333)
        result = self.scan(replay, valid, recorded, anchor)
        # The first half-second includes the unsupported leading edge; its
        # remaining blocks conservatively lack a complete aggregate context.
        self.assertGreater(result["frame_counts"]["fixed_map_compatible"], 9000)
        self.assertEqual(result["frame_counts"].get("waveform_residual", 0), 0)
        self.assertEqual(sum(result["frame_counts"].values()), len(recorded))
        self.assertFalse(result["qualified"])
        self.assertFalse(result["physical_qualified"])
        self.assertFalse(result["waveform_continuity_qualified"])
        self.assertIsNone(result["physical_latency_ms"])

    def test_drop_repeat_and_short_zero_are_not_hidden_by_refitting(self):
        replay, valid, original, anchor = self.fixture()
        for kind in ("drop", "repeat", "zero"):
            recorded = original.copy()
            if kind == "drop":
                recorded[10000:-16] = original[10016:]
            elif kind == "repeat":
                recorded[10000:] = original[9984:-16]
            else:
                recorded[10020:10028] = 0
            with self.subTest(kind=kind):
                result = self.scan(replay, valid, recorded, anchor)
                self.assertGreater(
                    result["frame_counts"].get("waveform_residual", 0), 0
                )
                self.assertEqual(
                    result["maps"]["owner-1"]["channels"], anchor["channels"]
                )

    def test_unknown_neighbor_hole_and_outside_support_are_counted(self):
        replay, valid, recorded, anchor = self.fixture()
        valid[9500:9508] = 0
        result = self.scan(replay, valid, recorded, anchor)
        self.assertGreater(result["frame_counts"].get("unknown_replay", 0), 0)
        self.assertGreater(result["frame_counts"].get("outside_replay", 0), 0)
        self.assertEqual(sum(result["frame_counts"].values()), len(recorded))

    def test_training_frames_are_not_reported_as_independent_coverage(self):
        replay, valid, recorded, anchor = self.fixture()
        result = self.scan(replay, valid, recorded, anchor)
        self.assertEqual(result["frame_counts"]["anchor_training"], 2000)
        for row in result["blocks"]:
            if row["start_frame"] >= 4000 and row["stop_frame"] <= 5000:
                self.assertEqual(row["category"], "anchor_training")

    def test_later_anchor_coordinate_jump_refuses_entire_owner_map(self):
        replay, valid, recorded, anchor = self.fixture()
        other = dict(
            anchor,
            recording_start_frame=10000,
            channels=[
                dict(
                    c,
                    recording_center_frame=11999.5,
                    replay_frame_at_recording_center=c[
                        "replay_frame_at_recording_center"
                    ]
                    + 6001,
                )
                for c in anchor["channels"]
            ],
        )
        result = self.module().scan_coverage(
            {"owner-1": (replay, valid)}, recorded, [anchor, other], self.rate
        )
        self.assertEqual(result["frame_counts"], {"anchor_conflict": len(recorded)})

    def test_duplicate_compatible_owners_are_ambiguous(self):
        replay, valid, recorded, anchor = self.fixture()
        result = self.module().scan_coverage(
            {"owner-1": (replay, valid), "owner-2": (replay, valid)},
            recorded,
            [anchor, dict(anchor, owner="owner-2")],
            self.rate,
        )
        self.assertGreater(result["frame_counts"].get("ambiguous_owner", 0), 10000)

    def test_silence_is_not_independent_proof_of_continuity(self):
        replay, valid, recorded, anchor = self.fixture()
        replay[:] = 0
        recorded[:] = 0
        result = self.scan(replay, valid, recorded, anchor)
        self.assertGreater(result["frame_counts"].get("uninformative_signal", 0), 10000)
        self.assertEqual(result["frame_counts"].get("fixed_map_compatible", 0), 0)

    def test_nonfinite_recording_is_local_refusal_not_erased_results(self):
        replay, valid, recorded, anchor = self.fixture()
        recorded[10000, 0] = np.nan
        result = self.scan(replay, valid, recorded, anchor)
        self.assertGreater(result["frame_counts"].get("nonfinite_recording", 0), 0)
        self.assertGreater(result["frame_counts"].get("fixed_map_compatible", 0), 5000)

    def test_coverage_is_contiguous_and_includes_last_single_frame(self):
        replay, valid, recorded, anchor = self.fixture()
        result = self.scan(replay, valid, recorded[:15937], anchor)
        cursor = 0
        for row in result["blocks"]:
            self.assertEqual(row["start_frame"], cursor)
            self.assertGreater(row["stop_frame"], cursor)
            cursor = row["stop_frame"]
        self.assertEqual(cursor, 15937)
        self.assertEqual(sum(result["frame_counts"].values()), cursor)

    def test_invalid_rate_mask_and_missing_owner_are_not_fallback_passes(self):
        replay, valid, recorded, anchor = self.fixture()
        for mask, anchors, rate in (
            (valid[:-1], [anchor], self.rate),
            (valid, [dict(anchor, owner="absent")], self.rate),
            (valid, [anchor], 0),
        ):
            with self.subTest(rate=rate, owners=[a["owner"] for a in anchors]):
                with self.assertRaises(ValueError):
                    self.module().scan_coverage(
                        {"owner-1": (replay, mask)}, recorded, anchors, rate
                    )

    def test_half_second_error_cannot_use_looser_local_gate_as_overall_gate(self):
        replay, valid, recorded, anchor = self.fixture()
        # 6% error is below the local10% bound but above the unchanged5%
        # half-second held-out/model NRMSE bound.
        recorded[8000:12000] *= 1.06
        result = self.scan(replay, valid, recorded, anchor)
        self.assertGreater(result["frame_counts"].get("waveform_residual", 0), 0)

    def test_gain_jump_on_later_anchor_does_not_retrain_fixed_map(self):
        replay, valid, recorded, anchor = self.fixture()
        other = dict(
            anchor,
            recording_start_frame=10000,
            channels=[
                dict(
                    c,
                    recording_center_frame=11999.5,
                    replay_frame_at_recording_center=c[
                        "replay_frame_at_recording_center"
                    ]
                    + 6000,
                    gain=1.2,
                )
                for c in anchor["channels"]
            ],
        )
        recorded[10000:] *= 1.2
        result = self.module().scan_coverage(
            {"owner-1": (replay, valid)}, recorded, [anchor, other], self.rate
        )
        self.assertGreater(result["frame_counts"].get("waveform_residual", 0), 0)
        self.assertEqual(result["maps"]["owner-1"]["channels"], anchor["channels"])

    def test_generation_crossing_pairs_are_not_interpolated_into_success(self):
        replay, valid, recorded, anchor = self.fixture()
        result = self.module().scan_coverage(
            {"owner-1": (replay, valid)},
            recorded,
            [anchor],
            self.rate,
            generation_boundaries={"owner-1": [9600]},
        )
        self.assertGreater(result["frame_counts"].get("generation_boundary", 0), 0)

    def test_unchanged_original_gates(self):
        module = self.module()
        self.assertEqual(module.GATES, module.ASSOCIATION.GATES)

    def test_aggregate_frontier_inside_local_block_never_leaks_success(self):
        replay, valid, recorded, anchor = self.fixture()
        recorded[12000:] *= 1.06
        result = self.scan(replay, valid, recorded, anchor)
        self.assertFalse(
            any(
                r["category"] == "fixed_map_compatible" and r["stop_frame"] > 12000
                for r in result["blocks"]
            )
        )

    def test_generation_lookup_is_logarithmic_not_full_list_scan(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "crosses_generation"), "missing bounded generation lookup"
        )

        class Counted:
            visits = 0

            def __len__(self):
                return 65536

            def __getitem__(self, index):
                self.visits += 1
                return index

        edges = Counted()
        self.assertFalse(module.crosses_generation(edges, 70000.0, 70064.0))
        self.assertLess(edges.visits, 30)

    def mock_analysis(self, root, *, wrong_hash=False, post_scan_error=False):
        module = self.module()
        replay, valid, recorded, anchor = self.fixture()
        archive = root / "archive"
        (archive / "native-pcm").mkdir(parents=True)
        (archive / "final.json").write_text(
            json.dumps(dict(artifact=dict(drained_chunks=0)))
        )
        (root / "manifest.json").write_text(
            json.dumps(
                dict(
                    archive_directory=str(archive),
                    recorded_wav=dict(path=str(root / "return.wav"), sha256="A" * 64),
                )
            )
        )

        def audited(*args, **kwargs):
            out = root / "output" / "association"
            (out / "replay").mkdir(parents=True)
            (out / "summary.json").write_text("{}")
            (out / "replay" / "summary.json").write_text(
                json.dumps(dict(replay=dict(requested_frames=len(replay))))
            )
            replay.tofile(out / "replay" / "reconstructed.f32le")
            valid.tofile(out / "replay" / "reconstructed-valid.u8")
            return dict(
                archive_status="structurally_complete",
                status="all_selected_windows_associated",
                manifest_sha256="B" * 64
                if wrong_hash
                else module.ASSOCIATION.file_hash(root / "manifest.json"),
                windows=[
                    dict(
                        association_compatible=True,
                        candidates=[
                            dict(
                                anchor, owner=archive.name, association_compatible=True
                            )
                        ],
                    )
                ],
            )

        original_scan = module.scan_coverage

        def scanned(*args, **kwargs):
            result = original_scan(*args, **kwargs)
            if post_scan_error:
                (root / "output" / "association" / "summary.json").unlink()
            return result

        with (
            mock.patch.object(module.ASSOCIATION, "analyze", side_effect=audited),
            mock.patch.object(
                module.ASSOCIATION, "pinned_input", return_value=root / "return.wav"
            ),
            mock.patch("scipy.io.wavfile.read", return_value=(self.rate, recorded)),
            mock.patch.object(module, "scan_coverage", side_effect=scanned),
        ):
            return module.analyze(
                root / "manifest.json", root / "output", make_plot=False
            )

    def test_changed_manifest_between_audit_and_scan_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = self.mock_analysis(Path(temporary), wrong_hash=True)
            self.assertEqual(report["status"], "refused")
            self.assertTrue(any("manifest" in r for r in report["refusals"]), report)

    def test_manifest_is_parsed_and_hashed_from_one_bounded_read(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "manifest_snapshot"),
            "missing single-read manifest snapshot",
        )
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "manifest.json"
            data = b'{"schema_version":1,"note":"original"}'
            path.write_bytes(data)
            with mock.patch.object(Path, "read_bytes", return_value=data) as read:
                parsed, raw, digest = module.manifest_snapshot(path)
            self.assertEqual(read.call_count, 1)
            self.assertEqual(parsed["note"], "original")
            self.assertEqual(raw, data)
            import hashlib

            self.assertEqual(digest, hashlib.sha256(data).hexdigest().upper())

    def test_association_uses_owned_manifest_snapshot_not_mutable_original(self):
        module = self.module()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "manifest.json").write_text(
                json.dumps(dict(archive_directory=str(root / "archive")))
            )

            def audited(path, output, **kwargs):
                self.assertNotEqual(path.resolve(), (root / "manifest.json").resolve())
                self.assertEqual(
                    path.read_bytes(), (root / "manifest.json").read_bytes()
                )
                output.mkdir(parents=True)
                (root / "manifest.json").write_text('{"tampered":true}')
                return dict(
                    archive_status="refused",
                    manifest_sha256=module.ASSOCIATION.file_hash(path),
                )

            with mock.patch.object(module.ASSOCIATION, "analyze", side_effect=audited):
                report = module.analyze(
                    root / "manifest.json", root / "output", make_plot=False
                )
            self.assertEqual(report["status"], "refused")
            self.assertTrue((root / "output" / "input-manifest.json").exists())

    def test_post_scan_hash_error_preserves_numeric_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.mock_analysis(root, post_scan_error=True)
            self.assertEqual(report["status"], "refused")
            self.assertEqual(report["denominator_frames"], 16000)
            self.assertTrue(report["refusals"])
            self.assertTrue((root / "output" / "summary.json").exists())

    def test_plot_collections_are_bounded_by_categories_not_runs(self):
        module = self.module()
        replay, valid, recorded, anchor = self.fixture()
        result = self.scan(replay, valid, recorded, anchor)
        result["blocks"] = [
            dict(
                start_frame=i,
                stop_frame=i + 1,
                category="waveform_residual" if i % 2 else "outside_replay",
                owner=None,
            )
            for i in range(1000)
        ]
        result["frame_counts"] = dict(waveform_residual=500, outside_replay=500)
        axes = [mock.MagicMock() for _ in range(3)]
        with (
            tempfile.TemporaryDirectory() as temporary,
            mock.patch(
                "matplotlib.pyplot.subplots", return_value=(mock.MagicMock(), axes)
            ),
            mock.patch("matplotlib.pyplot.close"),
        ):
            module.render(Path(temporary), result)
        self.assertLessEqual(axes[0].broken_barh.call_count, 2)

    def test_unknown_aggregate_context_cannot_bypass_five_percent_gate(self):
        replay, valid, recorded, anchor = self.fixture()
        valid[8000:8008] = 0
        recorded[8000:12000] *= 1.06
        result = self.scan(replay, valid, recorded, anchor)
        accepted = [
            r for r in result["blocks"] if r["category"] == "fixed_map_compatible"
        ]
        self.assertFalse(
            any(r["start_frame"] < 12000 and r["stop_frame"] > 9000 for r in accepted)
        )

    def test_output_parent_cannot_overlap_any_immutable_source(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "validate_output"),
            "missing outer output-overlap validation",
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "run" / "input.json"
            source.parent.mkdir()
            source.write_text("{}")
            manifest = dict(
                archive_directory=str(root / "run" / "archive"),
                recorded_wav=dict(path=str(source)),
            )
            with self.assertRaises(ValueError):
                module.validate_output(root / "manifest.json", root / "run", manifest)
            module.validate_output(
                root / "manifest.json", root / "new-output", manifest
            )

    def test_detail_cap_counts_remaining_frames_as_refused_not_omitted(self):
        replay, valid, recorded, anchor = self.fixture()
        recorded[::128] = 0
        module = self.module()
        with mock.patch.object(module, "MAX_DETAIL_RUNS", 3, create=True):
            result = self.scan(replay, valid, recorded, anchor)
        self.assertGreater(result["frame_counts"].get("analysis_cap_refused", 0), 0)
        self.assertEqual(sum(result["frame_counts"].values()), len(recorded))
        self.assertEqual(result["blocks"][-1]["stop_frame"], len(recorded))
        self.assertLessEqual(len(result["blocks"]), 4)

    def test_generation_metadata_reads_actual_span_edges_and_unknown_separators(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "read_generation_boundaries"),
            "missing actual generation metadata reader",
        )
        with tempfile.TemporaryDirectory() as temporary:
            owner = Path(temporary)
            (owner / "final.json").write_text(
                json.dumps(dict(artifact=dict(drained_chunks=1)))
            )
            (owner / "chunk-00000001.json").write_text(
                json.dumps(
                    dict(
                        events=[
                            dict(
                                span=dict(output_start_frame=0, frames=32, generation=1)
                            ),
                            dict(
                                span=dict(
                                    output_start_frame=32, frames=32, generation=2
                                )
                            ),
                            dict(
                                span=dict(
                                    output_start_frame=128, frames=32, generation=3
                                )
                            ),
                        ]
                    )
                )
            )
            self.assertEqual(module.read_generation_boundaries(owner), [32, 64, 128])

    def test_closed_archive_to_entire_recording_and_png(self):
        module = self.module()
        spec = importlib.util.spec_from_file_location(
            "coverage_native_fixture",
            SCRIPT.parent / "tests/process-native-pcm.tests.py",
        )
        native_fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(native_fixture)
        spec = importlib.util.spec_from_file_location(
            "coverage_binding_fixture",
            SCRIPT.parent / "tests/process-replay-obs.tests.py",
        )
        binding_fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(binding_fixture)
        from scipy.io import wavfile

        rate, frames = 48000, 96000
        native = (
            np.random.default_rng(2026).normal(0, 0.02, (frames + 2, 2)).astype("<f4")
        )
        events = [
            native_fixture.event(
                i // 32 + 1,
                frames=32,
                output_start_frame=i,
                input_start_frame=i,
                phase_bits=native_fixture.bits(0),
            )
            for i in range(0, frames, 32)
        ]
        recorded = np.column_stack(
            [
                np.interp(
                    np.arange(4 * rate) - rate - 0.4,
                    np.arange(frames),
                    native[:frames, c],
                    left=0,
                    right=0,
                )
                for c in range(2)
            ]
        ).astype("<f4")
        with tempfile.TemporaryDirectory(prefix="moiren-coverage-") as temporary:
            root = Path(temporary)
            owner = native_fixture.fixture(
                root / "owner",
                packets=[(native, len(native), 1, 0)],
                events=events,
                requested=frames,
                reads=1,
            )
            wavfile.write(root / "return.wav", rate, recorded)
            prior = dict(
                capture_complete=True,
                capture_error=None,
                control=dict(established=True),
                windows=[dict(recording_time_s=2.25, model_compatible=False)],
            )
            (root / "prior.json").write_text(json.dumps(prior))
            manifest = dict(
                schema_version=1,
                archive_directory=str(owner),
                recorded_wav=dict(
                    path=str(root / "return.wav"),
                    sha256=module.ASSOCIATION.file_hash(root / "return.wav"),
                ),
                prior_summary=dict(
                    path=str(root / "prior.json"),
                    sha256=module.ASSOCIATION.file_hash(root / "prior.json"),
                ),
            )
            binding_fixture.ReplayAssociationTests().bind_recording(
                root, prior, manifest
            )
            (root / "manifest.json").write_text(json.dumps(manifest))
            report = module.analyze(root / "manifest.json", root / "output")
            self.assertEqual(report["status"], "coverage_accounted", report)
            self.assertEqual(report["denominator_frames"], len(recorded))
            self.assertEqual(sum(report["frame_counts"].values()), len(recorded))
            self.assertGreater(report["frame_counts"]["fixed_map_compatible"], rate)
            self.assertGreater(report["frame_counts"]["outside_replay"], rate)
            self.assertEqual(
                (root / "output/overview.png").read_bytes()[:8], b"\x89PNG\r\n\x1a\n"
            )
            self.assertFalse(report["qualified"])


if __name__ == "__main__":
    unittest.main()
