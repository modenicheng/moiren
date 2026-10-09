//! Controlled experiment: only the probe's own child session is changed.
//! Captures are statistics-only; no PCM or private device/process IDs persist.
#[cfg(windows)]
#[path = "takeover_probe/mod.rs"]
mod experiment;
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    experiment::main()
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Takeover experiment requires Windows.");
}
