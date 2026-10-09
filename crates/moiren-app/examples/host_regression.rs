//! Opt-in native regression: only explicit devices and owned generator children.
#[cfg(windows)]
#[path = "host_regression/mod.rs"]
mod regression;

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    return regression::run();
    #[cfg(not(windows))]
    anyhow::bail!("host_regression requires Windows and explicit device selections");
}
