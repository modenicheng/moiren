#!/usr/bin/env python3
"""Account for the entire OBS return with frozen actual-replay coordinates.

The earliest independently associated anchor per owner fixes its recorder map.
Later anchors may contradict that map, never repair it. Every recorder frame is
in the denominator, including training, silence, UNKNOWN and unsupported edges.
This is an offline model diagnostic, not native or physical qualification.
"""

import argparse
from bisect import bisect_right
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np


sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "coverage_association", Path(__file__).with_name("process-replay-obs.py")
)
ASSOCIATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ASSOCIATION)
GATES = ASSOCIATION.GATES.copy()
BLOCK = 64
# This additional diagnostic bound can only refuse a map. It never relaxes an
# existing waveform gate or permits local coordinate corrections.
ANCHOR_DISAGREEMENT_FRAMES = 0.25
MAX_ANCHORS = 128
MAX_DETAIL_RUNS = 4096


def locations(channel, positions):
    return channel["replay_frame_at_recording_center"] + (
        positions - channel["recording_center_frame"]
    ) * (1 + channel["correction_ppm"] * 1e-6)


def crosses_generation(boundaries, first, last):
    index = bisect_right(boundaries, first)
    return index < len(boundaries) and boundaries[index] <= np.floor(last) + 1


def frozen_maps(owners, anchors, rate):
    ASSOCIATION.require(1 <= len(anchors) <= MAX_ANCHORS, "bounded anchors required")
    maps, training = {}, []
    for anchor in sorted(anchors, key=lambda a: a["recording_start_frame"]):
        name = anchor["owner"]
        ASSOCIATION.require(name in owners, "anchor owner absent from audited archive")
        channels = anchor["channels"]
        start, width = anchor["recording_start_frame"], rate // 2
        ASSOCIATION.require(
            type(start) is int and start >= 0 and len(channels) == 2,
            "invalid stereo anchor interval",
        )
        for channel in channels:
            for key in (
                "recording_center_frame",
                "replay_frame_at_recording_center",
                "correction_ppm",
                "gain",
                "dc",
            ):
                ASSOCIATION.require(
                    type(channel[key]) in (int, float) and np.isfinite(channel[key]),
                    "nonfinite or missing fixed-map parameter",
                )
            ASSOCIATION.require(
                abs(channel["correction_ppm"]) < GATES["model_correction_ppm_max"]
                and abs(channel["gain"]) > 1e-12
                and abs(channel["recording_center_frame"] - (start + (width - 1) / 2))
                < 1e-6,
                "invalid fixed-map rate/gain/center",
            )
        a, b = channels
        ASSOCIATION.require(
            abs(
                a["replay_frame_at_recording_center"]
                - b["replay_frame_at_recording_center"]
            )
            * 1000
            / rate
            <= GATES["stereo_offset_disagreement_ms_max"]
            and abs(a["correction_ppm"] - b["correction_ppm"])
            <= GATES["stereo_rate_disagreement_ppm_max"],
            "anchor stereo disagreement",
        )
        training.extend(
            [(start, start + width // 4), (start + 3 * width // 4, start + width)]
        )
        if name not in maps:
            maps[name] = dict(
                channels=channels, anchor_start_frame=start, consistent=True, checks=[]
            )
        else:
            fixed = maps[name]
            points = np.array([start, start + width - 1], dtype=np.float64)
            disagreement = max(
                float(
                    np.max(np.abs(locations(before, points) - locations(after, points)))
                )
                for before, after in zip(fixed["channels"], channels)
            )
            fixed["checks"].append(
                dict(anchor_start_frame=start, disagreement_frames=disagreement)
            )
            fixed["consistent"] &= disagreement <= ANCHOR_DISAGREEMENT_FRAMES
    return maps, training


def evaluate(
    replay, valid, target, start, channels, *, nrmse_limit=None, boundaries=()
):
    positions = np.arange(start, start + len(target), dtype=np.float64)
    if not np.isfinite(target).all():
        return dict(category="nonfinite_recording")
    metrics = []
    for c, channel in enumerate(channels):
        points = locations(channel, positions)
        if points[0] < 0 or points[-1] >= len(replay) - 1:
            return dict(category="outside_replay")
        if crosses_generation(boundaries, points[0], points[-1]):
            return dict(category="generation_boundary")
        indices = np.floor(points).astype(np.intp)
        if not ((valid[indices] == 1) & (valid[indices + 1] == 1)).all():
            return dict(category="unknown_replay")
        left = np.asarray(replay[indices, c], dtype=np.float64)
        right = np.asarray(replay[indices + 1, c], dtype=np.float64)
        prediction = (left + (right - left) * (points - indices)) * channel[
            "gain"
        ] + channel["dc"]
        if not np.isfinite(prediction).all():
            return dict(category="nonfinite_replay")
        x, y = prediction - prediction.mean(), target[:, c] - target[:, c].mean()
        energy = float(np.sqrt(np.mean(y * y)))
        predicted_energy = float(np.sqrt(np.mean(x * x)))
        if energy <= 1e-10 or predicted_energy <= 1e-10 or len(target) < 3:
            return dict(category="uninformative_signal")
        nrmse = float(np.sqrt(np.mean((prediction - target[:, c]) ** 2)) / energy)
        correlation = float(np.mean(x * y) / (energy * predicted_energy))
        metrics.append(dict(nrmse=nrmse, correlation=correlation))
    compatible = all(
        m["nrmse"] <= (GATES["local_nrmse_max"] if nrmse_limit is None else nrmse_limit)
        and m["correlation"] >= GATES["heldout_correlation_min"]
        for m in metrics
    )
    return dict(
        category="fixed_map_compatible" if compatible else "waveform_residual",
        nrmse_max=max(m["nrmse"] for m in metrics),
        correlation_min=min(m["correlation"] for m in metrics),
    )


def scan_coverage(owners, recorded, anchors, rate, *, generation_boundaries=None):
    ASSOCIATION.require(
        type(rate) is int and 8000 <= rate <= 192000, "supported rate8..192kHz"
    )
    ASSOCIATION.stereo_shape(recorded)
    ASSOCIATION.require(1 <= len(owners) <= 8, "one to eight audited owners required")
    for replay, valid in owners.values():
        ASSOCIATION.stereo_shape(replay)
        ASSOCIATION.validate_mask(valid, len(replay))
    maps, training = frozen_maps(owners, anchors, rate)
    generation_boundaries = generation_boundaries or {}
    ASSOCIATION.require(
        set(generation_boundaries) <= set(owners), "generation owner absent"
    )
    for name, points in generation_boundaries.items():
        ASSOCIATION.require(
            len(points) <= 65536
            and all(type(p) is int and 0 < p < len(owners[name][0]) for p in points),
            "invalid generation boundaries",
        )
        generation_boundaries[name] = sorted(set(points))
    boundaries = {0, len(recorded)}
    boundaries.update(range(0, len(recorded), BLOCK))
    boundaries.update(range(0, len(recorded), rate // 2))
    for start, stop in training:
        boundaries.update(
            [max(0, min(len(recorded), start)), max(0, min(len(recorded), stop))]
        )
    boundaries = sorted(boundaries)
    counts, blocks, trace, aggregates = Counter(), [], [], {}
    for start, stop in zip(boundaries, boundaries[1:]):
        if len(blocks) >= MAX_DETAIL_RUNS:
            counts["analysis_cap_refused"] += len(recorded) - start
            blocks.append(
                dict(
                    start_frame=start,
                    stop_frame=len(recorded),
                    category="analysis_cap_refused",
                    owner=None,
                )
            )
            break
        candidates = {}
        for name, fixed in maps.items():
            if not fixed["consistent"]:
                candidates[name] = dict(category="anchor_conflict")
            else:
                candidates[name] = evaluate(
                    *owners[name],
                    recorded[start:stop],
                    start,
                    fixed["channels"],
                    boundaries=generation_boundaries.get(name, ()),
                )
                parent = start // (rate // 2) * (rate // 2)
                key = name, parent
                if key not in aggregates:
                    aggregates[key] = evaluate(
                        *owners[name],
                        recorded[parent : min(len(recorded), parent + rate // 2)],
                        parent,
                        fixed["channels"],
                        nrmse_limit=GATES["heldout_nrmse_max"],
                        boundaries=generation_boundaries.get(name, ()),
                    )
                if (
                    candidates[name]["category"] == "fixed_map_compatible"
                    and aggregates[key]["category"] != "fixed_map_compatible"
                ):
                    candidates[name]["category"] = (
                        "waveform_residual"
                        if aggregates[key]["category"] == "waveform_residual"
                        else "aggregate_context_refused"
                    )
        accepted = [
            name
            for name, r in candidates.items()
            if r["category"] == "fixed_map_compatible"
        ]
        if len(accepted) > 1:
            category, owner = "ambiguous_owner", None
        elif len(accepted) == 1:
            category, owner = "fixed_map_compatible", accepted[0]
            if any(a <= start and stop <= b for a, b in training):
                category = "anchor_training"
        else:
            owner = None
            category = "outside_replay"
            # A refusal is retained conservatively, not replaced by a different
            # owner's less informative silence/outside-support classification.
            for reason in (
                "anchor_conflict",
                "nonfinite_recording",
                "nonfinite_replay",
                "unknown_replay",
                "generation_boundary",
                "aggregate_context_refused",
                "waveform_residual",
                "uninformative_signal",
            ):
                if any(r["category"] == reason for r in candidates.values()):
                    category = reason
                    break
        counts[category] += stop - start
        if (
            blocks
            and blocks[-1]["category"] == category
            and blocks[-1]["owner"] == owner
        ):
            blocks[-1]["stop_frame"] = stop
        else:
            blocks.append(
                dict(start_frame=start, stop_frame=stop, category=category, owner=owner)
            )
        slot = start // max(BLOCK, rate // 10)
        if not trace or trace[-1]["slot"] != slot:
            trace.append(
                dict(
                    slot=slot, time_s=start / rate, nrmse_max=None, correlation_min=None
                )
            )
        for candidate in candidates.values():
            if "nrmse_max" in candidate:
                point = trace[-1]
                point["nrmse_max"] = max(
                    point["nrmse_max"] or 0, candidate["nrmse_max"]
                )
                old = point["correlation_min"]
                point["correlation_min"] = min(
                    old if old is not None else 1, candidate["correlation_min"]
                )
    return dict(
        schema_version=1,
        status="coverage_accounted",
        qualified=False,
        physical_qualified=False,
        waveform_continuity_qualified=False,
        physical_latency_ms=None,
        sample_rate=rate,
        denominator_frames=len(recorded),
        denominator="Every frame in the entire decoded return recording, including silence and unsupported edges",
        frame_counts=dict(counts),
        maps=maps,
        blocks=blocks,
        trace=trace,
        gates=GATES,
        anchor_disagreement_frames_max=ANCHOR_DISAGREEMENT_FRAMES,
        detail_run_cap=MAX_DETAIL_RUNS,
        unanchored_owners=sorted(set(owners) - set(maps)),
        generation_boundaries=generation_boundaries,
        limits=[
            "Earliest anchor per owner freezes gain/DC/recorder coordinates; no local refit",
            "64-frame blocks conservatively classify all frames in a refused block",
            "Training frames remain in denominator but are not independent coverage",
            "Silence and UNKNOWN do not establish waveform continuity",
            "Recorder mappings do not establish native-owner lifetime or physical delay",
            "Original native gap counters and startup exclusions are not changed",
        ],
    )


def render(output, report):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    fig, axes = plt.subplots(3, 1, figsize=(14, 10), constrained_layout=True)
    fig.suptitle(
        "Entire retained OBS return | FROZEN actual-replay maps | NOT qualification"
    )
    colors = dict(
        fixed_map_compatible="#26786c",
        anchor_training="#ddaa44",
        outside_replay="#b9b9b9",
        uninformative_signal="#758fa5",
    )
    categories = sorted(report["frame_counts"])
    for category in categories:
        intervals = [
            (
                row["start_frame"] / report["sample_rate"],
                (row["stop_frame"] - row["start_frame"]) / report["sample_rate"],
            )
            for row in report["blocks"]
            if row["category"] == category
        ]
        axes[0].broken_barh(
            intervals,
            (categories.index(category) - 0.4, 0.8),
            facecolors=colors.get(category, "#bc3f38"),
        )
    axes[0].set(
        yticks=range(len(categories)),
        yticklabels=categories,
        xlabel="Recording time (s)",
        title="No gaps in denominator; red/gray/training spans remain unsupported",
    )
    times = [row["time_s"] for row in report["trace"]]
    axes[1].plot(times, [r["nrmse_max"] for r in report["trace"]], color="#bc3f38")
    axes[1].axhline(GATES["local_nrmse_max"], color="#555555", linestyle="--")
    axes[1].set(
        yscale="symlog",
        ylabel="Worst 64-frame NRMSE",
        xlabel="Recording time (s)",
        title="Fixed gain/DC/coordinates; 100ms display bins, no omitted analysis blocks",
    )
    counts = report["frame_counts"]
    axes[2].barh(
        categories,
        [counts[c] / report["sample_rate"] for c in categories],
        color=[colors.get(c, "#bc3f38") for c in categories],
    )
    axes[2].set(
        xlabel="Conservatively classified seconds",
        title=f"Entire denominator: {report['denominator_frames']} frames",
    )
    for ax in axes:
        ax.grid(alpha=0.2)
    fig.savefig(output / "overview.png", dpi=150)
    plt.close(fig)


def read_generation_boundaries(owner):
    """Keep generation edges explicit, even where UNKNOWN separates spans.

    Called only after the full native auditor validates event order and closure.
    No new generation is inferred from waveform similarity or recorder time.
    """
    artifact = ASSOCIATION.load_json(owner / "final.json")["artifact"]
    edges, previous = set(), None
    for index in range(1, artifact["drained_chunks"] + 1):
        chunk = ASSOCIATION.load_json(owner / ("chunk-%08d.json" % index))
        for event in chunk["events"]:
            span = event["span"]
            if previous and span["generation"] != previous["generation"]:
                edges.update(
                    [
                        previous["output_start_frame"] + previous["frames"],
                        span["output_start_frame"],
                    ]
                )
            previous = span
    return sorted(edges)


def validate_output(manifest_path, output, manifest):
    sources = [
        Path(manifest_path).resolve(),
        Path(manifest["archive_directory"]).resolve(),
    ]
    sources.extend(
        Path(item["path"]).resolve()
        for item in manifest.values()
        if type(item) is dict and "path" in item
    )
    ASSOCIATION.require(not output.exists(), "output must be create-new")
    for source in sources:
        ASSOCIATION.require(
            not output.is_relative_to(source) and not source.is_relative_to(output),
            "output must not overlap immutable source evidence",
        )


def manifest_snapshot(path):
    path = Path(path).resolve()
    ASSOCIATION.require(
        path.is_file()
        and not path.is_symlink()
        and path.stat().st_size <= 4 * 1024 * 1024,
        "invalid manifest snapshot source",
    )
    raw = path.read_bytes()
    ASSOCIATION.require(
        len(raw) <= 4 * 1024 * 1024, "manifest snapshot grew beyond cap"
    )
    manifest = ASSOCIATION.NATIVE.decode_json(raw)
    pending = [manifest]
    while pending:
        value = pending.pop()
        if type(value) is dict:
            pending.extend(value.values())
        elif type(value) is list:
            pending.extend(value)
        elif type(value) is float:
            ASSOCIATION.require(
                np.isfinite(value), "nonfinite manifest snapshot number"
            )
    return manifest, raw, hashlib.sha256(raw).hexdigest().upper()


def analyze(manifest_path, output_directory, *, make_plot=True):
    output = Path(output_directory).resolve()
    manifest, manifest_bytes, manifest_hash = manifest_snapshot(manifest_path)
    validate_output(manifest_path, output, manifest)
    output.mkdir(parents=True, exist_ok=False)
    snapshot = output / "input-manifest.json"
    with snapshot.open("xb") as stream:
        stream.write(manifest_bytes)
    # Reuse the complete binding/control/native audit, not a cached PASS. The
    # association tool checks every source/output overlap before any replay write.
    association = ASSOCIATION.analyze(snapshot, output / "association", make_plot=False)
    report = dict(
        status="refused",
        qualified=False,
        physical_qualified=False,
        waveform_continuity_qualified=False,
        physical_latency_ms=None,
        refusals=[],
    )
    maps = []
    try:
        ASSOCIATION.require(
            association["archive_status"] == "structurally_complete",
            "fresh association audit refused",
        )
        ASSOCIATION.require(
            association["manifest_sha256"] == manifest_hash
            and ASSOCIATION.file_hash(manifest_path) == manifest_hash,
            "manifest changed between binding audit and coverage",
        )
        from scipy.io import wavfile

        rate, recorded = wavfile.read(
            ASSOCIATION.pinned_input(manifest["recorded_wav"]), mmap=True
        )
        if isinstance(recorded, np.memmap):
            maps.append(recorded)
        replay_root = output / "association" / "replay"
        archive = Path(manifest["archive_directory"])
        directories = (
            [(archive.name, replay_root)]
            if (archive / "native-pcm").is_dir()
            else [
                (p.name, p) for p in sorted(replay_root.glob("owner-*")) if p.is_dir()
            ]
        )
        owners, generation_boundaries = {}, {}
        for name, directory in directories:
            receipt = ASSOCIATION.load_json(directory / "summary.json")
            count = receipt["replay"]["requested_frames"]
            replay = np.memmap(
                directory / "reconstructed.f32le",
                mode="r",
                dtype="<f4",
                shape=(count, 2),
            )
            valid = np.memmap(
                directory / "reconstructed-valid.u8",
                mode="r",
                dtype="u1",
                shape=(count,),
            )
            maps.extend([replay, valid])
            owners[name] = replay, valid
            source_owner = (
                archive if (archive / "native-pcm").is_dir() else archive / name
            )
            generation_boundaries[name] = read_generation_boundaries(source_owner)
        anchors = []
        for row in association["windows"]:
            if row["association_compatible"]:
                anchors.extend(
                    c for c in row["candidates"] if c["association_compatible"]
                )
        report = scan_coverage(
            owners, recorded, anchors, rate, generation_boundaries=generation_boundaries
        )
        report["refusals"] = []
        report.update(
            manifest_sha256=manifest_hash,
            association_summary_sha256=ASSOCIATION.file_hash(
                output / "association" / "summary.json"
            ),
            original_association_status=association["status"],
            original_window_refusals=[
                row
                for row in association["windows"]
                if not row["association_compatible"]
            ],
        )
        if make_plot:
            render(output, report)
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        OverflowError,
        ImportError,
    ) as error:
        report["status"] = "refused"
        report["refusals"].append((str(error) or type(error).__name__)[:1024])
    finally:
        for value in maps:
            value._mmap.close()
    data = json.dumps(report, indent=2, allow_nan=False) + "\n"
    with (output / "summary.json").open("x", encoding="utf-8") as stream:
        stream.write(data)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output-directory", required=True, type=Path)
    parser.add_argument("--no-plot", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = analyze(
            args.manifest, args.output_directory, make_plot=not args.no_plot
        )
    except (OSError, ValueError, KeyError, TypeError) as error:
        print("Refused: " + str(error), file=sys.stderr)
        return 2
    print(json.dumps({k: report[k] for k in ("status", "qualified", "refusals")}))
    return 0 if report["status"] == "coverage_accounted" else 2


if __name__ == "__main__":
    raise SystemExit(main())
