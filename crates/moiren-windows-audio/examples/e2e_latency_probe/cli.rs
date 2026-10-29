use super::live;
use super::pcm::{PacketTimeline, PcmSchedule, analyze};
use anyhow::{Context, Result, bail, ensure};
use moiren_windows_audio::latency::qualification::{
    QualificationExpectation, QualificationLevel, QualificationReport, evaluate_qualification,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::Path,
};

#[path = "process_replay.rs"]
mod process_replay;

const HELP: &str = "Offline evidence analysis plus an explicitly qualified live Process Loopback harness.
evaluate --reports FILE --expectation FILE --level experimental|default --max-input-bytes N --max-runs N --output FILE
analyze-pcm --pcm FILE --schedule FILE --max-input-bytes N --output FILE [--packets FILE --max-packets N]
replay-process --trace FILE --candidates FILE --process-pid PID --process-created-100ns N
    --owner-generation N --max-input-bytes 1..16777216 --output NEWFILE [--window-policy FILE]
Offline finite Process queue replay only, not qualification: no safe/stability/latency or source-age claim.
Trace document: schema 1 or 2; input hard cap 16 MiB per file, output hard cap 1 MiB independently.
Candidates: schema_version=1, candidates:[explicit ProcessBridgeOptions], 1..16 entries; no defaults.
Replay exits 0=completed candidate sweep (quality/refusals retained), 1=trace refused, 2=invalid input/IO.
Optional window policy: schema_version=1, policy:{max_initial_unserved_frames,max_initial_prime_discarded_frames,
    min_output_frames,min_output_reads,min_output_span_100ns}; all thresholds explicit; min frames/span >0, reads >=2.
With --window-policy, exit 0 requires every candidate replayed with no_violation_in_window;
    refused, observed_violation or inconclusive candidates exit 1 with raw reports/refusals retained; invalid policy/IO exits 2.
Window acceptance is finite caller-supplied queue evidence only; qualified remains false, not safety/stability/latency qualification.
live-process-loopback --source-pid PID --output ENDPOINT --artifact-dir DIR [--seconds 1..600]
live-vad --stable-endpoint-id ID --name NAME --output ENDPOINT --artifact-dir DIR [--seconds 1..600]
live-physical --input ID --output ID --recorder-config FILE --artifact-dir NEWDIR --seconds 1..600
Physical capture -> Bus -> physical render with independent bounded recorder; externally supplied signal and wiring.
Separate --capture-mode/--capture-period-frames and --render-mode/--render-period-frames; always unqualified.
live-vad --stable-endpoint-id PLAY_ID --name PLAY_NAME --vad-microphone-id MIC_ID --vad-microphone-name MIC_NAME
    --source-schedule FILE --vad-collection FILE --artifact-dir DIR --seconds 1..600
Virtual route is exact playback -> Bus -> microphone, Internal software evidence only, always unqualified.
Distinct explicit stable IDs; no physical --output/render-only options or calibration. Collection schema2 may omit input_endpoint_id;
only the exact owned capture endpoint is resolved, and any supplied recorder ID must match. Stereo duplication is not an independent reference.
record-measurement --input ENDPOINT --artifact-dir NEW_DIR --seconds 1..7200
    --ring-frames N --max-pcm-bytes N --max-metadata-bytes N --max-packets N [--dry-run]
Independent native capture retains bounded raw PCM/packet metadata; never claims physical qualification.
record-process --source-pid PID --source-created-100ns N --artifact-dir NEW_DIR
    --seconds 1..600 --max-frames 1..33554432 [--dry-run]
Bounded process-tree output recording as 48 kHz stereo f32le; capture-only, no render or volume writes.
Identity is rechecked before activation. PCM and partial native reports survive faults; always unqualified.
Process positions cannot prove waveform continuity, source age or physical presentation; not a physical reference.
    [--measurement-boundary internal|wasapi-loopback|physical-cable|external-dual-channel]
    [--measurement-pcm FILE --schedule FILE] [--calibration FILE]
    [--input-target-frames N --input-capacity-frames N --input-jitter-frames N]
Explicit input capacity/jitter/packet/gap flags require --input-target-frames; omission preserves compatibility.
    [--vad-recovery-cap-periods 1..8] (live-vad only; omitted preserves 8)
VAD cap limits virtual-clock catch-up, not physical whole-wake demand or measured latency; smaller caps may discard more overdue periods.
    [--vad-minimum-period] (live-vad only; omitted preserves the default exclusive period request)
