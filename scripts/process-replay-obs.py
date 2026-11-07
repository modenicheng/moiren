#!/usr/bin/env python3
"""Associate fixed actual-SRC replay with independent retained OBS samples.

Only recorder coordinates, local recorder rate, gain and DC are fitted. Native
SRC phases, generations and samples are never re-estimated. Separated held-out
windows are diagnostic associations, not physical latency or continuity proof.
"""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

import numpy as np


sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "replay_obs_linear", Path(__file__).with_name("obs-linear-clock.py")
)
LINEAR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LINEAR)
GATES = LINEAR.GATES.copy()
BLOCK_FRAMES = 65536
MAX_FRAMES = 32 * 1024 * 1024
MAX_FILE_BYTES = 512 * 1024 * 1024


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def stereo_shape(values):
    require(
        isinstance(values, np.ndarray)
        and values.ndim == 2
        and values.shape[1] == 2
        and 0 < len(values) <= MAX_FRAMES
        and np.issubdtype(values.dtype, np.floating),
        "bounded nonempty floating stereo PCM required",
    )


def validate_mask(values, frames):
    require(
        values.ndim == 1 and len(values) == frames, "replay validity length mismatch"
    )
    for start in range(0, frames, BLOCK_FRAMES):
        require(
            bool(np.isin(values[start : start + BLOCK_FRAMES], [0, 1]).all()),
            "replay validity must be binary",
        )


def coarse_match(template, values, valid, rate):
    """Two bounded passes find a unique global peak, including block edges.

    Candidate starts partition the timeline; template overlap is read-only.
    The second pass excludes only the winning one-ms neighborhood globally,
    so a repeated distant phrase cannot be mistaken for a unique owner origin.
    """
    require(
        3 <= len(template) <= rate and len(values) <= MAX_FRAMES,
        "coarse search input exceeds bounded work contract",
    )
    require(valid.ndim == 1 and len(valid) == len(values), "coarse validity length")
    if len(values) < len(template) or np.std(template) <= 1e-12:
        return None
    count, block = len(values) - len(template) + 1, 2 * rate

    def curves():
        for left in range(0, count, block):
            stop = min(count, left + block)
            right = stop + len(template) - 1
            curve = LINEAR.ISOLATED.BASE.ncc(template, values[left:right])
            unknown = np.r_[0, np.cumsum(valid[left:right] != 1)]
            curve[unknown[len(template) :] != unknown[: -len(template)]] = np.nan
            yield left, curve

    best = None
    for left, curve in curves():
        if np.isfinite(curve).any():
            local = int(np.nanargmax(np.abs(curve)))
            point = {"frame": left + local, "correlation": float(curve[local])}
            if best is None or abs(point["correlation"]) > abs(best["correlation"]):
                best = point
    if best is None:
        return None
    competitor = 0.0
    radius = max(1, int(round(rate / 1000)))
    for left, curve in curves():
        curve[np.abs(left + np.arange(len(curve)) - best["frame"]) <= radius] = np.nan
        if np.isfinite(curve).any():
            competitor = max(competitor, float(np.nanmax(np.abs(curve))))
    best.update(
        competitor_abs=competitor,
        peak_ratio=abs(best["correlation"]) / max(competitor, 1e-12),
    )
    return best


