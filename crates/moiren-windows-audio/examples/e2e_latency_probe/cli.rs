use super::live;
use super::pcm::{PcmSchedule, analyze};
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

const HELP: &str = "Offline evidence analysis plus an explicitly qualified live Process Loopback harness.
evaluate --reports FILE --expectation FILE --level experimental|default --max-input-bytes N --max-runs N --output FILE
analyze-pcm --pcm FILE --schedule FILE --max-input-bytes N --output FILE
live-process-loopback --source-pid PID --output ENDPOINT --artifact-dir DIR [--seconds 1..600]
live-vad --stable-endpoint-id ID --name NAME --output ENDPOINT --artifact-dir DIR [--seconds 1..600]
record-measurement --input ENDPOINT --artifact-dir NEW_DIR --seconds 1..7200
    --ring-frames N --max-pcm-bytes N --max-metadata-bytes N --max-packets N [--dry-run]
Independent native capture retains bounded raw PCM/packet metadata; never claims physical qualification.
    [--measurement-boundary internal|wasapi-loopback|physical-cable|external-dual-channel]
    [--measurement-pcm FILE --schedule FILE] [--calibration FILE]
    [--input-target-frames N --input-capacity-frames N --input-jitter-frames N]
    [--render-target-frames N --wake-margin-frames N --callback-budget-frames N]
    [--report FILE --manifest FILE --run-id ID --mode disabled|summary|trace]
    [--diagnostic-soak --seconds 1..7200 --observation-interval-seconds 5..60]
Diagnostic soak disables capture PCM; bounded periodic JSON observations do not qualify waveform continuity or physical latency.
PCM: headerless little-endian interleaved stereo f32, channels 0/1, explicit rate in schedule.
Schedule: schema_version=1, sample_rate, reference_channel, processed_channel, marker_frames,
marker_amplitude, reference_search_frames, max_delay_frames, max_marker_frames, max_window_frames,
max_markers, max_pcm_frames, max_correlation_samples_per_marker, max_total_correlation_samples,
max_total_window_frames, min_correlation, min_peak_margin, min_rms, windows:[{start_frame}].
Outputs must be new files. Exit 0=accepted analysis/evaluation, 1=failed marker/unqualified,
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
    if command == "controlled-load-worker" {
        return crate::controlled_load::run_worker(arguments[1..].to_vec());
    }
    if matches!(command.as_str(), "live-process-loopback" | "live-vad") {
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
        "analyze-pcm" => &["--pcm", "--schedule", "--max-input-bytes", "--output"][..],
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
        ensure!(options.contains_key(name), "missing {name}");
    }
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
        let analysis = analyze(&mut file, length, schedule)?;
        let code = i32::from(analysis.failed_markers != 0);
        write_json(output, &analysis)?;
        Ok(code)
    }
}
