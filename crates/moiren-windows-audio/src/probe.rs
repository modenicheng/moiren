//! Opt-in Process Loopback probe; reports contain statistics and no PCM payload.
use crate::{
    catalog::{self, ApiFailure, CatalogSnapshot, FormatSnapshot},
    owner::{Apartment, Process, ProcessIdentity},
    stats::CaptureSummary,
};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

mod activation;
mod capture;
use capture::{capture, capture_format};

#[derive(Serialize)]
pub struct PacketRecord {
    pub arrival_ms: f64,
    pub frames: u32,
    pub flags: u32,
    pub device_position_frames: u64,
    pub qpc_100ns: u64,
    pub rms: f64,
    pub peak: f64,
}

#[derive(Serialize)]
pub struct CaptureReport {
    pub target: Option<ProcessIdentity>,
    pub mode: &'static str,
    pub status: &'static str,
    pub last_stage: &'static str,
    pub requested_seconds: u32,
    pub elapsed_seconds: f64,
    pub capture_stream_format: FormatSnapshot,
    pub windows_auto_conversion: bool,
    pub buffer_frames: Option<u32>,
    pub mmcss_registered: bool,
    pub stop_succeeded: bool,
    pub summary: CaptureSummary,
    pub metadata_dropped: u64,
    pub packets: Vec<PacketRecord>,
    pub errors: Vec<ApiFailure>,
}

#[derive(Serialize)]
pub struct ProbeReport {
    pub schema_version: u32,
    pub started_unix_ms: u128,
    pub before: CatalogSnapshot,
    pub capture: Option<CaptureReport>,
    pub after: Option<CatalogSnapshot>,
    pub observed_state_changes: Vec<String>,
    pub errors: Vec<ApiFailure>,
}

/// None only enumerates. An explicit PID opts into capture of that process tree.
pub fn run(pid: Option<u32>, seconds: u32) -> anyhow::Result<ProbeReport> {
    if !(1..=600).contains(&seconds) {
        anyhow::bail!("--seconds must be within 1..=600, got {seconds}");
    }
    if pid == Some(0) {
        anyhow::bail!("pid 0 (system idle process) cannot be a capture target");
    }
    let _apartment = Apartment::new()?;
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis());
    let before = catalog::snapshot()?;
    let mut result = ProbeReport {
        schema_version: 1,
        started_unix_ms,
        before,
        capture: None,
        after: None,
        observed_state_changes: Vec::new(),
        errors: Vec::new(),
    };
    if let Some(pid) = pid {
        let mut report = CaptureReport {
            target: None,
            mode: "include_target_process_tree",
            status: "initializing",
            last_stage: "target process identity",
            requested_seconds: seconds,
            elapsed_seconds: 0.0,
            capture_stream_format: FormatSnapshot::from_base(capture_format()),
            windows_auto_conversion: true,
            buffer_frames: None,
            mmcss_registered: false,
            stop_succeeded: false,
            summary: CaptureSummary::default(),
            metadata_dropped: 0,
            packets: Vec::new(),
            errors: Vec::new(),
        };
        match Process::open(pid) {
            Ok(process) => {
                if let Err(error) = capture(&process, &mut report) {
                    report.status = "api_failed";
                    report
                        .errors
                        .push(ApiFailure::new(report.last_stage, error));
                }
                report.target = Some(process.identity);
            }
            Err(error) => {
                report.status = "api_failed";
                report
                    .errors
                    .push(ApiFailure::new("target process identity", error));
            }
        }
        result.capture = Some(report);
        match catalog::snapshot() {
            Ok(after) => {
                result.observed_state_changes = catalog::changes(&result.before, &after, pid);
                result.after = Some(after);
            }
            Err(error) => result
                .errors
                .push(ApiFailure::new("post-capture snapshot", error)),
        }
    }
    Ok(result)
}
