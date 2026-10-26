//! Bounded control-side diagnostic sampling, not continuous waveform evidence.

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};

pub(super) const MAX_RECORD_BYTES: usize = 32 * 1024;

pub(super) struct Journal {
    file: File,
    buffer: BoundedRecord,
    samples: u64,
    max_samples: u64,
}

struct BoundedRecord(Vec<u8>);

impl Write for BoundedRecord {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_RECORD_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("diagnostic observation exceeds byte cap"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Journal {
    pub(super) fn create(path: &Path, max_samples: u64) -> Result<Self> {
        ensure!(
            (1..=1442).contains(&max_samples),
            "diagnostic sample cap must be 1..1442"
        );
        Ok(Self {
            file: OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| format!("create new diagnostic journal {}", path.display()))?,
            buffer: BoundedRecord(Vec::with_capacity(MAX_RECORD_BYTES)),
            samples: 0,
            max_samples,
        })
    }

    pub(super) fn record(&mut self, sample: &impl Serialize) -> Result<()> {
        ensure!(
            self.samples < self.max_samples,
            "diagnostic sample cap exceeded"
        );
        self.buffer.0.clear();
        // Serialize into bounded control-side scratch first: an oversized
        // observation must not leave a partial JSON record on disk.
        serde_json::to_writer(&mut self.buffer, sample)?;
        self.file.write_all(&self.buffer.0)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.samples += 1;
        Ok(())
    }

    #[cfg(windows)]
    pub(super) fn samples(&self) -> u64 {
        self.samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn journal_is_collision_safe_and_caps_samples_and_record_bytes() {
        let root = std::env::temp_dir().join(format!("moiren-soak-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let path = root.join("observations.jsonl");
        let mut journal = Journal::create(&path, 2).unwrap();
        assert!(Journal::create(&path, 2).is_err());
        assert!(
            journal
                .record(&json!({"payload": "x".repeat(MAX_RECORD_BYTES)}))
                .is_err()
        );
        journal
            .record(&json!({"sequence": 1, "faults": u64::MAX}))
            .unwrap();
        journal
            .record(&json!({"sequence": 2, "faults": 7}))
            .unwrap();
        assert!(journal.record(&json!({"sequence": 3})).is_err());
        drop(journal);
        let contents = fs::read_to_string(&path).unwrap();
        let rows: Vec<serde_json::Value> = contents
            .lines()
            .map(|row| serde_json::from_str(row).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["faults"].as_u64(), Some(u64::MAX));
        assert_eq!(rows[1]["sequence"], 2);
        fs::remove_dir_all(root).unwrap();
    }
}