def fit_channel(replay, valid, recorded, start, rate):
    width = rate // 2
    target = np.asarray(recorded[start : start + width], dtype=np.float64)
    require(bool(np.isfinite(target).all()), "nonfinite recorded window")
    patch_width = min(512, width // 8)
    patch_at = width // 8 - patch_width // 2
    coarse = coarse_match(
        target[patch_at : patch_at + patch_width], replay, valid, rate
    )
    if coarse is None or coarse["peak_ratio"] < GATES["coarse_peak_ratio_min"]:
        return {
            "compatible": False,
            "reason": "ambiguous_global_replay_origin",
            "global_coarse": coarse,
        }
    # Translate the global owner coordinate to an equal-coordinate local crop
    # for the existing held-out fitter. Its optimizer and gates stay unchanged.
    pad = rate // 2
    origin = max(0, coarse["frame"] - patch_at - pad)
    reference = replay[origin : min(len(replay), origin + 2 * rate)]
    local_recorded = np.zeros(pad + width, dtype=np.float64)
    local_recorded[pad:] = target
    result = LINEAR.fit_channel(reference, local_recorded, pad, rate)
    result["global_coarse"] = coarse
    if "offset_frames" not in result:
        return result
    center = pad + (width - 1) / 2
    local = np.arange(width, dtype=np.float64) + pad
    locations = (
        origin
        + local
        - result["offset_frames"]
        + (local - center) * result["correction_ppm"] * 1e-6
    )
    if locations[0] < 0 or locations[-1] >= len(replay) - 1:
        result.update(compatible=False, reason="incomplete_replay_support")
        return result
    indices = np.floor(locations).astype(np.intp)
    support = (valid[indices] == 1) & (valid[indices + 1] == 1)
    result["unknown_neighbor_pairs"] = int((~support).sum())
    result["replay_frame_at_recording_center"] = float(
        origin + center - result["offset_frames"]
    )
    result["recording_center_frame"] = start + (width - 1) / 2
    result["offset_frames"] = (
        result["recording_center_frame"] - result["replay_frame_at_recording_center"]
    )
    if not support.all():
        result.update(compatible=False, reason="unknown_actual_replay_support")
    return result


def fit_recorded_window(replay, valid, recorded, start, rate):
    stereo_shape(replay)
    stereo_shape(recorded)
    validate_mask(valid, len(replay))
    require(type(rate) is int and 8000 <= rate <= 192000, "supported rate8..192kHz")
    require(type(start) is int, "recording start must be an integer frame")
    row = dict(
        recording_start_frame=start,
        recording_time_s=(start + rate / 4) / rate,
        association_compatible=False,
        physical_qualified=False,
        waveform_continuity_qualified=False,
        replay_frame_at_recording_center=None,
        channels=[],
        reason="incomplete_recorded_window",
    )
    if start < 0 or start + rate // 2 > len(recorded):
        return row
    for channel in range(2):
        row["channels"].append(
            fit_channel(replay[:, channel], valid, recorded[:, channel], start, rate)
        )
    unknown = any(p.get("unknown_neighbor_pairs", 0) for p in row["channels"])
    row["reason"] = (
        "unknown_actual_replay_support"
        if unknown
        else "unresolved_actual_replay_association"
    )
    if all(p["compatible"] for p in row["channels"]):
        a, b = row["channels"]
        location_agrees = (
            abs(
                a["replay_frame_at_recording_center"]
                - b["replay_frame_at_recording_center"]
            )
            * 1000
            / rate
            <= GATES["stereo_offset_disagreement_ms_max"]
        )
        rate_agrees = (
            abs(a["correction_ppm"] - b["correction_ppm"])
            <= GATES["stereo_rate_disagreement_ppm_max"]
        )
        if location_agrees and rate_agrees:
            row.update(
                association_compatible=True,
                reason="heldout_actual_replay_association",
                replay_frame_at_recording_center=(
                    a["replay_frame_at_recording_center"]
                    + b["replay_frame_at_recording_center"]
                )
                / 2,
            )
    if any(p["reason"] == "ambiguous_global_replay_origin" for p in row["channels"]):
        row["reason"] = "ambiguous_global_replay_origin"
    return row


def associate_windows(owners, recorded, prior_windows, rate):
    require(1 <= len(owners) <= 8, "one to eight audited owners required")
    require(
        type(prior_windows) is list and 1 <= len(prior_windows) <= 128,
        "one to128 prespecified windows required",
    )
    rows = []
    for previous in prior_windows:
        time = previous["recording_time_s"]
        require(
            type(time) in (float, int) and np.isfinite(time) and time >= 0.25,
            "invalid prespecified recording coordinate",
        )
        start = int(round(time * rate - rate / 4))
        candidates = []
        for name, (replay, valid) in owners.items():
            try:
                result = fit_recorded_window(replay, valid, recorded, start, rate)
            except (ValueError, TypeError, OverflowError) as error:
                result = dict(
                    association_compatible=False,
                    channels=[],
                    reason=(str(error) or type(error).__name__)[:1024],
                )
            result["owner"] = name
            candidates.append(result)
        compatible = [r for r in candidates if r["association_compatible"]]
        accepted = len(compatible) == 1
        row = dict(
            recording_time_s=time,
            recording_start_frame=start,
            prior_affine_compatible=previous.get("model_compatible"),
            association_compatible=accepted,
            owner=compatible[0]["owner"] if accepted else None,
            replay_frame_at_recording_center=compatible[0][
                "replay_frame_at_recording_center"
            ]
            if accepted
            else None,
            reason="unique_heldout_actual_replay_association"
            if accepted
            else "ambiguous_owner_association"
            if len(compatible) > 1
            else "all_owners_refused",
            candidates=candidates,
        )
        rows.append(row)
    return rows


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest().upper()


def load_json(path):
    path = Path(path)
    require(
        path.is_file()
        and not path.is_symlink()
        and path.stat().st_size <= 4 * 1024 * 1024,
        "missing/nonregular/oversized JSON receipt",
    )
    data = path.read_bytes()
    require(len(data) <= 4 * 1024 * 1024, "JSON receipt grew beyond cap")
    # The native auditor already rejects duplicates and nonfinite JSON tokens.
    result = NATIVE.decode_json(data)
    pending = [result]
    while pending:
        value = pending.pop()
        if type(value) is dict:
            pending.extend(value.values())
        elif type(value) is list:
            pending.extend(value)
        elif type(value) is float:
            require(np.isfinite(value), "nonfinite decoded JSON number")
    return result


NATIVE_SPEC = importlib.util.spec_from_file_location(
    "replay_obs_native", Path(__file__).with_name("process-native-pcm.py")
)
NATIVE = importlib.util.module_from_spec(NATIVE_SPEC)
NATIVE_SPEC.loader.exec_module(NATIVE)


def pinned_input(item):
    require(
        type(item) is dict and set(item) == {"path", "sha256"},
        "invalid hash-pinned input",
    )
    path = Path(item["path"]).resolve()
    require(
        path.is_file()
        and not path.is_symlink()
        and path.stat().st_size <= MAX_FILE_BYTES,
        "missing or oversized pinned input",
    )
    require(
        type(item["sha256"]) is str
        and len(item["sha256"]) == 64
        and file_hash(path) == item["sha256"].upper(),
        "pinned input hash mismatch",
    )
    return path


def preflight_archive(archive):
    """Separate512MiB source and derived budgets across all owners, not each."""
    if (archive / "native-pcm").is_dir():
        paths = [archive]
    else:
        receipt = load_json(archive / "summary.json")
        count = NATIVE.number(receipt, "expected_owners", 8, 1)
        require(
            NATIVE.number(receipt, "sessions", 8, 1) == count,
            "archive owner coverage incomplete",
        )
        paths = []
        for path in archive.iterdir():
            if path.name.startswith("owner-"):
                require(
                    path.is_dir() and not path.is_symlink(), "invalid owner directory"
                )
                paths.append(path)
                require(len(paths) <= 8, "owner count cap reached")
        require(len(paths) == count, "archive owner count mismatch")
    source_bytes = derived_bytes = 0
    for owner in paths:
        input_bytes, chunks = NATIVE.owner_files(owner, MAX_FILE_BYTES)
        _, summary, artifact, _, _, _, _ = NATIVE.validate_receipts(
            owner, input_bytes, chunks
        )
        source_bytes += input_bytes
        derived_bytes += (
            summary["accepted_frames"] * 16 + artifact["requested_frames"] * 17
        )
        require(source_bytes <= MAX_FILE_BYTES, "combined source archive cap exceeded")
        require(derived_bytes <= MAX_FILE_BYTES, "combined derived replay cap exceeded")
    return dict(
        owners=len(paths), input_bytes=source_bytes, derived_bytes=derived_bytes
    )


def decoded_track_hash(decoder, recording, index):
    flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    command = [
        str(decoder),
        "-nostdin",
        "-v",
        "error",
        "-i",
        str(recording),
        "-map",
        "0:a:%d" % index,
        "-c:a",
        "pcm_f32le",
        "-f",
        "hash",
        "-hash",
        "sha256",
        "-",
    ]
    decoded = subprocess.run(
        command, check=True, capture_output=True, timeout=90, creationflags=flags
    )
    text = decoded.stdout.decode("ascii").strip()
    require(
        len(text) == 71 and text.startswith("SHA256="),
        "recording decoder did not produce one SHA256 receipt",
    )
    return text[7:].upper()


def pcm_hash(values):
    digest = hashlib.sha256()
    for start in range(0, len(values), BLOCK_FRAMES):
        digest.update(
            np.asarray(values[start : start + BLOCK_FRAMES], dtype="<f4").tobytes()
        )
    return digest.hexdigest().upper()


def verify_recording_binding(manifest, wav_path, prior, recorded, rate=None):
    require(
        "recording" in manifest and "decoder" in manifest and "probe" in manifest,
        "recording/return track binding is required",
    )
    item = manifest["recording"]
    require(
        type(item) is dict
        and set(item) == {"path", "sha256", "return_audio_index"}
        and type(item["return_audio_index"]) is int
        and item["return_audio_index"] == 1,
        "recording return audio index must be1, not reference0",
    )
    recording = pinned_input({key: item[key] for key in ("path", "sha256")})
    decoder = pinned_input(manifest["decoder"])
    probe = pinned_input(manifest["probe"])
    require(
        type(prior.get("recording")) is str
        and Path(prior["recording"]).resolve() == recording
        and type(prior.get("recording_bytes")) is int
        and prior["recording_bytes"] == recording.stat().st_size,
        "prior control evidence belongs to another recording",
    )
    require(
        recorded is not None and wav_path.resolve() != recording,
        "recording and decoded return WAV must be distinct",
    )
    flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    metadata = subprocess.run(
        [str(probe), "-v", "error", "-show_streams", "-of", "json", str(recording)],
        check=True,
        capture_output=True,
        timeout=30,
        creationflags=flags,
    )
    streams = [
        s
        for s in NATIVE.decode_json(metadata.stdout)["streams"]
        if s["codec_type"] == "audio"
    ]
    if rate is None:
        from scipy.io import wavfile

        rate, header = wavfile.read(wav_path, mmap=True)
        if isinstance(header, np.memmap):
            header._mmap.close()
    require(
        len(streams) == 2
        and all(
            int(s["sample_rate"]) == rate
            and s["channels"] == 2
            and s["codec_name"] in ("pcm_s24le", "pcm_f32le")
            for s in streams
        ),
        "recording needs exactly two lossless stereo tracks at the WAV sample rate",
    )
    digest = pcm_hash(recorded)
    require(
        digest == decoded_track_hash(decoder, recording, 1),
        "decoded WAV does not match recording return audio1",
    )
    return dict(
        recording=item,
        decoder=manifest["decoder"],
        probe=manifest["probe"],
        sample_rate=rate,
        decoded_return_pcm_sha256=digest,
        decoded_wav=str(wav_path),
        verification="independent whole-track decode; no audio client",
    )


def render(output, report, owners, recorded, rate):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    fig, axes = plt.subplots(3, 1, figsize=(13, 10), constrained_layout=True)
    fig.suptitle(
        "Actual Process SRC replay -> retained OBS | "
        + report["status"]
        + "\nDIAGNOSTIC ONLY: no physical latency or continuous-output qualification"
    )
    for row in report["windows"]:
        time = row["recording_time_s"]
        color = "#167d8d" if row["association_compatible"] else "#ae3e32"
        for candidate in row["candidates"]:
            for channel in candidate["channels"]:
                axes[0].scatter(
                    time, channel.get("heldout_correlation", np.nan), color=color, s=28
                )
                axes[1].scatter(
                    time, channel.get("heldout_nrmse", np.nan), color=color, s=28
                )
        if row["association_compatible"]:
            candidate = next(c for c in row["candidates"] if c["owner"] == row["owner"])
            channel = candidate["channels"][0]
            center = row["recording_start_frame"] + (rate // 2 - 1) / 2
            locations = np.arange(-64, 64) + center
            positions = channel["replay_frame_at_recording_center"] + (
                locations - center
            ) * (1 + channel["correction_ppm"] * 1e-6)
            predicted = LINEAR.predict_linear(owners[row["owner"]][0][:, 0], positions)
            if predicted is not None:
                axes[2].plot(
                    np.arange(128) / rate * 1000,
                    predicted * channel["gain"] + channel["dc"],
                    label="actual replay %.2fs" % time,
                    alpha=0.7,
                )
                axes[2].plot(
                    np.arange(128) / rate * 1000,
                    np.interp(
                        locations,
                        np.arange(int(center) - 66, int(center) + 67),
                        recorded[int(center) - 66 : int(center) + 67, 0],
                    ),
                    "--",
                    label="OBS %.2fs" % time,
                    alpha=0.6,
                )
    axes[0].axhline(GATES["heldout_correlation_min"], color="#555555", linestyle="--")
    axes[0].set(
        ylabel="Held-out NCC",
        title="All prespecified windows / both channels; red remains refused",
    )
    axes[1].axhline(GATES["heldout_nrmse_max"], color="#555555", linestyle="--")
    axes[1].set(
        ylabel="Held-out NRMSE",
        xlabel="Recording time (s)",
        title="Unchanged gates; native gaps and original refusals are not erased",
    )
    axes[2].set(
        xlabel="Local waveform (ms)",
        ylabel="PCM",
        title="Accepted held-out samples only; owner clocks are not physical delay",
    )
    if any(r["association_compatible"] for r in report["windows"]):
        axes[2].legend(fontsize=8, ncol=2)
    else:
        axes[2].text(
            0.05,
            0.5,
            "No accepted owner association; no waveform overlay invented.",
            transform=axes[2].transAxes,
        )
    for axis in axes:
        axis.grid(alpha=0.2)
    fig.savefig(output / "overview.png", dpi=140)
    plt.close(fig)


def finish_report(output, report, owners, recorded, rate, *, make_plot):
    if make_plot:
        try:
            render(output, report, owners, recorded, rate)
        except (OSError, ValueError, ImportError) as error:
            report["status"] = "refused"
            report["refusals"].append("PNG visualization failed: " + str(error)[:1024])
    data = json.dumps(report, indent=2, allow_nan=False) + "\n"
    with (output / "summary.json").open("x", encoding="utf-8") as stream:
        stream.write(data)


def analyze(manifest_path, output_directory, *, make_plot=True):
    manifest_path, output = (
        Path(manifest_path).resolve(),
        Path(output_directory).resolve(),
    )
    manifest = load_json(manifest_path)
    require(
        set(manifest)
        == {
            "schema_version",
            "archive_directory",
            "recorded_wav",
            "prior_summary",
            "recording",
            "decoder",
            "probe",
            "reference_wav",
            "control_wav",
        }
        and type(manifest["schema_version"]) is int
        and manifest["schema_version"] == 1,
        "invalid association manifest schema",
    )
    archive = Path(manifest["archive_directory"]).resolve()
    for source in (
        archive,
        manifest_path,
        Path(manifest["recorded_wav"]["path"]).resolve(),
        Path(manifest["prior_summary"]["path"]).resolve(),
        Path(manifest["recording"]["path"]).resolve(),
        Path(manifest["decoder"]["path"]).resolve(),
        Path(manifest["probe"]["path"]).resolve(),
        Path(manifest["reference_wav"]["path"]).resolve(),
        Path(manifest["control_wav"]["path"]).resolve(),
    ):
        require(
            not output.is_relative_to(source) and not source.is_relative_to(output),
            "output must not overlap immutable source evidence",
        )
    output.mkdir(parents=True, exist_ok=False)
    report = dict(
        schema_version=1,
        status="refused",
        qualified=False,
        physical_qualified=False,
        waveform_continuity_qualified=False,
        physical_latency_ms=None,
        archive_status=None,
        windows=[],
        refusals=[],
        scope="Fixed actual Process SRC replay to independent OBS coordinates; local recorder mapping only",
        gates=GATES,
        manifest_sha256=file_hash(manifest_path),
        limits=[
            "No fitted production SRC phase, rate, reset or generation",
            "Per-window owner association is not owner lifetime or Engine/render epoch proof",
            "Unknown replay neighbors refuse, not certified silent samples",
            "Original native faults and old waveform refusals remain unchanged",
            "Uncalibrated OBS coordinates are not physical latency or whole-run percentiles",
        ],
    )
    owners, maps, recorded, rate = {}, [], None, None
    try:
        recorded_path = pinned_input(manifest["recorded_wav"])
        prior_path = pinned_input(manifest["prior_summary"])
        prior = load_json(prior_path)
        report["source"] = dict(
            archive_directory=str(archive),
            recorded_wav=manifest["recorded_wav"],
            prior_summary=manifest["prior_summary"],
        )
        report["original_control"] = prior.get("control")
        require(
            prior.get("capture_complete") is True
            and prior.get("capture_error") is None,
            "original recording capture incomplete",
        )
        require(
            type(prior.get("control")) is dict
            and prior["control"].get("established") is True,
            "original isolated control not established",
        )
        from scipy.io import wavfile

        rate, recorded = wavfile.read(recorded_path, mmap=True)
        if isinstance(recorded, np.memmap):
            maps.append(recorded)
        require(
            type(rate) is int and 8000 <= rate <= 192000,
            "recording sample rate unsupported",
        )
        require(
            recorded.dtype == np.dtype("<f4"),
            "exact decoded float32 WAV required; no implicit conversion",
        )
        stereo_shape(recorded)
        report["recording_binding"] = verify_recording_binding(
            manifest, recorded_path, prior, recorded, rate
        )
        reference_path = pinned_input(manifest["reference_wav"])
        control_path = pinned_input(manifest["control_wav"])
        other = []
        for path in (reference_path, control_path):
            current_rate, values = wavfile.read(path, mmap=True)
            if isinstance(values, np.memmap):
                maps.append(values)
            require(
                current_rate == rate and values.dtype == np.dtype("<f4"),
                "reference/control rate/format mismatch",
            )
            stereo_shape(values)
            other.append(values)
        reference, control = other
        require(
            len(reference) == len(recorded) and len(control) <= 10 * rate,
            "incomplete reference or oversized control",
        )
        require(
            pcm_hash(reference)
            == decoded_track_hash(
                Path(manifest["decoder"]["path"]),
                Path(manifest["recording"]["path"]),
                0,
            ),
            "reference WAV does not match recording audio0",
        )
        limit = min(len(reference), 60 * rate)
        report["fresh_control"] = LINEAR.ISOLATED.control_isolation(
            control, reference[:limit], recorded[:limit], rate
        )
        require(
            report["fresh_control"]["established"], "fresh isolated control refused"
        )
        report["preflight"] = preflight_archive(archive)
        audit = NATIVE.audit_archive(archive, output / "replay", make_plot=False)
        report["archive_status"] = audit["status"]
        require(
            audit["status"] == "structurally_complete",
            "fresh native PCM/cursor audit refused",
        )
        audit_owners = (
            [(archive.name, output / "replay")]
            if (archive / "native-pcm").is_dir()
            else [
                (item["directory"], output / "replay" / item["directory"])
                for item in audit["owners"]
            ]
        )
        for name, directory in audit_owners:
            receipt = load_json(directory / "summary.json")
            require(
                receipt["origin"]["consumer_sample_rate"] == rate,
                "no implicit sample-rate conversion of replay/recorder",
            )
            count = receipt["replay"]["requested_frames"]
            require(0 < count <= MAX_FRAMES, "replay frame cap exceeded")
            replay_path, valid_path = (
                directory / "reconstructed.f32le",
                directory / "reconstructed-valid.u8",
            )
            require(
                replay_path.stat().st_size == count * 8
                and valid_path.stat().st_size == count,
                "re-audited output array size mismatch",
            )
            replay = np.memmap(replay_path, mode="r", dtype="<f4", shape=(count, 2))
            valid = np.memmap(valid_path, mode="r", dtype="u1", shape=(count,))
            maps.extend([replay, valid])
            owners[name] = (replay, valid)
        report["windows"] = associate_windows(owners, recorded, prior["windows"], rate)
        report["associated_windows"] = sum(
            row["association_compatible"] for row in report["windows"]
        )
        report["selected_windows"] = len(report["windows"])
        if report["associated_windows"] == report["selected_windows"]:
            report["status"] = "all_selected_windows_associated"
        else:
            report["refusals"].append(
                "one or more prespecified windows refused; all retained"
            )
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        OverflowError,
        subprocess.SubprocessError,
    ) as error:
        report["refusals"].append((str(error) or type(error).__name__)[:1024])
    try:
        finish_report(output, report, owners, recorded, rate, make_plot=make_plot)
    finally:
        for value in maps:
            value._mmap.close()
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output-directory", required=True, type=Path)
    parser.add_argument("--no-plot", action="store_true")
    args = parser.parse_args(argv)
    try:
        result = analyze(
            args.manifest, args.output_directory, make_plot=not args.no_plot
        )
    except (OSError, ValueError, KeyError, TypeError) as error:
        print("Refused: " + str(error), file=sys.stderr)
        return 2
    print(json.dumps({k: result[k] for k in ("status", "qualified", "refusals")}))
    return 0 if result["status"] == "all_selected_windows_associated" else 2


if __name__ == "__main__":
    raise SystemExit(main())
