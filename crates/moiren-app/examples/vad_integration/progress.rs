//! One durable replaceable progress snapshot; final qualification lives in report.json.
use super::report::Report;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

#[derive(Serialize)]
struct Progress<'a> {
    schema_version: u32,
    status: &'static str,
    final_qualification: bool,
    sequence: u64,
    emitted_unix_ms: u128,
    elapsed_ms: u128,
    sampled_report_age_ms: u128,
    report: &'a Report,
}

pub(super) fn persist(
    directory: &Path,
    report: &Report,
    sequence: u64,
    elapsed_ms: u128,
    emitted_unix_ms: u128,
    finished: bool,
) -> anyhow::Result<()> {
    let evidence = Progress {
        schema_version: 1,
        status: if finished {
            "finished_see_report_json"
        } else {
            "running"
        },
        final_qualification: false,
        sequence,
        emitted_unix_ms,
        elapsed_ms,
        sampled_report_age_ms: elapsed_ms.saturating_sub(report.elapsed_ms),
        report,
    };
    let bytes = serde_json::to_vec_pretty(&evidence)?;
    let pending = directory.join("progress.pending.json");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&pending)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    // Same-directory rename replaces the previous snapshot, after the new file
    // is flushed. An interrupted write leaves the previous progress.json intact.
    fs::rename(pending, directory.join("progress.json"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_progress_replaces_in_place_and_cannot_claim_final_qualification() {
        let directory = std::env::temp_dir().join(format!(
            "moiren-soak-progress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let report = Report {
            elapsed_ms: 10,
            ..Default::default()
        };
        persist(&directory, &report, 1, 20, 30, false).unwrap();
        persist(&directory, &report, 2, 40, 50, false).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("progress.json")).unwrap()).unwrap();
        assert_eq!(json["sequence"], 2);
        assert_eq!(json["status"], "running");
        assert_eq!(json["final_qualification"], false);
        assert_eq!(json["sampled_report_age_ms"], 30);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_file(directory.join("progress.json")).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