Period requests are unqualified; actual native buffer frames and provider packet observations remain separate.
    [--render-target-frames N --wake-margin-frames N --callback-budget-frames N]
    [--render-mode shared|exclusive --render-period-frames 64|128|256|512|1024|2048|4096]
Physical render request is common to Process/VAD physical output, not the external VAD source or virtual microphone.
Omission preserves existing Shared/default-period behavior. Explicit period must be observed exactly without Shared fallback;
Exclusive requires full-capacity rendering. Requests, observed period/capacity and measured delay are separate evidence.
    [--validation-mode latency-sensitive|correctness --jitter-tolerance-ms 0..3000]
Correctness requires --vad-collection or owned --process-collection; timing faults are retained without early abort.
Correctness exit 0 means operational collection and cleanup completed; quality_passed and qualified stay separate.
    [--report FILE --manifest FILE --run-id ID --mode disabled|summary|trace]
    [--diagnostic-soak --seconds 1..7200 --observation-interval-seconds 5..60]
Diagnostic soak disables capture PCM; bounded periodic JSON observations do not qualify waveform continuity or physical latency.
PCM: headerless little-endian interleaved stereo f32, channels 0/1, explicit rate in schedule.
Schedule: schema_version=1, sample_rate, reference_channel, processed_channel, marker_frames,
marker_amplitude, reference_search_frames, max_delay_frames, max_marker_frames, max_window_frames,
max_markers, max_pcm_frames, max_correlation_samples_per_marker, max_total_correlation_samples,
max_total_window_frames, min_correlation, min_peak_margin, min_rms, windows:[{start_frame}].
Optional rate_search:{min_rate_ppm,max_rate_ppm,rate_step_ppm,max_hypotheses,max_total_correlation_samples}.
Positive ppm stretches a zero-extended linear template; explicit grid max129 hypotheses. Full warped windows and both-channel
work (including a second rate-score pass) must fit schedule limits before reading PCM. Selected ppm is a template hypothesis,
not clock calibration; integer onset resolution is not physical uncertainty. Raw common-recorder delays are never detrended.
Opt-in rate search reports schema3 (PCM) or4 (packet-validated); omitted retains legacy schemas1/2 and behavior.
Optional trend:{min_markers,min_span_frames}: min_markers>=3 within marker cap; positive span within2^53-1 frames.
Trend opts into schema5 (PCM) or6 (packet-validated). Reports centered relative delay slope and residual RMS sqrt(SSE/n),
not hardware clock skew, physical uncertainty or adjusted latency. Raw delays/distribution retain every accepted outlier.
Failed markers and invalid packets BETWEEN windows split fits. Missing packets means unverified continuity;
clean metadata is diagnostic only, not a native epoch or full-waveform continuity. Insufficient segments retain reasons.
Imported live reports retain trend-only analysis in trend_measurement; combined rate+trend in rate_search_measurement.
Without --packets, legacy schema_version=1 reports successful markers only, not capture continuity.
With --packets, schema_version=2 validates measurement_sink JSONL against all stored PCM frames.
Metadata uses --max-input-bytes (hard cap 256 MiB), --max-packets (1..1000000), 1024 bytes/record.
Lost/reordered/reset/unknown-timestamp/silent packet windows are invalidated; failures are retained.
Native epoch, hardware clock provenance, marker startup phase and physical boundary are not inferred.
Outputs must be new files. Exit 0=accepted analysis/evaluation or completed correctness collection, 1=failed/incomplete/unqualified,
2=invalid input or IO error. Live mode never claims physical qualification without a native
source, measurement artifact and independently supplied calibration.";

