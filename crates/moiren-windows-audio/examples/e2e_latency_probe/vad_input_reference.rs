//! Opt-in raw VAD input, not graph/output delivery or a latency reference.

use super::*;

pub(super) fn reserve(
    options: &LiveOptions,
    report: &Path,
    manifest: &Path,
) -> Result<(Option<String>, Option<String>)> {
    if options.vad_input_capture_frames.is_none() {
        return Ok((None, None));
    }
    let raw = options.artifact_dir.join("vad-input.raw.bin");
    let metadata = options.artifact_dir.join("vad-input.metadata.json");
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
            "input/report/manifest aliases reserved VAD input reference"
        );
    }
    Ok((
        Some(raw.to_string_lossy().into_owned()),
        Some(metadata.to_string_lossy().into_owned()),
    ))
}

#[cfg(windows)]
pub(super) struct Recorder {
    handle: moiren_windows_audio::vad::VadInputCaptureHandle,
    raw_path: String,
    metadata_path: String,
}

#[cfg(windows)]
impl Recorder {
    pub(super) fn new(
        handle: moiren_windows_audio::vad::VadInputCaptureHandle,
        paths: &ArtifactPaths,
    ) -> Result<Self> {
        Ok(Self {
            handle,
            raw_path: paths
                .vad_input_raw
                .clone()
                .context("VAD input raw path missing")?,
            metadata_path: paths
                .vad_input_metadata
                .clone()
                .context("VAD input metadata path missing")?,
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
    artifact: std::result::Result<moiren_windows_audio::vad::VadInputCaptureArtifact, &'static str>,
    raw_path: &str,
    metadata_path: &str,
    evidence: &mut Option<Value>,
) -> Result<()> {
    // A completed owner, not cancellation or drained counters, authorizes access.
    // Native failures do not prevent a joined owner's partial evidence export.
    *evidence = Some(serde_json::json!({
        "schema_version": 1, "qualified": false,
        "scope": "actual_raw_playback_input_before_decode_not_graph_or_microphone_delivery",
        "clock_meaning": "driver transfer service QPC, not sample time or recorder mapping",
        "raw_path": raw_path, "metadata_path": metadata_path,
        "status": "unavailable", "failure": null,
    }));
    let result = (|| -> Result<()> {
        let artifact = artifact
            .map_err(|reason| anyhow::anyhow!("VAD input capture unavailable: {reason}"))?;
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
            "moiren-vad-input-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn vad_input_reference_rejects_invalid_modes_budgets_and_duplicate_flags_before_mkdir() {
        for extra in [
            vec!["--vad-input-capture-frames", "0"],
            vec!["--vad-input-capture-frames", "480001"],
            vec!["--vad-input-capture-frames", "-1"],
            vec!["--vad-input-capture-frames", "480.5"],
            vec![
                "--vad-input-capture-frames",
                "1",
                "--vad-input-capture-frames",
                "2",
            ],
            vec!["--vad-input-capture-frames", "1", "--diagnostic-soak"],
        ] {
            let dir = directory("refuse");
            let mut args: Vec<String> = [
                "--source-kind",
                "vad-playback",
                "--stable-endpoint-id",
                "4d6f6972656e53325031353336190001",
                "--name",
                "Reference",
                "--output",
                "never-opened",
                "--artifact-dir",
                dir.to_str().unwrap(),
                "--dry-run",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            args.extend(extra.into_iter().map(str::to_owned));
            assert!(run(args).is_err());
            assert!(!dir.exists());
        }
        let dir = directory("process");
        let options = LiveOptions {
            source_pid: 42,
            output_endpoint: "unused".into(),
            artifact_dir: dir.clone(),
            vad_input_capture_frames: Some(1),
            ..Default::default()
        };
        assert!(validate(&options).is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn vad_input_reference_aliases_refuse_before_artifact_creation_and_omission_stays_off() {
        for name in [
            "vad-input.raw.bin",
            "./vad-input.metadata.json",
            "child/../vad-input.raw.bin",
        ] {
            for manifest in [false, true] {
                let dir = directory("alias");
                let mut options = LiveOptions {
                    source_kind: EvidenceSourceKind::VadPlayback,
                    artifact_dir: dir.clone(),
                    vad_input_capture_frames: Some(480),
                    ..Default::default()
                };
                if manifest {
                    options.manifest_path = Some(dir.join(name));
                } else {
                    options.report_path = Some(dir.join(name));
                }
                assert!(prepare_paths(&options).is_err());
                assert!(!dir.exists());
            }
            let dir = directory("input-alias");
            let options = LiveOptions {
                source_kind: EvidenceSourceKind::VadPlayback,
                artifact_dir: dir.clone(),
                source_schedule: Some(dir.join(name)),
                vad_input_capture_frames: Some(480),
                ..Default::default()
            };
            assert!(prepare_paths(&options).is_err());
            assert!(!dir.exists());
        }
        let dir = directory("off");
        let options = LiveOptions {
            source_kind: EvidenceSourceKind::VadPlayback,
            artifact_dir: dir.clone(),
            ..Default::default()
        };
        let paths = prepare_paths(&options).unwrap();
        assert!(paths.vad_input_raw.is_none() && paths.vad_input_metadata.is_none());
        fs::remove_dir(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn vad_input_reference_unjoined_refuses_without_files_and_preserves_error() {
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
    fn partial_artifact() -> moiren_windows_audio::vad::VadInputCaptureArtifact {
        moiren_windows_audio::vad::VadInputCaptureArtifact {
            requested_frames: 480,
            stored_frames: 1,
            identity: Some(moiren_windows_audio::vad::VadInputCaptureIdentity {
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
            excluded_stale_packets: 2,
            excluded_epoch_packets: 1,
            worker_join: "joined",
            worker_outcome: "failed",
            packets: vec![moiren_windows_audio::vad::VadInputCapturePacket {
                sequence: 15,
                driver_qpc: 1234,
                byte_offset: 0,
                actual_frames: 1,
                byte_count: 4,
                publication: "decode_error",
            }],
            raw: vec![0x00, 0x80, 0xff, 0x7f],
        }
    }

    #[cfg(windows)]
    #[test]
    fn vad_input_reference_preserves_raw_format_failed_partial_metadata_and_refuses_overwrite() {
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
        let before = fs::read(&metadata).unwrap();
        let saved: Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(fs::read(&raw).unwrap(), partial_artifact().raw);
        assert_eq!(saved["identity"]["format"], 1);
        assert_eq!(saved["packets"][0]["publication"], "decode_error");
        assert_eq!(saved["worker_outcome"], "failed");
        assert_eq!(saved["worker_join"], "joined");
        assert_eq!(saved["excluded_epoch_packets"], 1);
        assert_eq!(saved["raw_bytes"], 4);
        assert_eq!(saved["qualified"], false);
        assert!(
            super::finish(
                Ok(partial_artifact()),
                raw.to_str().unwrap(),
                metadata.to_str().unwrap(),
                &mut evidence
            )
            .is_err()
        );
        assert_eq!(fs::read(&metadata).unwrap(), before);
        assert_eq!(fs::read(&raw).unwrap(), partial_artifact().raw);
        assert_eq!(evidence.unwrap()["status"], "unavailable");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn vad_input_reference_metadata_write_failure_retains_raw_and_reports_unavailable() {
        let dir = directory("metadata-failure");
        fs::create_dir(&dir).unwrap();
        let raw = dir.join("raw");
        let metadata = dir.join("metadata");
        fs::write(&metadata, b"preexisting evidence").unwrap();
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
        assert_eq!(fs::read(&raw).unwrap(), partial_artifact().raw);
        assert_eq!(fs::read(&metadata).unwrap(), b"preexisting evidence");
        let native = NativeReport {
            vad_input_reference: evidence,
            ..Default::default()
        };
        assert_eq!(
            native.vad_input_reference.as_ref().unwrap()["status"],
            "unavailable"
        );
        assert_eq!(
            native.vad_input_reference.as_ref().unwrap()["metadata"]["worker_join"],
            "joined"
        );
        let options = LiveOptions {
            vad_input_capture_frames: Some(480),
            ..Default::default()
        };
        let measurement = MeasurementEvidence {
            analysis: None,
            calibration: None,
            gaps: vec![],
        };
        assert_eq!(
            artifact_report(&ArtifactPaths::default(), &options, &measurement, &native)
                .capture_pcm_status,
            "raw_vad_input_unavailable_or_export_failed"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn vad_input_reference_joined_empty_remains_unknown_not_waveform_data() {
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
            vad_input_reference: evidence,
            ..Default::default()
        };
        let options = LiveOptions {
            vad_input_capture_frames: Some(480),
            ..Default::default()
        };
        let measurement = MeasurementEvidence {
            analysis: None,
            calibration: None,
            gaps: vec![],
        };
        assert_eq!(
            artifact_report(&ArtifactPaths::default(), &options, &measurement, &native)
                .capture_pcm_status,
            "raw_vad_input_empty_unknown"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn vad_input_reference_dry_run_reserves_explicit_raw_paths_without_claiming_data() {
        let directory = std::env::temp_dir().join(format!(
            "moiren-vad-input-reference-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let args = [
            "--source-kind",
            "vad-playback",
            "--stable-endpoint-id",
            "4d6f6972656e53325031353336190001",
            "--name",
            "InputReference",
            "--output",
            "never-opened",
            "--artifact-dir",
            directory.to_str().unwrap(),
            "--vad-input-capture-frames",
            "48000",
            "--dry-run",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(run(args).unwrap(), 1);
        let report: Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(report["qualified"], false);
        assert_eq!(
            report["artifacts"]["capture_pcm_status"],
            "not_collected_dry_run"
        );
        assert_eq!(manifest["vad_input_capture_frames"], 48000);
        assert!(report["native"]["vad_input_reference"].is_null());
        for (key, name) in [
            ("vad_input_raw", "vad-input.raw.bin"),
            ("vad_input_metadata", "vad-input.metadata.json"),
        ] {
            assert_eq!(
                Path::new(report["artifacts"]["paths"][key].as_str().unwrap())
                    .file_name()
                    .unwrap(),
                name
            );
            assert_eq!(
                report["artifacts"]["paths"][key],
                manifest["artifacts"][key]
            );
            assert!(!directory.join(name).exists());
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
