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
    let published = directory.join("progress.json");
    #[cfg(windows)]
    for _ in 0..20 {
        match fs::rename(&pending, &published) {
            // Readers without FILE_SHARE_DELETE can temporarily block replacement.
            // Retry only that operation and keep the last complete snapshot intact.
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            result => return result.map_err(Into::into),
        }
    }
    fs::rename(pending, published)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress_directory() -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "moiren-soak-progress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn durable_progress_replaces_in_place_and_cannot_claim_final_qualification() {
        let directory = progress_directory();
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

    #[cfg(windows)]
    #[test]
    fn progress_replaces_snapshot_after_short_lived_windows_reader() {
        use std::{os::windows::fs::OpenOptionsExt, thread, time::Duration};

        let directory = progress_directory();
        let report = Report::default();
        let published = directory.join("progress.json");
        let pending = directory.join("progress.pending.json");
        persist(&directory, &report, 1, 20, 30, false).unwrap();
        // A reader without FILE_SHARE_DELETE temporarily prevents replacement.
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(0x03)
            .open(&published)
            .unwrap();
        let result = thread::scope(|scope| {
            scope.spawn(move || {
                thread::sleep(Duration::from_millis(100));
                drop(reader);
            });
            persist(&directory, &report, 2, 40, 50, false)
        });
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(&published).unwrap()).unwrap();
        fs::remove_file(published).unwrap();
        if pending.exists() {
            fs::remove_file(pending).unwrap();
        }
        fs::remove_dir(directory).unwrap();
        result.unwrap();
        assert_eq!(json["sequence"], 2);
    }
}
