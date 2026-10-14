//! Opt-in, muted WASAPI stress probe. Does not alter system device selection.
#[cfg(windows)]
#[path = "managed_stress/native.rs"]
mod native;
#[path = "managed_stress/options.rs"]
mod options;
#[cfg(any(windows, test))]
#[path = "managed_stress/project.rs"]
mod project;

fn main() -> anyhow::Result<()> {
    let options = options::Options::parse(std::env::args().skip(1))?;
    #[cfg(windows)]
    return native::run(options);
    #[cfg(not(windows))]
    {
        let _ = options;
        anyhow::bail!("managed_stress requires a live Windows WASAPI default output")
    }
}
