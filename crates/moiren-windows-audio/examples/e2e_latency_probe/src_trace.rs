//! Control-side bounded export. Imported JSON cannot issue native provenance.
use anyhow::{Context, Result};
#[cfg(windows)]
use moiren_windows_audio::engine_backend::NativeSrcTraceCollector;
use std::{fs::OpenOptions, io::Write, path::Path};

const MAX_ARCHIVE_BYTES: usize = 192 * 1024 * 1024;

#[cfg(windows)]
pub(super) fn export(
    collector: &NativeSrcTraceCollector,
    path: &Path,
    report: &mut Option<serde_json::Value>,
) -> Result<()> {
    let trace = collector.artifact();
    let result = persist(path, trace);
    *report = Some(serde_json::json!({
        "schema_version": 1,
        "path": path, "exported": result.is_ok(),
        "owner_generation": trace.owner_generation,
        "history_complete": trace.history_complete,
        "retained_checkpoints": trace.retained_checkpoints,
        "checkpoint_limit": trace.checkpoint_limit,
        "refusal": trace.refusal, "qualified": false,
        "failure": result.as_ref().err().map(|error| format!("{error:#}")),
        "scope": "actual callback scalar history; no source/recorder reference, clean waveform or physical delay claim",
    }));
    result
}

struct CappedArchive {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for CappedArchive {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(std::io::Error::other("SRC trace archive byte cap exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn persist(path: &Path, artifact: &impl serde::Serialize) -> Result<()> {
    // Charge JSON escaping/newline before opening; preserve any partial file
    // on IO failure and never overwrite an earlier run's evidence.
    let mut bytes = CappedArchive {
        bytes: Vec::new(),
        limit: MAX_ARCHIVE_BYTES,
    };
    serde_json::to_writer(&mut bytes, artifact).context("serialize bounded SRC trace")?;
    bytes.write_all(b"\n")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("create new SRC trace")?;
    file.write_all(&bytes.bytes).context("write SRC trace")?;
    file.flush().context("flush SRC trace")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn src_trace_export_is_bounded_create_new_and_preserves_first_evidence() {
        let directory = std::env::temp_dir().join(format!(
            "moiren-src-trace-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("src-trace.json");
        persist(
            &path,
            &serde_json::json!({"qualified":false,"history_complete":false}),
        )
        .unwrap();
        let first = std::fs::read(&path).unwrap();
        assert!(persist(&path, &serde_json::json!({"qualified":true})).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), first);
        let mut bounded = CappedArchive {
            bytes: Vec::new(),
            limit: 8,
        };
        assert!(serde_json::to_writer(&mut bounded, &"123456789").is_err());
        assert!(bounded.bytes.len() <= 8);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
