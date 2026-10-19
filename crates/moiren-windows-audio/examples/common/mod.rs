//! Shared `w00_*` example argument parsing.
//!
//! Not an example target itself (no `main.rs` here); each `w00_*.rs` binary
//! includes it with `#[path = "common/mod.rs"] mod common;`.

use anyhow::Context;

/// Takes the value following `flag`, failing with "`{flag}` requires {what}".
pub fn value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
    what: &str,
) -> anyhow::Result<String> {
    args.next()
        .with_context(|| format!("{flag} requires {what}"))
}

/// Parses the value following `flag` as a `u32`, failing with
/// "`{flag}` requires {requires}`" or "`{flag}` expects {expects}`".
pub fn u32(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
    requires: &str,
    expects: &str,
) -> anyhow::Result<u32> {
    value(args, flag, requires)?
        .parse::<u32>()
        .with_context(|| format!("{flag} expects {expects}"))
}
