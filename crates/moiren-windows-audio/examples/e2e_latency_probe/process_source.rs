//! Owned marker source only; independent collection is a separate caller.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::time::Duration;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const RECEIPT_CAP: u64 = 512 * 1024;
const STOP_GRACE: Duration = Duration::from_secs(5);
const READY_GRACE: Duration = Duration::from_secs(16);
#[cfg(windows)]
const POLL: Duration = Duration::from_millis(10);

#[cfg(windows)]
#[path = "process_source/native.rs"]
mod native;
#[cfg(windows)]
#[expect(
    dead_code,
    reason = "Section B consumes the parent API in a later slice"
)]
pub(crate) type ProcessSourceSession = native::ProcessSourceSession;

#[path = "process_source/session.rs"]
mod session;
#[path = "process_source/supervision.rs"]
mod supervision;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub(crate) pid: u32,
    pub(crate) creation_time_100ns: u64,
    pub(crate) executable_name: String,
}

impl From<&moiren_windows_audio::process_loopback::ProcessIdentity> for Identity {
    fn from(value: &moiren_windows_audio::process_loopback::ProcessIdentity) -> Self {
        Self {
            pid: value.pid,
            creation_time_100ns: value.creation_time_100ns,
            executable_name: value.executable_name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Ready,
    Running,
    Submitted,
    Final,
}

impl Phase {
    fn file(self) -> &'static str {
        match self {
            Self::Ready => "ready.json",
            Self::Running => "running.json",
            Self::Submitted => "submitted.json",
            Self::Final => "final.json",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub(crate) schema_version: u32,
    pub(crate) phase: Phase,
    pub(crate) identity: Identity,
    pub(crate) source_endpoint: String,
    pub(crate) seconds: u64,
    pub(crate) schedule: super::source::SourceSchedule,
    /// Exact serialized native DTO; no source/played claims are fabricated.
    pub(crate) playback: Value,
    pub(crate) stop_requested: bool,
    pub(crate) failure: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Progress {
    pub(crate) prepared: bool,
    pub(crate) running: bool,
    pub(crate) submitted: bool,
    pub(crate) ready: Option<Receipt>,
    pub(crate) running_receipt: Option<Receipt>,
    pub(crate) submitted_receipt: Option<Receipt>,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Outcome {
    pub(crate) source_endpoint: Option<String>,
    pub(crate) seconds: Option<u64>,
    pub(crate) schedule: Option<super::source::SourceSchedule>,
    pub(crate) child_pid: Option<u32>,
    pub(crate) pinned_identity: Option<Identity>,
    pub(crate) progress: Progress,
    pub(crate) final_receipt: Option<Receipt>,
    pub(crate) child_exit_code: Option<i32>,
    pub(crate) forced_termination: bool,
    pub(crate) joined: bool,
    pub(crate) valid: bool,
    pub(crate) failure: Option<String>,
}

impl Progress {
    fn observe(
        &mut self,
        receipt: Receipt,
        phase: Phase,
        activated: bool,
        identity: &Identity,
        config: &Config,
        schedule: &super::source::SourceSchedule,
    ) -> Result<()> {
        let accepted = validate_receipt(&receipt, phase, identity, config, schedule);
        // Preserve even an invalid parsed observation; accepted flags are separate.
        match phase {
            Phase::Ready => self.ready = Some(receipt),
            Phase::Running => self.running_receipt = Some(receipt),
            Phase::Submitted => self.submitted_receipt = Some(receipt),
            Phase::Final => anyhow::bail!("final receipt is not live progress"),
        }
        accepted?;
        match phase {
            Phase::Ready => self.prepared = true,
            Phase::Running => {
                ensure!(
                    activated && self.prepared,
                    "source running before parent activation"
                );
                self.running = true;
            }
            Phase::Submitted => {
                ensure!(self.running, "source submitted before running");
                self.submitted = true;
            }
            Phase::Final => unreachable!(),
        }
        Ok(())
    }
}

fn read_receipt(path: &Path) -> Result<Receipt> {
    read_bounded(path, RECEIPT_CAP)
}

fn read_bounded<T: DeserializeOwned>(path: &Path, cap: u64) -> Result<T> {
    ensure!(cap > 0, "receipt cap must be positive");
    regular_path(path)?;
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= cap,
        "receipt byte cap exceeded"
    );
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= cap, "receipt grew beyond byte cap");
    serde_json::from_slice(&bytes).context("invalid process source receipt")
}

fn regular_path(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "source IPC refuses symbolic links"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "source IPC refuses reparse points"
        );
    }
    Ok(())
}

fn publish(directory: &Path, name: &str, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    publish_bytes(directory, name, &bytes)
}

fn publish_bytes(directory: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    regular_path(directory)?;
    ensure!(
        bytes.len() as u64 <= RECEIPT_CAP,
        "receipt publication exceeds cap"
    );
    let temporary = directory.join(format!(".{name}.new"));
    let final_path = directory.join(name);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    // A new hard link publishes only a completed file and never replaces a
    // receipt. No filesystem operation occurs on the native callback thread.
    let result = fs::hard_link(&temporary, &final_path);
    let cleanup = fs::remove_file(&temporary);
    result?;
    cleanup?;
    Ok(())
}

fn validate_receipt_budget(
    config: &Config,
    schedule: &super::source::SourceSchedule,
) -> Result<()> {
    // Final native evidence repeats the schedule in provenance. Reserve bounded
    // diagnostic/name space before spawning, not after native preparation.
    let envelope = serde_json::json!({
        "schema_version": 1, "phase": "final", "source_endpoint": config.source_endpoint,
        "seconds": config.seconds, "schedule": schedule,
        "identity": {"pid": u32::MAX, "creation_time_100ns": u64::MAX,
            "executable_name": "\0".repeat(260)},
        "playback": {"mmdevice_id": config.source_endpoint,
            "observed_name": "\0".repeat(4096),
            "playback_source": {"schedule": schedule},
            "failure": "\0".repeat(16 * 1024)},
        "failure": "\0".repeat(4096)
    });
    ensure!(
        serde_json::to_vec(&envelope)?.len() as u64 + 8192 <= RECEIPT_CAP,
        "source schedule/endpoint cannot fit bounded final receipt"
    );
    Ok(())
}

fn create_run_directory(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().context("source directory requires parent")?;
    regular_path(parent)?;
    fs::create_dir(path).context("source run directory must be new")?;
    regular_path(path)?;
    Ok(path.canonicalize()?)
}

fn check_lifetime(elapsed: Duration, seconds: u64) -> Result<()> {
    ensure!(
        elapsed < Duration::from_secs(seconds),
        "process source finite lifetime exhausted"
    );
    Ok(())
}

fn token(directory: &Path, name: &str) -> Result<()> {
    // Readers must never observe a create_new file before its byte is written.
    publish_bytes(directory, name, b"1")
}

fn has_token(directory: &Path, name: &str) -> Result<bool> {
    let path = directory.join(name);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
        Ok(metadata) => {
            regular_path(&path)?;
            ensure!(
                metadata.is_file() && metadata.len() == 1,
                "invalid source token"
            );
            let mut file = File::open(path)?;
            let mut bytes = [0; 2];
            ensure!(
                file.read(&mut bytes)? == 1 && bytes[0] == b'1',
                "invalid source token contents"
            );
            Ok(true)
        }
    }
}

