//! Opt-in encoded MIC WRITE attempts, never native consumption or recorder mapping.

use super::*;

pub(super) fn reserve(
    options: &LiveOptions,
    report: &Path,
    manifest: &Path,
) -> Result<(Option<String>, Option<String>)> {
    if options.vad_output_capture_frames.is_none() {
        return Ok((None, None));
    }
    let raw = options.artifact_dir.join("vad-output.raw.bin");
    let metadata = options.artifact_dir.join("vad-output.metadata.json");
    let keys = [output_path_key(&raw)?, output_path_key(&metadata)?];
    for path in [
        Some(report),
        Some(manifest),
        options.source_schedule.as_deref(),
        options.measurement_pcm.as_deref(),
        options.measurement_schedule.as_deref(),
        options.calibration.as_deref(),
        options.vad_collection.as_deref(),
        options.process_collection.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        ensure!(
            !keys.contains(&output_path_key(path)?),
            "input/report/manifest aliases reserved VAD output reference"
        );
    }
    Ok((
        Some(raw.to_string_lossy().into_owned()),
        Some(metadata.to_string_lossy().into_owned()),
    ))
}

#[cfg(windows)]
pub(super) struct Recorder {
    handle: moiren_windows_audio::vad::VadOutputCaptureHandle,
    raw_path: String,
    metadata_path: String,
}

#[cfg(windows)]
impl Recorder {
    pub(super) fn new(
        handle: moiren_windows_audio::vad::VadOutputCaptureHandle,
        paths: &ArtifactPaths,
    ) -> Result<Self> {
        Ok(Self {
            handle,
            raw_path: paths
                .vad_output_raw
                .clone()
                .context("VAD output raw path missing")?,
            metadata_path: paths
                .vad_output_metadata
                .clone()
                .context("VAD output metadata path missing")?,
        })
    }

    pub(super) fn finish(&self, evidence: &mut Option<Value>) -> Result<()> {
        finish(
            self.handle.take_after_join(),
            &self.raw_path,
            &self.metadata_path,
            evidence,
        )
    }
}

