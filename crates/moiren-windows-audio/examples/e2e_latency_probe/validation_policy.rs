//! Test intent is independent of physical qualification and native buffer sizes.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ValidationMode {
    #[default]
    LatencySensitive,
    Correctness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ValidationPolicy {
    pub mode: ValidationMode,
    pub jitter_tolerance_ms: u64,
}

impl Default for ValidationPolicy {
    fn default() -> Self {
        Self {
            mode: ValidationMode::LatencySensitive,
            jitter_tolerance_ms: 3000,
        }
    }
}

impl ValidationPolicy {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.jitter_tolerance_ms <= 3000,
            "jitter tolerance must be 0..3000 ms"
        );
        Ok(())
    }

    pub fn is_correctness(self) -> bool {
        self.mode == ValidationMode::Correctness
    }

    pub fn progress_grace(self) -> Duration {
        if self.is_correctness() {
            Duration::from_millis(self.jitter_tolerance_ms)
        } else {
            Duration::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{parse, validate};
    use super::{ValidationMode, ValidationPolicy};
    use std::time::Duration;

    fn arguments() -> Vec<String> {
        [
            "--source-kind",
            "process-loopback",
            "--source-output",
            "source",
            "--source-schedule",
            "schedule.json",
            "--process-collection",
            "collection.json",
            "--output",
            "output",
            "--artifact-dir",
            "not-opened",
            "--seconds",
            "20",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn correctness_cli_accepts_explicit_three_second_jitter_without_opening_owners() {
        let mut args = arguments();
        args.extend(
            [
                "--validation-mode",
                "correctness",
                "--jitter-tolerance-ms",
                "3000",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        let options = parse(args).expect("correctness mode must be selectable");
        validate(&options).unwrap();
        assert_eq!(options.validation.mode, ValidationMode::Correctness);
        assert_eq!(options.validation.progress_grace(), Duration::from_secs(3));
    }

    #[test]
    fn both_live_subcommands_show_help_without_source_or_endpoint_options() {
        for command in ["live-process-loopback", "live-vad"] {
            assert_eq!(
                crate::cli::run(vec![command.into(), "--help".into()]).unwrap(),
                0
            );
        }
    }

    #[test]
    fn latency_sensitive_remains_default_and_has_no_jitter_grace() {
        let options = parse(arguments()).unwrap();
        validate(&options).unwrap();
        assert_eq!(options.validation.mode, ValidationMode::LatencySensitive);
        assert_eq!(options.validation.progress_grace(), Duration::ZERO);
        assert_eq!(
            ValidationPolicy {
                mode: ValidationMode::LatencySensitive,
                jitter_tolerance_ms: 3000,
            }
            .progress_grace(),
            Duration::ZERO
        );
    }

    #[test]
    fn correctness_tolerance_round_trips_and_can_be_zero() {
        for jitter_tolerance_ms in [0, 1, 2999, 3000] {
            let policy = ValidationPolicy {
                mode: ValidationMode::Correctness,
                jitter_tolerance_ms,
            };
            policy.validate().unwrap();
            let json = serde_json::to_value(policy).unwrap();
            assert_eq!(json["mode"], "correctness");
            assert_eq!(
                serde_json::from_value::<ValidationPolicy>(json).unwrap(),
                policy
            );
        }
    }

    #[test]
    fn correctness_requires_a_supervised_collector() {
        let options = super::super::LiveOptions {
            validation: ValidationPolicy {
                mode: ValidationMode::Correctness,
                ..Default::default()
            },
            source_pid: 42,
            output_endpoint: "output".into(),
            artifact_dir: "not-opened".into(),
            ..Default::default()
        };
        assert!(
            validate(&options)
                .unwrap_err()
                .to_string()
                .contains("requires a supervised")
        );
    }

    #[test]
    fn correctness_cannot_build_latency_qualification_even_without_collection_flags() {
        let options = super::super::LiveOptions {
            validation: ValidationPolicy {
                mode: ValidationMode::Correctness,
                ..Default::default()
            },
            ..Default::default()
        };
        let measurement = super::super::MeasurementEvidence {
            analysis: None,
            calibration: None,
            gaps: Vec::new(),
        };
        assert!(
            super::super::build_qualification_report(
                &options,
                "correctness-diagnostic",
                None,
                3.0,
                &super::super::NativeReport::default(),
                &measurement
            )
            .is_none()
        );
    }

    #[test]
    fn completed_correctness_can_succeed_with_quality_faults_but_never_with_hard_failures() {
        use super::super::{LiveOptions, NativeReport, collection_exit_code};
        use serde_json::{Value, json};
        let options = LiveOptions {
            validation: ValidationPolicy {
                mode: ValidationMode::Correctness,
                ..Default::default()
            },
            ..Default::default()
        };
        let native = NativeReport::default();
        let completed = json!({
            "collection_complete": true,
            "cleanup_complete": true,
            "failure": null,
            "quality_passed": false,
            "qualified": false
        });
        assert_eq!(collection_exit_code(&options, &native, Some(&completed)), 0);
        for (key, value) in [
            ("collection_complete", Value::Bool(false)),
            ("cleanup_complete", Value::Bool(false)),
            ("failure", json!("disk write failed")),
        ] {
            let mut incomplete = completed.clone();
            incomplete[key] = value;
            assert_eq!(
                collection_exit_code(&options, &native, Some(&incomplete)),
                1
            );
        }
        let missing = json!({"collection_complete":true, "cleanup_complete":true});
        assert_eq!(collection_exit_code(&options, &native, Some(&missing)), 1);
        assert_eq!(collection_exit_code(&options, &native, None), 1);
        let failed_native = NativeReport {
            error: Some("owner failed".into()),
            ..Default::default()
        };
        assert_eq!(
            collection_exit_code(&options, &failed_native, Some(&completed)),
            1
        );
        let dry = LiveOptions {
            dry_run: true,
            ..options.clone()
        };
        assert_eq!(collection_exit_code(&dry, &native, Some(&completed)), 1);
        let strict = LiveOptions {
            validation: ValidationPolicy::default(),
            ..options
        };
        assert_eq!(collection_exit_code(&strict, &native, Some(&completed)), 1);
    }

    #[test]
    fn validation_cli_refuses_unsafe_unbounded_or_duplicate_policy_options() {
        for tail in [
            vec!["--validation-mode", "skip-errors"],
            vec![
                "--validation-mode",
                "correctness",
                "--jitter-tolerance-ms",
                "3001",
            ],
            vec![
                "--validation-mode",
                "correctness",
                "--validation-mode",
                "latency-sensitive",
            ],
            vec!["--jitter-tolerance-ms", "1", "--jitter-tolerance-ms", "2"],
        ] {
            let mut args = arguments();
            args.extend(tail.into_iter().map(str::to_owned));
            assert!(parse(args).and_then(|options| validate(&options)).is_err());
        }
    }

    #[test]
    fn correctness_dry_run_persists_both_policy_modes_without_native_or_qualification_claim() {
        use std::fs;
        for (label, vad) in [("process", false), ("vad", true)] {
            let directory = std::env::temp_dir().join(format!(
                "moiren-correctness-dry-{label}-{}",
                std::process::id()
            ));
            fs::create_dir(&directory).unwrap();
            let output = directory.join("artifacts");
            let source = directory.join("source.json");
            let config = directory.join("config.json");
            fs::write(
                &source,
                br#"{
                "schema_version":1,"schedule_id":"correctness-dry",
                "waveform_version":"moiren-marker-xorshift32-v1",
                "sample_rate":48000,"channels":2,"marker_frames":32,
                "marker_amplitude":0.1,"total_source_frames":48000,"starts":[0]
            }"#,
            )
            .unwrap();
            fs::write(
                &config,
                br#"{
                "schema_version":1,"input_endpoint_id":"recorder",
                "ring_frames":1024,"max_pcm_bytes":1048576,
                "max_metadata_bytes":100000,"max_packets":1000,
                "preroll_ms":100,"postroll_ms":100
            }"#,
            )
            .unwrap();
            let mut args: Vec<String> = [
                "--dry-run",
                "--validation-mode",
                "correctness",
                "--jitter-tolerance-ms",
                "3000",
                "--seconds",
                "20",
                "--output",
                "graph-output",
                "--artifact-dir",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            args.push(output.to_string_lossy().into_owned());
            args.extend([
                "--source-schedule".into(),
                source.to_string_lossy().into_owned(),
            ]);
            args.extend([
                if vad {
                    "--vad-collection"
                } else {
                    "--process-collection"
                }
                .into(),
                config.to_string_lossy().into_owned(),
            ]);
            args.extend(
                if vad {
                    vec![
                        "--source-kind",
                        "vad-playback",
                        "--stable-endpoint-id",
                        "07070707070707070707070707070707",
                        "--name",
                        "Correctness test",
                    ]
                } else {
                    vec!["--source-output", "source-output"]
                }
                .into_iter()
                .map(str::to_owned),
            );
            assert_eq!(super::super::run(args).unwrap(), 1);
            for filename in ["report.json", "manifest.json"] {
                let value: serde_json::Value =
                    serde_json::from_slice(&fs::read(output.join(filename)).unwrap()).unwrap();
                assert_eq!(value["validation"]["mode"], "correctness");
                assert_eq!(value["validation"]["jitter_tolerance_ms"], 3000);
                assert_eq!(value["status"], "unqualified");
                assert_eq!(value["collection"]["collection_complete"], false);
                assert_eq!(value["collection"]["cleanup_complete"], false);
                if filename == "report.json" {
                    assert_eq!(value["qualified"], false);
                    assert!(value["qualification"].is_null());
                }
            }
            assert!(!output.join("measurement.f32le").exists());
            assert!(!output.join("process-source").exists());
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
