//! Explicit, opt-in process admission before the native CLI is entered.
#[cfg(any(windows, test))]
use anyhow::{Context, ensure};
use anyhow::{Result, bail};
#[cfg(any(windows, test))]
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[cfg(any(windows, test))]
const CAP: u64 = 8192;
#[cfg(any(windows, test))]
const MAX_TICKS: u64 = 3_155_378_975_999_999_999;

#[cfg(windows)]
#[path = "managed_removal/windows.rs"]
mod native;

#[derive(Debug)]
#[cfg(any(windows, test))]
struct Config {
    directory: PathBuf,
    nonce: String,
    controller_pid: u32,
    controller_ticks: u64,
    worker_pid: u32,
    worker_ticks: u64,
    timeout: Duration,
    arguments: Vec<String>,
}

#[cfg(any(windows, test))]
impl Config {
    fn parse(arguments: &[String]) -> Result<Self> {
        let split = arguments
            .iter()
            .position(|s| s == "--")
            .context("missing managed separator")?;
        ensure!(
            split == 15,
            "managed admission requires seven exact options"
        );
        let mut options = BTreeMap::new();
        for pair in arguments[1..split].as_chunks::<2>().0 {
            ensure!(
                options.insert(pair[0].as_str(), pair[1].as_str()).is_none(),
                "duplicate managed option"
            );
        }
        let mut take = |key| {
            options
                .remove(key)
                .with_context(|| format!("missing managed option {key}"))
        };
        let directory = PathBuf::from(take("--run-dir")?);
        let nonce = take("--nonce")?.to_owned();
        let controller_pid = take("--controller-pid")?.parse()?;
        let controller_ticks = take("--controller-creation-ticks")?.parse()?;
        let worker_pid = take("--worker-pid")?.parse()?;
        let worker_ticks = take("--worker-creation-ticks")?.parse()?;
        let milliseconds = take("--admission-timeout-ms")?.parse::<u64>()?;
        ensure!(options.is_empty(), "unknown managed option");
        ensure!(
            nonce.len() == 32
                && nonce
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "invalid managed nonce"
        );
        ensure!(
            controller_pid > 0 && worker_pid > 0 && controller_pid != worker_pid,
            "distinct managed owner PIDs required"
        );
        ensure!(
            (1..=MAX_TICKS).contains(&controller_ticks) && (1..=MAX_TICKS).contains(&worker_ticks),
            "invalid managed creation ticks"
        );
        ensure!(
            (1..=15000).contains(&milliseconds),
            "managed admission timeout must be 1..15000 ms"
        );
        let arguments = arguments[split + 1..].to_vec();
        ensure!(
            arguments.first().is_some_and(|s| s == "live-vad"),
            "managed admission permits only live-vad"
        );
        ensure!(
            directory.is_absolute()
                && !directory
                    .components()
                    .any(|c| matches!(c, Component::ParentDir)),
            "absolute managed run directory required"
        );
        Ok(Self {
            directory,
            nonce,
            controller_pid,
            controller_ticks,
            worker_pid,
            worker_ticks,
            timeout: Duration::from_millis(milliseconds),
            arguments,
        })
    }
}

/// Exact numeric DateTime UTC ticks, not Unix milliseconds or floating point.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[cfg(any(windows, test))]
struct Receipt {
    schema_version: u32,
    nonce: String,
    probe_pid: u32,
    probe_creation_utc_ticks: u64,
    controller_pid: u32,
    controller_creation_utc_ticks: u64,
    worker_pid: u32,
    worker_creation_utc_ticks: u64,
    session_id: u32,
}

#[cfg(any(windows, test))]
trait Boundary {
    fn receipt(&self, config: &Config) -> Receipt;
    fn owners_alive(&self) -> Result<()>;
    fn elapsed(&self) -> Duration;
    fn poll(&mut self);
}

#[cfg(any(windows, test))]
fn regular_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "managed path is a symlink"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "managed path is a reparse point"
            );
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
struct Exchange<'a> {
    directory: &'a Path,
}

#[cfg(any(windows, test))]
impl<'a> Exchange<'a> {
    fn open(directory: &'a Path) -> Result<Self> {
        regular_path(directory)?;
        ensure!(
            directory.is_dir(),
            "managed run path must be an existing directory"
        );
        for name in [
            "probe-ready.json",
            "probe-ready.json.partial",
            "probe-admit.json",
            "probe-admit.json.partial",
        ] {
            match fs::symlink_metadata(directory.join(name)) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
                Ok(_) => bail!("managed receipt already exists: {name}"),
            }
        }
        Ok(Self { directory })
    }

    fn ready(&self, receipt: &Receipt) -> Result<()> {
        regular_path(self.directory)?;
        let bytes = serde_json::to_vec(receipt)?;
        ensure!(
            bytes.len() as u64 <= CAP,
            "managed ready receipt exceeds cap"
        );
        let temporary = self.directory.join("probe-ready.json.partial");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Publish complete bytes without an overwrite on either Windows or Unix.
        // The directory/receipt is correlation, not an authorization boundary
        // against another process already able to write this same-user directory.
        fs::hard_link(&temporary, self.directory.join("probe-ready.json"))?;
        fs::remove_file(temporary)?;
        Ok(())
    }

    fn admitted(&self) -> Result<Option<Receipt>> {
        let path = self.directory.join("probe-admit.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(metadata) => ensure!(metadata.is_file(), "managed admit must be a regular file"),
        }
        regular_path(&path)?;
        let file = File::open(&path)?;
        ensure!(file.metadata()?.len() <= CAP, "managed admit exceeds cap");
        let mut bytes = Vec::new();
        file.take(CAP + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= CAP, "managed admit grew beyond cap");
        Ok(Some(
            serde_json::from_slice(&bytes).context("invalid managed admit receipt")?,
        ))
    }
}

#[cfg(any(windows, test))]
fn admitted_run(
    config: Config,
    boundary: &mut impl Boundary,
    invoke: impl FnOnce(Vec<String>) -> Result<i32>,
) -> Result<i32> {
    let exchange = Exchange::open(&config.directory)?;
    let receipt = boundary.receipt(&config);
    boundary.owners_alive()?;
    ensure!(
        boundary.elapsed() < config.timeout,
        "managed admission deadline exhausted"
    );
    exchange.ready(&receipt)?;
    loop {
        // Recheck owners and time after file IO as well: a late ack is never a
        // success, and a dead/reused PID cannot replace the retained owner.
        boundary.owners_alive()?;
        ensure!(
            boundary.elapsed() < config.timeout,
            "managed admission timed out"
        );
        if let Some(admit) = exchange.admitted()? {
            ensure!(
                admit == receipt,
                "managed admit identity differs from ready"
            );
            boundary.owners_alive()?;
            ensure!(
                boundary.elapsed() < config.timeout,
                "managed admit arrived after deadline"
            );
            return invoke(config.arguments);
        }
        boundary.poll();
    }
}

pub(super) fn run(
    arguments: Vec<String>,
    invoke: impl FnOnce(Vec<String>) -> Result<i32>,
) -> Result<i32> {
    if arguments.first().is_none_or(|s| s != "managed-removal") {
        return invoke(arguments);
    }
    #[cfg(windows)]
    {
        let config = Config::parse(&arguments)?;
        let mut boundary = native::Processes::open(&config)?;
        admitted_run(config, &mut boundary, invoke)
    }
    #[cfg(not(windows))]
    {
        bail!("managed removal admission requires Windows process handles")
    }
}

#[cfg(test)]
#[path = "managed_removal/tests.rs"]
mod tests;