#[cfg(windows)]
fn finish(
    artifact: std::result::Result<
        moiren_windows_audio::vad::VadOutputCaptureArtifact,
        &'static str,
    >,
    raw_path: &str,
    metadata_path: &str,
    evidence: &mut Option<Value>,
) -> Result<()> {
    // Actual owner join, not cancellation/drained counters, permits export.
    *evidence = Some(serde_json::json!({
        "schema_version": 1, "qualified": false,
        "scope": "actual_encoded_before_write_attempts_and_observed_completions_not_native_consumption_recorder_or_physical_mapping",
        "clock_meaning": "echoed host WRITE-header QPC, not native consumption/sample time or recorder mapping",
        "raw_path": raw_path, "metadata_path": metadata_path,
        "status": "unavailable", "failure": null,
    }));
    let result = (|| -> Result<()> {
        let artifact = artifact
            .map_err(|reason| anyhow::anyhow!("VAD output capture unavailable: {reason}"))?;
        let mut metadata = serde_json::to_value(&artifact)?;
        metadata["schema_version"] = 1.into();
        metadata["qualified"] = false.into();
        metadata["raw_bytes"] = artifact.raw.len().into();
        let persist = (|| -> Result<()> {
            let mut raw = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(raw_path)?;
            raw.write_all(&artifact.raw)?;
            raw.flush()?;
            write_json(metadata_path, &metadata)
        })();
        let report = evidence.as_mut().expect("initialized reference evidence");
        report["metadata"] = metadata;
        persist?;
        report["status"] = if artifact.raw.is_empty() {
            "retained_empty_unknown"
        } else {
            "retained_not_delivery_proof"
        }
        .into();
        Ok(())
    })();
    if let Err(error) = &result {
        evidence.as_mut().expect("initialized reference evidence")["failure"] =
            format!("{error:#}").into();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "moiren-vad-output-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn options(directory: &Path) -> LiveOptions {
        LiveOptions {
            source_kind: EvidenceSourceKind::VadPlayback,
            stable_endpoint_id: Some("0123456789abcdef0123456789abcdef".into()),
            source_name: Some("Playback".into()),
            vad_microphone_id: Some("fedcba9876543210fedcba9876543210".into()),
            vad_microphone_name: Some("Microphone".into()),
            source_schedule: Some(directory.with_extension("source.json")),
            vad_collection: Some(directory.with_extension("collection.json")),
            artifact_dir: directory.into(),
            vad_output_capture_frames: Some(480_000),
            ..Default::default()
        }
    }

    #[test]
    fn vad_output_reference_requires_bounded_nonsoak_microphone_route_before_mkdir() {
        let dir = directory("validation");
        for frames in [0, 480_001, u32::MAX] {
            let mut o = options(&dir);
            o.vad_output_capture_frames = Some(frames);
            assert!(validate(&o).is_err());
        }
        for frames in [1, 480_000] {
            let mut o = options(&dir);
            o.vad_output_capture_frames = Some(frames);
            validate(&o).unwrap();
        }
        for source in [EvidenceSourceKind::ProcessLoopback, EvidenceSourceKind::Vad] {
            let mut o = options(&dir);
            o.source_kind = source;
            assert!(validate(&o).is_err());
        }
        let mut o = options(&dir);
        o.vad_microphone_id = None;
        o.vad_microphone_name = None;
        o.output_endpoint = "unused".into();
        assert!(validate(&o).is_err());
        let mut o = options(&dir);
        o.diagnostic_soak = true;
        assert!(validate(&o).is_err());
        for extra in [
            vec!["--vad-output-capture-frames", "-1"],
            vec!["--vad-output-capture-frames", "480.5"],
            vec!["--vad-output-capture-frames", "true"],
            vec!["--vad-output-capture-frames"],
            vec![
                "--vad-output-capture-frames",
                "1",
                "--vad-output-capture-frames",
                "2",
            ],
        ] {
            assert!(parse(extra.into_iter().map(str::to_owned).collect()).is_err());
        }
        assert!(!dir.exists());
    }

    #[test]
    fn vad_output_reference_aliases_all_inputs_and_report_before_mkdir() {
        for name in [
            "vad-output.raw.bin",
            "./vad-output.metadata.json",
            "child/../vad-output.raw.bin",
        ] {
            for target in 0..9 {
                let dir = directory("alias");
                let mut o = options(&dir);
                let path = Some(dir.join(name));
                match target {
                    0 => o.report_path = path,
                    1 => o.manifest_path = path,
                    2 => o.source_schedule = path,
                    3 => o.measurement_pcm = path,
                    4 => o.measurement_schedule = path,
                    5 => o.calibration = path,
                    6 => o.vad_collection = path,
                    7 => o.process_collection = path,
                    8 => {
                        o.vad_input_capture_frames = Some(480);
                        o.report_path = path;
                    }
                    _ => unreachable!(),
                }
                assert!(prepare_paths(&o).is_err(), "{name} target {target}");
                assert!(!dir.exists());
            }
        }
    }

    #[test]
    fn vad_output_reference_dry_run_flags_are_independent_and_unqualified() {
        for (input, output) in [
            (None, None),
            (Some(1), None),
            (None, Some(480_000)),
            (Some(480), Some(960)),
        ] {
            let dir = directory("dry");
            let mut o = options(&dir);
            o.vad_input_capture_frames = input;
            o.vad_output_capture_frames = output;
            o.dry_run = true;
            let source = br#"{"schema_version":1,"schedule_id":"output-reference-test","waveform_version":"moiren-marker-xorshift32-v1","sample_rate":48000,"channels":2,"marker_frames":32,"marker_amplitude":0.1,"total_source_frames":48000,"starts":[0,40000]}"#;
            fs::write(o.source_schedule.as_ref().unwrap(), source).unwrap();
            let config = br#"{"schema_version":2,"ring_frames":1024,"max_pcm_bytes":1048576,"max_metadata_bytes":100000,"max_packets":1000,"preroll_ms":100,"postroll_ms":200}"#;
            fs::write(o.vad_collection.as_ref().unwrap(), config).unwrap();
            let mut args: Vec<String> = [
                "--source-kind",
                "vad-playback",
                "--stable-endpoint-id",
                o.stable_endpoint_id.as_deref().unwrap(),
                "--name",
                "Playback",
                "--vad-microphone-id",
                o.vad_microphone_id.as_deref().unwrap(),
                "--vad-microphone-name",
                "Microphone",
                "--source-schedule",
                o.source_schedule.as_ref().unwrap().to_str().unwrap(),
                "--vad-collection",
                o.vad_collection.as_ref().unwrap().to_str().unwrap(),
                "--seconds",
                "20",
                "--dry-run",
                "--artifact-dir",
                dir.to_str().unwrap(),
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            for (flag, frames) in [
                ("--vad-input-capture-frames", input),
                ("--vad-output-capture-frames", output),
            ] {
                if let Some(frames) = frames {
                    args.extend([flag.into(), frames.to_string()]);
                }
            }
            assert_eq!(run(args).unwrap(), 1);
            let report: Value =
                serde_json::from_slice(&fs::read(dir.join("report.json")).unwrap()).unwrap();
            let manifest: Value =
                serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
            assert_eq!(report["qualified"], false);
            assert_eq!(
                manifest["vad_input_capture_frames"],
                serde_json::to_value(input).unwrap()
            );
            assert_eq!(
                manifest["vad_output_capture_frames"],
                serde_json::to_value(output).unwrap()
            );
            assert!(report["native"]["vad_output_reference"].is_null());
            assert!(report["native"]["vad_input_reference"].is_null());
            assert_eq!(
                report["artifacts"]["vad_output_reference_status"],
                if output.is_some() {
                    Value::from("not_collected_dry_run")
                } else {
                    Value::Null
                }
            );
            for (key, name, enabled) in [
                ("vad_input_raw", "vad-input.raw.bin", input.is_some()),
                (
                    "vad_input_metadata",
                    "vad-input.metadata.json",
                    input.is_some(),
                ),
                ("vad_output_raw", "vad-output.raw.bin", output.is_some()),
                (
                    "vad_output_metadata",
                    "vad-output.metadata.json",
                    output.is_some(),
                ),
            ] {
                let path = &report["artifacts"]["paths"][key];
                assert_eq!(path, &manifest["artifacts"][key]);
                if enabled {
                    assert_eq!(Path::new(path.as_str().unwrap()).file_name().unwrap(), name);
                } else {
                    assert!(path.is_null());
                }
                assert!(!dir.join(name).exists());
            }
            fs::remove_file(o.source_schedule.unwrap()).unwrap();
            fs::remove_file(o.vad_collection.unwrap()).unwrap();
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[cfg(windows)]
    fn partial_artifact() -> moiren_windows_audio::vad::VadOutputCaptureArtifact {
        use moiren_windows_audio::vad::{
            VadOutputCaptureArtifact, VadOutputCaptureIdentity, VadOutputCapturePacket,
        };
        VadOutputCaptureArtifact {
            requested_frames: 480,
            stored_frames: 1,
            identity: Some(VadOutputCaptureIdentity {
                stable_id: [7; 16],
                native_stamp: [1, 2, 1, 3, 4],
                reset_epoch: 0,
                admission_ticket: 9,
                active_revision: 8,
                format: 1,
                channels: 2,
                packet_frames: 480,
            }),
            seal_reason: "worker_error",
            untracked_completion_observations: 2,
            excluded_epoch_packets: 1,
            worker_join: "joined",
            worker_outcome: "failed",
            packets: vec![VadOutputCapturePacket {
                byte_offset: 0,
                byte_count: 4,
                actual_frames: 1,
                silence_frames: 0,
                submission: "submitted",
                identity_current_after_submit: Some(true),
                completion: "error",
                completion_bytes: None,
                sequence: None,
                host_submission_qpc: None,
                error_kind: Some("windows"),
                raw_hresult: Some(-1),
            }],
            raw: vec![0x00, 0x80, 0xff, 0x7f],
        }
    }

    #[cfg(windows)]
    #[test]
    fn vad_output_reference_unjoined_retains_failure_without_creating_files() {
        let dir = directory("unjoined");
        fs::create_dir(&dir).unwrap();
        let raw = dir.join("raw");
        let metadata = dir.join("metadata");
        let mut evidence = None;
        assert!(
            super::finish(
                Err("not_joined"),
                raw.to_str().unwrap(),
                metadata.to_str().unwrap(),
                &mut evidence
            )
            .is_err()
        );
        assert!(!raw.exists() && !metadata.exists());
        let evidence = evidence.unwrap();
        assert_eq!(evidence["qualified"], false);
        assert_eq!(evidence["status"], "unavailable");
        assert!(evidence["failure"].as_str().unwrap().contains("not_joined"));
        fs::remove_dir(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn vad_output_reference_partial_failure_preserves_raw_statuses_and_nullable_host_qpc() {
        let dir = directory("partial");
        fs::create_dir(&dir).unwrap();
        let raw = dir.join("raw");
        let metadata = dir.join("metadata");
        let mut evidence = None;
        super::finish(
            Ok(partial_artifact()),
            raw.to_str().unwrap(),
            metadata.to_str().unwrap(),
            &mut evidence,
        )
        .unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
        assert_eq!(fs::read(&raw).unwrap(), partial_artifact().raw);
        assert_eq!(saved["identity"]["format"], 1);
        assert_eq!(saved["packets"][0]["submission"], "submitted");
        assert_eq!(saved["packets"][0]["completion"], "error");
        assert!(saved["packets"][0]["host_submission_qpc"].is_null());
        assert_eq!(saved["packets"][0]["raw_hresult"], -1);
        assert_eq!(saved["worker_outcome"], "failed");
        assert_eq!(saved["worker_join"], "joined");
        assert_eq!(saved["excluded_epoch_packets"], 1);
        assert_eq!(saved["raw_bytes"], 4);
        assert_eq!(saved["qualified"], false);
        let evidence = evidence.unwrap();
        assert_eq!(evidence["status"], "retained_not_delivery_proof");
        assert!(
            evidence["scope"]
                .as_str()
                .unwrap()
                .contains("not_native_consumption")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn vad_output_reference_export_failures_never_overwrite_raw_or_metadata() {
        for existing_raw in [false, true] {
            let dir = directory("persist-failure");
            fs::create_dir(&dir).unwrap();
            let raw = dir.join("raw");
            let metadata = dir.join("metadata");
            let blocked = if existing_raw { &raw } else { &metadata };
            fs::write(blocked, b"original evidence").unwrap();
            let mut evidence = None;
            assert!(
                super::finish(
                    Ok(partial_artifact()),
                    raw.to_str().unwrap(),
                    metadata.to_str().unwrap(),
                    &mut evidence
                )
                .is_err()
            );
            assert_eq!(fs::read(blocked).unwrap(), b"original evidence");
            if existing_raw {
                assert!(!metadata.exists());
            } else {
                assert_eq!(fs::read(&raw).unwrap(), partial_artifact().raw);
            }
            let mut native = NativeReport {
                vad_output_reference: evidence,
                ..Default::default()
            };
            native.vad_input_reference =
                Some(serde_json::json!({"status":"retained_not_delivery_proof"}));
            assert_eq!(
                native.vad_output_reference.as_ref().unwrap()["metadata"]["worker_join"],
                "joined"
            );
            assert_eq!(
                native.vad_output_reference.as_ref().unwrap()["status"],
                "unavailable"
            );
            let mut o = options(&dir);
            o.vad_input_capture_frames = Some(480);
            let measurement = MeasurementEvidence {
                analysis: None,
                calibration: None,
                gaps: vec![],
            };
            let report = artifact_report(&ArtifactPaths::default(), &o, &measurement, &native);
            assert_eq!(
                report.capture_pcm_status,
                "raw_vad_input_retained_not_delivery_proof"
            );
            assert_eq!(
                report.vad_output_reference_status,
                Some("raw_vad_output_unavailable_or_export_failed")
            );
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn vad_output_reference_joined_empty_is_unknown_not_delivery() {
        let dir = directory("empty");
        fs::create_dir(&dir).unwrap();
        let mut artifact = partial_artifact();
        artifact.raw.clear();
        artifact.packets.clear();
        artifact.stored_frames = 0;
        artifact.identity = None;
        artifact.seal_reason = "no_accepted_data_unknown";
        let mut evidence = None;
        super::finish(
            Ok(artifact),
            dir.join("raw").to_str().unwrap(),
            dir.join("metadata").to_str().unwrap(),
            &mut evidence,
        )
        .unwrap();
        let native = NativeReport {
            vad_output_reference: evidence,
            ..Default::default()
        };
        let measurement = MeasurementEvidence {
            analysis: None,
            calibration: None,
            gaps: vec![],
        };
        assert_eq!(
            artifact_report(
                &ArtifactPaths::default(),
                &options(&dir),
                &measurement,
                &native
            )
            .vad_output_reference_status,
            Some("raw_vad_output_empty_unknown")
        );
        assert_eq!(fs::metadata(dir.join("raw")).unwrap().len(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}