fn validate_receipt(
    receipt: &Receipt,
    phase: Phase,
    identity: &Identity,
    config: &Config,
    schedule: &super::source::SourceSchedule,
) -> Result<()> {
    ensure!(
        receipt.schema_version == 1 && receipt.phase == phase,
        "source receipt schema/phase mismatch"
    );
    ensure!(
        receipt.identity == *identity && identity.pid != 0 && identity.creation_time_100ns != 0,
        "source receipt producer identity mismatch"
    );
    ensure!(
        receipt.source_endpoint == config.source_endpoint
            && receipt.seconds == config.seconds
            && receipt.schedule == *schedule,
        "source receipt configuration/schedule mismatch"
    );
    ensure!(
        receipt.failure.is_none(),
        "source receipt failed: {:?}",
        receipt.failure
    );
    ensure!(
        phase == Phase::Final || !receipt.stop_requested,
        "source live receipt claims stop"
    );
    let data = &receipt.playback;
    if phase == Phase::Ready {
        ensure!(
            data["mmdevice_id"].as_str() == Some(config.source_endpoint.as_str())
                && data["source_mode"].as_str() == Some("shared_explicit_mmdevice"),
            "source ready endpoint/mode mismatch"
        );
        ensure!(
            data["stable_endpoint_id"].is_null() && data["adapter_instance"].is_null(),
            "shared source cannot claim VAD identity"
        );
        ensure!(
            data["native_buffer_frames"]
                .as_u64()
                .is_some_and(|frames| (1..=9600).contains(&frames))
                && matches!(data["packet_format"].as_str(), Some("Float32" | "Pcm16")),
            "source ready buffer/format invalid"
        );
        return Ok(());
    }
    ensure!(
        data["started"].as_bool() == Some(true) && data["event_timeouts"].as_u64() == Some(0),
        "source not started or event timeout"
    );
    let progress = if phase == Phase::Final {
        &data["source_progress"]
    } else {
        &data["source"]
    };
    let acknowledged = progress["acknowledged_source_frames"]
        .as_u64()
        .context("source acknowledged frames absent")?;
    let submitted = progress["native_submitted_frames"]
        .as_u64()
        .context("source native submissions absent")?;
    let preroll = progress["native_preroll_frames"]
        .as_u64()
        .context("source preroll absent")?;
    ensure!(
        acknowledged <= schedule.total_source_frames
            && submitted >= preroll
            && submitted - preroll >= acknowledged,
        "source frame accounting invalid"
    );
    if acknowledged > 0 {
        ensure!(
            progress["origin_submission_frame"].as_u64() == Some(preroll),
            "source origin does not follow silent preroll"
        );
    }
    let complete = progress["schedule_submitted_complete"]
        .as_bool()
        .context("source completion observation absent")?;
    ensure!(
        complete == (acknowledged == schedule.total_source_frames),
        "source completion/frame contradiction"
    );
    if phase != Phase::Running {
        ensure!(complete, "source schedule submission incomplete");
    }
    if phase == Phase::Final {
        ensure!(
            receipt.stop_requested
                && data["stopped"].as_bool() == Some(true)
                && data["worker_joined"].as_bool() == Some(true)
                && data["native_stop_attempted"].as_bool() == Some(true)
                && data["native_stop_succeeded"].as_bool() == Some(true)
                && data.get("failure") == Some(&Value::Null),
            "source final Stop/join failed or absent"
        );
    } else {
        ensure!(
            data["stopped"].as_bool() == Some(false),
            "source stopped before receipt"
        );
    }
    Ok(())
}