fn read_json<T: DeserializeOwned>(path: &str, limit: u64) -> Result<T> {
    ensure!(
        limit > 0 && limit < usize::MAX as u64,
        "invalid JSON input byte cap"
    );
    let file = File::open(path).with_context(|| format!("open {path}"))?;
    ensure!(
        file.metadata()?.len() <= limit,
        "JSON input byte cap exceeded: {path}"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "JSON input grew beyond byte cap: {path}"
    );
    serde_json::from_slice(&bytes).with_context(|| format!("parse {path}"))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create new evidence file {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn run(arguments: Vec<String>) -> Result<i32> {
    if arguments.is_empty() || arguments == ["--help"] {
        println!("{HELP}");
        return Ok(0);
    }
    let command = &arguments[0];
    if command == "live-physical" {
        return live::physical::run(arguments[1..].to_vec());
    }
    if command == "replay-process" {
        return process_replay::run(arguments[1..].to_vec());
    }
    if command == "process-source-worker" {
        #[cfg(any(windows, test))]
        return live::process_source::run_worker(arguments[1..].to_vec());
        #[cfg(not(any(windows, test)))]
        bail!("process source worker requires Windows");
    }
    if command == "record-measurement" {
        if arguments[1..] == ["--list"] {
            return crate::measurement::list_inputs();
        }
        return crate::measurement::run(arguments[1..].to_vec());
    }
    if command == "record-process" {
        return crate::process_recording::run(arguments[1..].to_vec());
    }
    if command == "controlled-load-worker" {
        return crate::controlled_load::run_worker(arguments[1..].to_vec());
    }
    if matches!(command.as_str(), "live-process-loopback" | "live-vad") {
        if arguments[1..] == ["--help"] {
            return live::run(vec!["--help".into()]);
        }
        let mut live_arguments = arguments[1..].to_vec();
        if command == "live-vad" {
            live_arguments.insert(0, "--source-kind".into());
            live_arguments.insert(1, "vad-playback".into());
        } else {
            live_arguments.insert(0, "--source-kind".into());
            live_arguments.insert(1, "process-loopback".into());
        }
        return live::run(live_arguments);
    }
    let permitted = match command.as_str() {
        "evaluate" => &[
            "--reports",
            "--expectation",
            "--level",
            "--max-input-bytes",
            "--max-runs",
            "--output",
        ][..],
        "analyze-pcm" => &[
            "--pcm",
            "--schedule",
            "--max-input-bytes",
            "--output",
            "--packets",
            "--max-packets",
        ][..],
        _ => bail!("unknown offline command: {command}"),
    };
    ensure!(
        (arguments.len() - 1).is_multiple_of(2),
        "options require separate values"
    );
    let mut options = BTreeMap::new();
    let (pairs, remainder) = arguments[1..].as_chunks::<2>();
    ensure!(remainder.is_empty(), "options require separate values");
    for pair in pairs {
        ensure!(
            permitted.contains(&pair[0].as_str()),
            "unknown option {}",
            pair[0]
        );
        ensure!(
            options.insert(pair[0].as_str(), pair[1].as_str()).is_none(),
            "duplicate option {}",
            pair[0]
        );
    }
    for name in permitted {
        if matches!(*name, "--packets" | "--max-packets") {
            continue;
        }
        ensure!(options.contains_key(name), "missing {name}");
    }
    ensure!(
        options.contains_key("--packets") == options.contains_key("--max-packets"),
        "--packets and --max-packets must be supplied together"
    );
    let limit: u64 = options["--max-input-bytes"]
        .parse()
        .context("parse JSON byte cap")?;
    let output = Path::new(options["--output"]);
    if command == "evaluate" {
        let reports: Vec<QualificationReport> = read_json(options["--reports"], limit)?;
        let expected: QualificationExpectation = read_json(options["--expectation"], limit)?;
        let level = match options["--level"] {
            "experimental" => QualificationLevel::Experimental30Minutes,
            "default" => QualificationLevel::DefaultTwoHourRepeated,
            _ => bail!("level must be experimental or default"),
        };
        let max_runs = options["--max-runs"].parse().context("parse run cap")?;
        let result = evaluate_qualification(&reports, &expected, level, max_runs);
        write_json(
            output,
            &serde_json::json!({
                "schema_version": 1, "mode": "offline_report_evaluation", "qualified": result.is_ok(),
                "level": level, "error": result.err(), "reports": reports, "expectation": expected,
                "gaps": ["Evaluation checks submitted evidence; it does not authenticate artifacts, collect PCM or complete the Stage 7 harness."]
            }),
        )?;
        Ok(i32::from(result.is_err()))
    } else {
        let schedule: PcmSchedule = read_json(options["--schedule"], limit)?;
        let mut file = File::open(options["--pcm"]).context("open stereo PCM capture")?;
        let length = file.metadata()?.len();
        if let Some(path) = options.get("--packets") {
            let max_packets = options["--max-packets"]
                .parse()
                .context("parse packet cap")?;
            let metadata = File::open(path).context("open packet metadata")?;
            ensure!(
                metadata.metadata()?.len() <= limit,
                "packet metadata byte cap exceeded"
            );
            let timeline = PacketTimeline::read(
                metadata,
                limit,
                max_packets,
                length / 8,
                schedule.sample_rate,
            )?;
            let analysis = timeline.analyze(&mut file, length, schedule)?;
            let code = i32::from(analysis.failed());
            write_json(output, &analysis)?;
            Ok(code)
        } else {
            let analysis = analyze(&mut file, length, schedule)?;
            let code = i32::from(analysis.failed_markers != 0);
            write_json(output, &analysis)?;
            Ok(code)
        }
    }
}
