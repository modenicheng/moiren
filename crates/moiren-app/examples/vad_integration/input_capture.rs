//! Post-join artifact export only. No file is opened by the PCM worker.
use anyhow::{Context, ensure};
use moiren_windows_audio::vad::VadInputCaptureArtifact;
use serde::Serialize;
use std::{fs::OpenOptions, io::Write, path::Path};

pub(super) const RAW_FILE: &str = "provider-input.pcm";
pub(super) const JSON_FILE: &str = "provider-input.json";

#[derive(Serialize)]
struct Receipt<'a> {
    schema_version: u32,
    qualified: bool,
    direction: &'static str,
    boundary: &'static str,
    data_state: &'static str,
    raw_file: &'static str,
    raw_bytes: usize,
    sample_rate: u32,
    exported_unix_ms: u128,
    capture: &'a VadInputCaptureArtifact,
}

pub(super) fn persist(directory: &Path, artifact: &VadInputCaptureArtifact) -> anyhow::Result<()> {
    ensure!(
        artifact.raw.len() <= 4 * 1024 * 1024 && artifact.packets.len() <= 4096,
        "input capture exceeds diagnostic bounds"
    );
    let receipt = Receipt {
        schema_version: 1,
        qualified: false,
        direction: "playback_source_input",
        boundary: "accepted_provider_read_before_decode",
        data_state: if artifact.raw.is_empty() {
            "no_data_unknown"
        } else {
            "raw_observed_not_qualified"
        },
        raw_file: RAW_FILE,
        raw_bytes: artifact.raw.len(),
        sample_rate: 48_000,
        exported_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
        capture: artifact,
    };
    // Both leaves must be fresh; never overwrite an earlier observation. If a
    // later write fails, leave the partial files and report that failure.
    let mut raw = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(RAW_FILE))
        .context("create input PCM artifact")?;
    let mut json = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(JSON_FILE))
        .context("create input metadata artifact")?;
    raw.write_all(&artifact.raw)?;
    raw.sync_all()?;
    serde_json::to_writer_pretty(&mut json, &receipt)?;
    json.write_all(b"\n")?;
    json.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_windows_audio::vad::{VadInputCaptureIdentity, VadInputCapturePacket};

    #[test]
    fn input_capture_export_is_fresh_unknown_and_not_qualification() {
        let path = std::env::temp_dir().join(format!(
            "moiren-provider-input-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        let artifact = VadInputCaptureArtifact {
            requested_frames: 1,
            stored_frames: 0,
            identity: None,
            seal_reason: "no_accepted_data_unknown",
            excluded_stale_packets: 0,
            excluded_epoch_packets: 0,
            worker_join: "joined",
            worker_outcome: "failed",
            packets: vec![],
            raw: vec![],
        };
        persist(&path, &artifact).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join(JSON_FILE)).unwrap()).unwrap();
        assert_eq!(json["qualified"], false);
        assert_eq!(json["data_state"], "no_data_unknown");
        assert_eq!(json["capture"]["worker_outcome"], "failed");
        assert!(persist(&path, &artifact).is_err());
        std::fs::remove_file(path.join(RAW_FILE)).unwrap();
        std::fs::remove_file(path.join(JSON_FILE)).unwrap();
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn input_capture_export_preserves_raced_publication_and_original_capacity_seal() {
        let path = std::env::temp_dir().join(format!(
            "moiren-provider-input-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        let raw = [0.25_f32, -0.5]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let artifact = VadInputCaptureArtifact {
            requested_frames: 1,
            stored_frames: 1,
            identity: Some(VadInputCaptureIdentity {
                stable_id: [1; 16],
                native_stamp: [11, 7, 11, 3, 5],
                reset_epoch: 0,
                admission_ticket: 2,
                active_revision: 9,
                format: 2,
                channels: 2,
                packet_frames: 1,
            }),
            seal_reason: "frame_capacity",
            excluded_stale_packets: 0,
            excluded_epoch_packets: 0,
            worker_join: "joined",
            worker_outcome: "completed",
            packets: vec![VadInputCapturePacket {
                sequence: 1,
                driver_qpc: 100,
                byte_offset: 0,
                actual_frames: 1,
                byte_count: raw.len(),
                publication: "published_identity_changed",
            }],
            raw,
        };
        persist(&path, &artifact).unwrap();
        let before = std::fs::read(path.join(JSON_FILE)).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["qualified"], false);
        assert_eq!(json["data_state"], "raw_observed_not_qualified");
        assert_eq!(json["capture"]["seal_reason"], "frame_capacity");
        assert_eq!(json["capture"]["identity"]["active_revision"], 9);
        assert_eq!(
            json["capture"]["packets"][0]["publication"],
            "published_identity_changed"
        );
        assert_eq!(std::fs::read(path.join(RAW_FILE)).unwrap(), artifact.raw);
        assert!(persist(&path, &artifact).is_err());
        assert_eq!(std::fs::read(path.join(JSON_FILE)).unwrap(), before);
        std::fs::remove_file(path.join(RAW_FILE)).unwrap();
        std::fs::remove_file(path.join(JSON_FILE)).unwrap();
        std::fs::remove_dir(path).unwrap();
    }
}