fn append(failure: &mut Option<String>, message: &str) {
    let text = failure.get_or_insert_with(String::new);
    if text.len() >= 4096 {
        return;
    }
    if !text.is_empty() && text.len() + 2 <= 4096 {
        text.push_str("; ");
    }
    for character in message.chars() {
        if text.len() + character.len_utf8() > 4096 {
            break;
        }
        text.push(character);
    }
}

fn append_option(failure: &mut Option<String>, message: Option<&str>) {
    if let Some(message) = message {
        append(failure, message);
    }
}

fn finalize(outcome: &mut Outcome) {
    if !outcome.pinned_identity.as_ref().is_some_and(|identity| {
        Some(identity.pid) == outcome.child_pid
            && identity.pid != 0
            && identity.creation_time_100ns != 0
    }) {
        append(&mut outcome.failure, "source child identity not pinned");
    }
    if !outcome.joined || outcome.forced_termination || outcome.child_exit_code != Some(0) {
        append(
            &mut outcome.failure,
            "source child did not exit cleanly and join",
        );
    }
    if !(outcome.progress.prepared
        && outcome.progress.running
        && outcome.progress.submitted
        && outcome.progress.ready.is_some()
        && outcome.progress.running_receipt.is_some()
        && outcome.progress.submitted_receipt.is_some()
        && outcome.final_receipt.is_some())
    {
        append(&mut outcome.failure, "source phase evidence incomplete");
    }
    outcome.valid = outcome.failure.is_none();
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) source_endpoint: String,
    pub(crate) source_schedule: PathBuf,
    pub(crate) seconds: u64,
}

impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=600).contains(&self.seconds),
            "source lifetime must be 1..600 seconds"
        );
        ensure!(
            !self.source_endpoint.trim().is_empty()
                && !self.source_endpoint.contains('\0')
                && self.source_endpoint.encode_utf16().count() <= 4096,
            "explicit bounded source endpoint required"
        );
        ensure!(
            !self.source_schedule.as_os_str().is_empty(),
            "explicit source schedule required"
        );
        Ok(())
    }
}

fn parse(arguments: Vec<String>) -> Result<(Config, PathBuf)> {
    ensure!(
        arguments.len() == 8,
        "process source worker requires four separate option pairs"
    );
    let mut values = BTreeMap::new();
    for pair in arguments.as_chunks::<2>().0 {
        ensure!(
            [
                "--source-endpoint",
                "--source-schedule",
                "--run-directory",
                "--seconds"
            ]
            .contains(&pair[0].as_str()),
            "unknown source worker option {}",
            pair[0]
        );
        ensure!(
            values.insert(pair[0].as_str(), pair[1].as_str()).is_none(),
            "duplicate source option"
        );
    }
    let required = |key| {
        values
            .get(key)
            .copied()
            .context("missing process source worker option")
    };
    let raw = required("--seconds")?;
    ensure!(
        !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()),
        "source seconds require decimal digits"
    );
    let config = Config {
        source_endpoint: required("--source-endpoint")?.into(),
        source_schedule: required("--source-schedule")?.into(),
        seconds: raw.parse()?,
    };
    config.validate()?;
    let directory = PathBuf::from(required("--run-directory")?);
    ensure!(
        !directory.as_os_str().is_empty(),
        "new source run directory required"
    );
    Ok((config, directory))
}

pub(crate) fn run_worker(arguments: Vec<String>) -> Result<i32> {
    let (config, directory) = parse(arguments)?;
    #[cfg(windows)]
    {
        native::run(config, directory)
    }
    #[cfg(not(windows))]
    {
        let _ = (config, directory);
        anyhow::bail!("process source worker requires Windows")
    }
}

#[cfg(test)]
#[path = "process_source/tests.rs"]
mod tests;
