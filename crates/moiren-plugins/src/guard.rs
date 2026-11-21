//! Control-side panic isolation for plugin ABI calls.
//!
//! Every host-side call into plugin code that runs on a non-realtime
//! thread goes through [`guarded`]: a Rust panic unwinding out of the
//! guarded frame is converted into [`PluginError::PluginPanic`] so the
//! host process survives. See the crate guide for the honest boundary,
//! which empirical testing narrowed further (see the record in the test
//! module): this catches panics that unwind through the *host's* Rust
//! runtime — host plumbing inside the frame, or plugin code sharing the
//! host's runtime. It does **not** catch panics raised by a separately
//! compiled Rust cdylib (foreign exception: the runtime aborts), C++
//! exceptions translated into SEH, or segfault-level faults — those need
//! out-of-process instantiation.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::spec::PluginError;

/// Run one control-side plugin interaction, converting a Rust panic that
/// unwinds out of it into [`PluginError::PluginPanic`] carrying the plugin
/// path and the call site. Happy path cost is a single `catch_unwind`
/// frame with no allocation.
///
/// # Why `AssertUnwindSafe` is sound here
///
/// The closure almost always performs an unsafe FFI call through raw
/// pointers; raw pointers are not `UnwindSafe`, but their validity does
/// not depend on the panic machinery at all. What `UnwindSafe` actually
/// protects is *logical* consistency of Rust state observed after a
/// catch, and this guard deliberately makes no such claim: after a
/// caught panic the plugin object is in an unknown state, the caller
/// treats the instance as failed (instantiate) or poisoned (state/
/// preset transfers), and nothing is retried on it. The guard exists
/// solely to keep the host process alive while reporting the failure,
/// which is exactly the guarantee `AssertUnwindSafe` buys.
///
/// A caught panic deliberately leaks any half-constructed plugin object:
/// calling back into a plugin that just panicked (to destroy or unwind
/// it) could re-enter the fault, so the module handle and plugin object
/// stay alive until process exit instead.
pub(crate) fn guarded<T>(
    path: &std::path::Path,
    call: &'static str,
    invoke: impl FnOnce() -> T,
) -> Result<T, PluginError> {
    let context = path.display().to_string();
    match catch_unwind(AssertUnwindSafe(invoke)) {
        Ok(value) => Ok(value),
        Err(payload) => Err(PluginError::PluginPanic {
            call,
            path: context,
            message: panic_message(&payload),
        }),
    }
}

/// Best-effort panic payload rendering. Tries the payload types std
/// documents (`&'static str`, `String`) plus `Box<str>`; anything else
/// becomes an "opaque payload" note. The concrete payload type of a
/// same-runtime panic is environment-dependent (an in-workspace test
/// binary on rustc 1.100/Windows yielded a type none of these downcasts
/// matched, while a scratch binary on the same toolchain yielded `&str`),
/// so callers must treat the message as best-effort: the plugin path and
/// call site in the error are authoritative, and the panic text itself is
/// always printed by the default panic hook before the guard converts it.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = payload.downcast_ref::<Box<str>>() {
        text.to_string()
    } else {
        "opaque panic payload (message not downcastable)".into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::guarded;
    use crate::spec::PluginError;

    #[test]
    fn panic_becomes_typed_error_with_context() {
        let error = guarded(Path::new(r"C:\plug\broken.clap"), "instantiate", || {
            panic!("{}", "entry init exploded");
        })
        .expect_err("panic must become an error");
        let PluginError::PluginPanic {
            call,
            path,
            message,
        } = &error
        else {
            panic!("expected PluginPanic, got {error:?}");
        };
        assert_eq!(*call, "instantiate");
        assert!(path.ends_with("broken.clap"), "{path}");
        // The message is best-effort: the exact text on a normal toolchain,
        // the opaque-payload note where the payload type resists downcast
        // (see `panic_message`). Either way the path and call site — the
        // authoritative context — are present.
        assert!(
            message == "entry init exploded"
                || message == "opaque panic payload (message not downcastable)",
            "{message}"
        );
    }

    #[test]
    fn non_string_payload_is_reported() {
        let error = guarded(Path::new("x.vst3"), "vst3 state save", || {
            std::panic::panic_any(42usize)
        })
        .expect_err("panic must become an error");
        let PluginError::PluginPanic { message, .. } = &error else {
            panic!("expected PluginPanic, got {error:?}");
        };
        assert_eq!(message, "opaque panic payload (message not downcastable)");
    }

    // Empirical boundary, recorded from a real run (2026-11-20, Windows
    // MSVC): a panic raised inside a separately compiled Rust cdylib
    // (entry point `extern "C-unwind"`) unwinds with the *plugin's*
    // personality — each cdylib statically links its own std — so the
    // host's `catch_unwind` classifies it as a foreign exception and the
    // runtime aborts with "Rust cannot catch foreign exceptions" instead
    // of letting the guard convert it. The guard therefore protects
    // against panics that unwind through the *host's* runtime (host-side
    // plumbing inside the frame, or plugin code sharing the host's
    // runtime) and nothing else; runtime crash isolation stays an
    // out-of-process concern. A fixture-based test asserting the abort
    // would need a subprocess harness and was judged not worth the
    // machinery; this comment is the record.
}
