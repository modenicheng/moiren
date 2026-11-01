#!/usr/bin/env python3
"""Actual replay stays fixed; only the independent recorder mapping is fitted."""

import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import numpy as np


sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().parents[1] / "process-replay-obs.py"
SPEC = (
    importlib.util.spec_from_file_location("process_replay_obs", SCRIPT)
    if SCRIPT.exists()
    else None
)
MODULE = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(MODULE)


class ReplayAssociationTests(unittest.TestCase):
    rate = 8000

    def module(self):
        self.assertIsNotNone(
            MODULE, "missing actual SRC replay/independent recorder association"
        )
        return MODULE

    def bind_recording(self, root, prior, manifest):
        module = self.module()
        decoder = shutil.which("ffmpeg")
        self.assertIsNotNone(decoder, "full archive integration needs ffmpeg")
        recording = root / "capture.mkv"
        from scipy.io import wavfile

        rate, returned = wavfile.read(root / "return.wav")
        reference = returned.copy()
        control = (
            np.random.default_rng(731).normal(0, 0.01, (rate // 4, 2)).astype("<f4")
        )
        control_at = rate // 20
        reference[control_at : control_at + len(control)] = control
        wavfile.write(root / "reference.wav", rate, reference)
        wavfile.write(root / "control.wav", rate, control)
        subprocess.run(
            [
                decoder,
                "-nostdin",
                "-v",
                "error",
                "-i",
                str(root / "reference.wav"),
                "-i",
                str(root / "return.wav"),
                "-map",
                "0:a:0",
                "-map",
                "1:a:0",
                "-c:a",
                "pcm_f32le",
                str(recording),
            ],
            check=True,
            capture_output=True,
            timeout=30,
        )
        prior.update(recording=str(recording), recording_bytes=recording.stat().st_size)
        (root / "prior.json").write_text(json.dumps(prior))
        manifest["prior_summary"]["sha256"] = module.file_hash(root / "prior.json")
        manifest["recording"] = dict(
            path=str(recording),
            sha256=module.file_hash(recording),
            return_audio_index=1,
        )
        manifest["decoder"] = dict(path=decoder, sha256=module.file_hash(decoder))
        probe = shutil.which("ffprobe")
        self.assertIsNotNone(probe)
        manifest["probe"] = dict(path=probe, sha256=module.file_hash(probe))
        manifest["reference_wav"] = dict(
            path=str(root / "reference.wav"),
            sha256=module.file_hash(root / "reference.wav"),
        )
        manifest["control_wav"] = dict(
            path=str(root / "control.wav"),
            sha256=module.file_hash(root / "control.wav"),
        )

    def fixture(self, *, offset=12037.4, ppm=0, gain=1):
        replay = (
            np.random.default_rng(20261102)
            .normal(0, 0.02, (4 * self.rate, 2))
            .astype("<f4")
        )
        n = np.arange(7 * self.rate)
        positions = (n - offset) * (1 + ppm * 1e-6)
        recorded = gain * np.column_stack(
            [
                np.interp(
                    positions, np.arange(len(replay)), replay[:, c], left=0, right=0
                )
                for c in range(2)
            ]
        )
        valid = np.ones(len(replay), dtype="u1")
        return replay, valid, recorded

    def fit(self, replay, valid, recorded, **kwargs):
        return self.module().fit_recorded_window(
            replay, valid, recorded, 3 * self.rate, self.rate, **kwargs
        )

    def test_global_origin_and_fractional_recorder_clock_are_not_production_servo(self):
        replay, valid, recorded = self.fixture(ppm=333, gain=-0.75)
        row = self.fit(replay, valid, recorded)
        self.assertTrue(row["association_compatible"], row)
        center = 3 * self.rate + (self.rate // 2 - 1) / 2
        expected = (center - 12037.4) * (1 + 333e-6)
        self.assertAlmostEqual(
            row["replay_frame_at_recording_center"], expected, delta=0.03
        )
        for channel in row["channels"]:
            self.assertAlmostEqual(channel["correction_ppm"], 333, delta=3)
            self.assertAlmostEqual(channel["gain"], -0.75, delta=0.005)
        self.assertFalse(row["physical_qualified"])
        self.assertFalse(row["waveform_continuity_qualified"])

    def test_recorded_mapping_does_not_reconstruct_nominal_source_instead(self):
        replay, valid, _ = self.fixture()
        # Independent non-affine 32-frame source phase; the actual replay is
        # already fixed and must not be replaced by a fitted nominal source.
        positions = np.cumsum(
            1 + np.where((np.arange(len(replay)) // 256) % 2, 0.0018, -0.0018)
        )
        actual = np.column_stack(
            [
                np.interp(positions, np.arange(len(replay)), replay[:, c])
                for c in range(2)
            ]
        )
        recorded = np.vstack([np.zeros((12000, 2)), actual, np.zeros((12000, 2))])
        self.assertTrue(self.fit(actual, valid, recorded)["association_compatible"])
        wrong = self.fit(replay, valid, recorded)
        self.assertFalse(wrong["association_compatible"])

    def test_holdout_corruption_refuses_without_retuning_training(self):
        replay, valid, recorded = self.fixture(ppm=500)
        baseline = self.fit(replay, valid, recorded)
        self.assertTrue(baseline["association_compatible"], baseline)
        broken = recorded.copy()
        broken[3 * self.rate + 1500 : 3 * self.rate + 1700] = 0
        row = self.fit(replay, valid, broken)
        self.assertFalse(row["association_compatible"])
        for before, after in zip(baseline["channels"], row["channels"]):
            for key in ("offset_frames", "correction_ppm", "gain"):
                self.assertAlmostEqual(before[key], after[key], delta=1e-8)

    def test_zero_drop_repeat_and_direct_mix_remain_refused(self):
        replay, valid, recorded = self.fixture(offset=12000)
        for kind in ("zero", "drop", "repeat", "mix"):
            with self.subTest(kind=kind):
                broken = recorded.copy()
                at = 3 * self.rate + 1800
                if kind == "zero":
                    broken[at : at + 256] = 0
                elif kind == "drop":
                    broken[at:-32] = recorded[at + 32 :]
                elif kind == "repeat":
                    broken[at:] = recorded[at - 32 : -32]
                else:
                    broken[80:] += recorded[:-80]
                self.assertFalse(
                    self.fit(replay, valid, broken)["association_compatible"]
                )

    def test_unknown_replay_support_never_becomes_certified_silence(self):
        replay, valid, recorded = self.fixture(offset=12000)
        valid[13500:13700] = 0
        replay[13500:13700] = 0
        row = self.fit(replay, valid, recorded)
        self.assertFalse(row["association_compatible"])
        self.assertIn("unknown", row["reason"])

    def test_unknown_elsewhere_does_not_destroy_complete_local_support(self):
        replay, valid, recorded = self.fixture(offset=12000)
        valid[:2000] = 0
        self.assertTrue(self.fit(replay, valid, recorded)["association_compatible"])

    def test_duplicate_waveform_is_not_a_unique_global_origin(self):
        replay, valid, recorded = self.fixture(offset=12000)
        repeated = np.vstack([replay, replay])
        row = self.fit(repeated, np.ones(len(repeated), dtype="u1"), recorded)
        self.assertFalse(row["association_compatible"])
        self.assertIn("ambiguous", row["reason"])

    def test_stereo_mapping_disagreement_is_not_an_accepted_mean(self):
        replay, valid, recorded = self.fixture(offset=12000)
        recorded[40:, 1] = recorded[:-40, 1]
        self.assertFalse(self.fit(replay, valid, recorded)["association_compatible"])

    def test_unrelated_silent_and_periodic_recordings_do_not_pass(self):
        replay, valid, recorded = self.fixture()
        n = np.arange(len(recorded))
        for other in (
            np.zeros_like(recorded),
            np.random.default_rng(8).normal(0, 0.02, recorded.shape),
            np.repeat(np.sin(n[:, None] * 0.2), 2, axis=1),
        ):
            self.assertFalse(self.fit(replay, valid, other)["association_compatible"])

    def test_open_out_of_range_and_malformed_numeric_inputs_refuse(self):
        replay, valid, recorded = self.fixture()
        self.assertFalse(
            self.module().fit_recorded_window(
                replay, valid, recorded, len(recorded) - 100, self.rate
            )["association_compatible"]
        )
        for bad in (valid[:-1], np.full(len(valid), 2, dtype="u1")):
            with self.assertRaises(ValueError):
                self.fit(replay, bad, recorded)
        recorded[3 * self.rate + 500, 0] = np.nan
        with self.assertRaises(ValueError):
            self.fit(replay, valid, recorded)

    def test_global_search_is_chunk_bounded_and_keeps_boundary_peak(self):
        module = self.module()
        template = np.random.default_rng(81).normal(size=512)
        values = np.random.default_rng(82).normal(size=100000)
        values[15990 : 15990 + len(template)] = template
        original = module.LINEAR.ISOLATED.BASE.ncc
        from unittest import mock

        def bounded(x, y):
            self.assertLessEqual(len(y), 2 * self.rate + len(x))
            return original(x, y)

        with mock.patch.object(module.LINEAR.ISOLATED.BASE, "ncc", side_effect=bounded):
            point = module.coarse_match(
                template, values, np.ones(len(values), dtype="u1"), self.rate
            )
        self.assertEqual(point["frame"], 15990)
        self.assertGreater(point["peak_ratio"], 1.8)

    def test_existing_model_gates_are_unchanged(self):
        module = self.module()
        self.assertEqual(module.GATES, module.LINEAR.GATES)

    def test_json_exponent_overflow_refuses_before_retaining_original_control(self):
        module = self.module()
        with tempfile.TemporaryDirectory(
            prefix="moiren-replay-nonfinite-"
        ) as temporary:
            path = Path(temporary) / "prior.json"
            path.write_text('{"control":{"established":true,"rms":1e400}}')
            with self.assertRaisesRegex(ValueError, "nonfinite"):
                module.load_json(path)

    def test_pinned_reference_wav_cannot_borrow_unrelated_return_control(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "verify_recording_binding"),
            "missing recording/return-track binding",
        )
        with self.assertRaisesRegex(ValueError, "recording"):
            module.verify_recording_binding({}, Path("unrelated.wav"), {}, None)

    def test_reference_track_swap_and_wrong_prior_recording_are_refused(self):
        module = self.module()
        from scipy.io import wavfile

        decoder = shutil.which("ffmpeg")
        self.assertIsNotNone(decoder)
        with tempfile.TemporaryDirectory(
            prefix="moiren-return-track-binding-"
        ) as temporary:
            root = Path(temporary)
            reference = (
                np.random.default_rng(991).normal(0, 0.01, (16000, 2)).astype("<f4")
            )
            returned = reference * 0.5
            wavfile.write(root / "reference.wav", 8000, reference)
            wavfile.write(root / "return.wav", 8000, returned)
            recording = root / "two-track.mkv"
            subprocess.run(
                [
                    decoder,
                    "-nostdin",
                    "-v",
                    "error",
                    "-i",
                    str(root / "reference.wav"),
                    "-i",
                    str(root / "return.wav"),
                    "-map",
                    "0:a:0",
                    "-map",
                    "1:a:0",
                    "-c:a",
                    "pcm_f32le",
                    str(recording),
                ],
                check=True,
                capture_output=True,
                timeout=30,
            )
            manifest = dict(
                recording=dict(
                    path=str(recording),
                    sha256=module.file_hash(recording),
                    return_audio_index=1,
                ),
                decoder=dict(path=decoder, sha256=module.file_hash(decoder)),
                probe=dict(
                    path=shutil.which("ffprobe"),
                    sha256=module.file_hash(shutil.which("ffprobe")),
                ),
            )
            prior = dict(
                recording=str(recording), recording_bytes=recording.stat().st_size
            )
            good = module.verify_recording_binding(
                manifest, root / "return.wav", prior, returned
            )
            self.assertIn("decoded_return_pcm_sha256", good)
            with self.assertRaisesRegex(ValueError, "return audio1"):
                module.verify_recording_binding(
                    manifest, root / "reference.wav", prior, reference
                )
            prior["recording"] = str(root / "unrelated.mkv")
            with self.assertRaisesRegex(ValueError, "another recording"):
                module.verify_recording_binding(
                    manifest, root / "return.wav", prior, returned
                )
            prior["recording"] = str(recording)
            wavfile.write(root / "wrong-rate.wav", 16000, returned)
            with self.assertRaisesRegex(ValueError, "sample rate"):
                module.verify_recording_binding(
                    manifest, root / "wrong-rate.wav", prior, returned
                )

    def test_all_prespecified_windows_and_cross_owner_ambiguity_remain_visible(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "associate_windows"),
            "missing multi-owner fixed-window association",
        )
        replay, valid, recorded = self.fixture(offset=12000)
        owners = {"owner-1": (replay, valid), "owner-2": (np.zeros_like(replay), valid)}
        prior = [
            dict(recording_time_s=3.25, model_compatible=False),
            dict(recording_time_s=6.25, model_compatible=False),
        ]
        rows = module.associate_windows(owners, recorded, prior, self.rate)
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["owner"], "owner-1")
        self.assertTrue(rows[0]["association_compatible"])
        self.assertFalse(rows[1]["association_compatible"])
        self.assertFalse(rows[0]["prior_affine_compatible"])
        owners["owner-2"] = (replay.copy(), valid)
        ambiguous = module.associate_windows(owners, recorded, prior[:1], self.rate)[0]
        self.assertFalse(ambiguous["association_compatible"])
        self.assertEqual(ambiguous["reason"], "ambiguous_owner_association")

    def test_one_nonfinite_window_does_not_erase_other_fixed_window_results(self):
        module = self.module()
        replay, valid, recorded = self.fixture(offset=12000)
        recorded[2 * self.rate + 100, 0] = np.nan
        rows = module.associate_windows(
            {"owner": (replay, valid)},
            recorded,
            [dict(recording_time_s=2.25), dict(recording_time_s=3.25)],
            self.rate,
        )
        self.assertEqual(len(rows), 2)
        self.assertFalse(rows[0]["association_compatible"])
        self.assertIn("nonfinite", rows[0]["candidates"][0]["reason"])
        self.assertTrue(rows[1]["association_compatible"])

    def test_archive_driver_reaudits_sources_and_hash_pins_recorded_pcm(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "analyze"), "missing independently reaudited archive driver"
        )
        # Reuse the Rust wire-format fixture, not a second archive protocol.
        spec = importlib.util.spec_from_file_location(
            "native_fixture", SCRIPT.parent / "tests/process-native-pcm.tests.py"
        )
        fixture_module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixture_module)
        from scipy.io import wavfile

        with tempfile.TemporaryDirectory(prefix="moiren-replay-obs-") as temporary:
            root = Path(temporary)
            owner = fixture_module.fixture(root / "owner")
            wavfile.write(root / "return.wav", 48000, np.zeros((48000, 2), dtype="<f4"))
            prior = dict(
                capture_complete=True,
                capture_error=None,
                control={"established": True},
                windows=[dict(recording_time_s=0.25, model_compatible=False)],
            )
            (root / "prior.json").write_text(json.dumps(prior))
            manifest = dict(
                schema_version=1,
                archive_directory=str(owner),
                recorded_wav=dict(
                    path=str(root / "return.wav"),
                    sha256=module.file_hash(root / "return.wav"),
                ),
                prior_summary=dict(
                    path=str(root / "prior.json"),
                    sha256=module.file_hash(root / "prior.json"),
                ),
            )
            self.bind_recording(root, prior, manifest)
            (root / "manifest.json").write_text(json.dumps(manifest))
            result = module.analyze(
                root / "manifest.json", root / "output", make_plot=False
            )
            self.assertEqual(result["archive_status"], "structurally_complete")
            self.assertEqual(len(result["windows"]), 1)
            self.assertFalse(result["physical_qualified"])
            self.assertEqual(result["status"], "refused")
            self.assertTrue(result["fresh_control"]["established"])
            wavfile.write(
                root / "control.wav", 48000, np.zeros((12000, 2), dtype="<f4")
            )
            manifest["control_wav"]["sha256"] = module.file_hash(root / "control.wav")
            (root / "manifest.json").write_text(json.dumps(manifest))
            wrong_control = module.analyze(
                root / "manifest.json", root / "wrong-control", make_plot=False
            )
            self.assertIn("fresh isolated control", " ".join(wrong_control["refusals"]))
            self.assertFalse((root / "wrong-control/replay").exists())
            manifest["recorded_wav"]["sha256"] = "0" * 64
            (root / "manifest.json").write_text(json.dumps(manifest))
            refused = module.analyze(
                root / "manifest.json", root / "wrong-hash", make_plot=False
            )
            self.assertEqual(refused["status"], "refused")
            self.assertIn("hash", " ".join(refused["refusals"]))
            self.assertFalse((root / "wrong-hash/replay").exists())
            with self.assertRaises(FileExistsError):
                module.analyze(root / "manifest.json", root / "output", make_plot=False)

    def test_visualization_failure_retains_refused_numeric_receipt(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "finish_report"),
            "missing failure-safe plot/JSON finalization",
        )
        from unittest import mock

        with tempfile.TemporaryDirectory(
            prefix="moiren-replay-plot-failure-"
        ) as temporary:
            output = Path(temporary)
            report = dict(status="all_selected_windows_associated", refusals=[])
            with mock.patch.object(
                module, "render", side_effect=OSError("injected PNG disk failure")
            ):
                module.finish_report(output, report, {}, None, None, make_plot=True)
            saved = json.loads((output / "summary.json").read_text())
            self.assertEqual(saved["status"], "refused")
            self.assertIn("PNG", saved["refusals"][0])

    def test_full_closed_archive_actual_replay_associates_and_renders(self):
        module = self.module()
        spec = importlib.util.spec_from_file_location(
            "native_full_fixture", SCRIPT.parent / "tests/process-native-pcm.tests.py"
        )
        fixture_module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixture_module)
        from scipy.io import wavfile

        rate, frames = 48000, 96000
        native = (
            np.random.default_rng(2026).normal(0, 0.02, (frames + 2, 2)).astype("<f4")
        )
        events = [
            fixture_module.event(
                index // 32 + 1,
                frames=32,
                output_start_frame=index,
                input_start_frame=index,
                phase_bits=fixture_module.bits(0.0),
            )
            for index in range(0, frames, 32)
        ]
        n = np.arange(4 * rate)
        recorded = np.column_stack(
            [
                np.interp(
                    n - rate - 0.4,
                    np.arange(frames),
                    native[:frames, c],
                    left=0,
                    right=0,
                )
                for c in range(2)
            ]
        ).astype("<f4")
        with tempfile.TemporaryDirectory(prefix="moiren-full-replay-obs-") as temporary:
            root = Path(temporary)
            owner = fixture_module.fixture(
                root / "owner",
                packets=[(native, len(native), 1, 0)],
                events=events,
                requested=frames,
                reads=1,
            )
            wavfile.write(root / "return.wav", rate, recorded)
            (root / "prior.json").write_text(
                json.dumps(
                    dict(
                        capture_complete=True,
                        capture_error=None,
                        control=dict(established=True),
                        windows=[dict(recording_time_s=2.25, model_compatible=False)],
                    )
                )
            )
            manifest = dict(
                schema_version=1,
                archive_directory=str(owner),
                recorded_wav=dict(
                    path=str(root / "return.wav"),
                    sha256=module.file_hash(root / "return.wav"),
                ),
                prior_summary=dict(
                    path=str(root / "prior.json"),
                    sha256=module.file_hash(root / "prior.json"),
                ),
            )
            prior = json.loads((root / "prior.json").read_text())
            self.bind_recording(root, prior, manifest)
            (root / "manifest.json").write_text(json.dumps(manifest))
            result = module.analyze(
                root / "manifest.json", root / "output", make_plot=True
            )
            self.assertEqual(
                result["status"], "all_selected_windows_associated", result
            )
            self.assertFalse(result["qualified"])
            self.assertFalse(result["waveform_continuity_qualified"])
            self.assertIsNone(result["physical_latency_ms"])
            self.assertEqual(
                (root / "output/overview.png").read_bytes()[:8], b"\x89PNG\r\n\x1a\n"
            )
            self.assertLess((root / "output/summary.json").stat().st_size, 65536)

    def test_archive_preflight_caps_combined_derived_storage_before_replay(self):
        module = self.module()
        self.assertTrue(
            hasattr(module, "preflight_archive"),
            "missing aggregate derived-storage preflight",
        )
        spec = importlib.util.spec_from_file_location(
            "native_cap_fixture", SCRIPT.parent / "tests/process-native-pcm.tests.py"
        )
        fixture_module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixture_module)
        with tempfile.TemporaryDirectory(prefix="moiren-replay-cap-") as temporary:
            root = Path(temporary)
            archive = root / "archive"
            archive.mkdir()
            (archive / "summary.json").write_text(
                json.dumps(
                    dict(
                        schema_version=1,
                        sessions=2,
                        expected_owners=2,
                        native_pcm_enabled=True,
                        error=None,
                        qualified=False,
                    )
                )
            )
            for index in (1, 2):
                owner = fixture_module.fixture(
                    archive / ("owner-%04d-generation-1" % index)
                )
                data = json.loads((owner / "final.json").read_text())
                data["artifact"]["requested_frames"] = 18_000_000
                (owner / "final.json").write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "combined derived"):
                module.preflight_archive(archive)


if __name__ == "__main__":
    unittest.main()
