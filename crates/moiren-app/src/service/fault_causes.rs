//! Service-side fault-cause catalog keys (issue #86, design 21 §4/§5).
//!
//! Convention: a user-visible fault cause raised by the service layer is a
//! stable English sentence that is itself a `locales/zh-CN.yml` catalog key.
//! The UI translates it at paint (`t!(fault.cause.as_str())` and
//! `t!(hint)` in shell/fault_popover.rs, `t!(snapshot.error.as_str())` for
//! the session error), so a locale switch re-renders stored faults without
//! service involvement and a catalog miss can only fall back to the key's
//! own English wording.
//!
//! Closed error enums from the layers below (`moiren_signal`'s registration
//! failures, `moiren_engine`'s [`Diagnostic`] kinds) map variant-by-variant
//! to their keys here and nowhere else; the exhaustive `match` arms make a
//! new variant without a key a compile error, and the ledger guards
//! (`DYNAMIC_KEY_EXCEPTIONS` in i18n_guards.rs, `ALLOWED_RUNTIME_KEYS` in
//! tests/i18n_ledger.rs) keep every key catalogued.
//!
//! A cause string with data baked in can never hit the catalog, so the
//! free-form payloads those types carry (backend HRESULT prose, compile
//! diagnostics) stay diagnostic data in the service log and never enter a
//! cause — the UI-facing "kind" is localized, the detail is not (the
//! `UiFailure::ExportInvalid` precedent for producer free text).

use moiren_engine::host::{BackendError, Diagnostic};
use moiren_signal::{RegisterError, UnregisterError};

/// The cause a waiting session reports when neither a blocked update nor a
/// stored fault names the diagnosis (the old code recorded the `Option`'s
/// Debug string `None` here).
pub(super) const RESOURCE_WAIT_CAUSE: &str =
    "The audio session is waiting for an audio device or resource";

/// One complete cause key per [`RegisterError`] variant: the spectrum relay
/// registers one application topic per analyzer node
/// (service/signals.rs `prepare_node_spectra`), and a failure surfaces as a
/// graph fault cause through `record_error`.
pub(super) fn spectrum_registration(error: &RegisterError) -> &'static str {
    match error {
        RegisterError::InvalidName => {
            "Spectrum analyzer streams could not be registered: invalid topic name"
        }
        RegisterError::InvalidCapacity => {
            "Spectrum analyzer streams could not be registered: invalid capacity or bus configuration"
        }
        RegisterError::SizeOverflow => {
            "Spectrum analyzer streams could not be registered: storage layout overflow"
        }
        RegisterError::BudgetExceeded => {
            "Spectrum analyzer streams could not be registered: bus byte budget exceeded"
        }
        RegisterError::TopicLimit => {
            "Spectrum analyzer streams could not be registered: topic limit reached"
        }
        RegisterError::DuplicateName => {
            "Spectrum analyzer streams could not be registered: duplicate topic name"
        }
        RegisterError::GenerationExhausted => {
            "Spectrum analyzer streams could not be registered: identity generation exhausted"
        }
        RegisterError::PayloadTooLarge => {
            "Spectrum analyzer streams could not be registered: publication payload too large"
        }
    }
}

/// One complete cause key per [`UnregisterError`] variant: the same relay
/// unregisters a stale analyzer topic before re-registering it at a new
/// shape, and that failure surfaces through the same fault channel.
pub(super) fn spectrum_unregistration(error: &UnregisterError) -> &'static str {
    match error {
        UnregisterError::Busy => {
            "Spectrum analyzer streams could not be re-registered: the previous topic still has a publisher binding or outstanding leases"
        }
        UnregisterError::Closed => {
            "Spectrum analyzer streams could not be re-registered: the previous topic is closed"
        }
        UnregisterError::ForeignTopic => {
            "Spectrum analyzer streams could not be re-registered: the previous topic belongs to another bus"
        }
    }
}

/// One cause key per engine [`Diagnostic`] kind — the blocked-update reason
/// and the session fault of a waiting engine. The payloads stay in the
/// service log; the user sees the localized kind.
pub(super) fn diagnostic_cause(diagnostic: &Diagnostic) -> &'static str {
    match diagnostic {
        Diagnostic::Backend(error) => match error {
            BackendError::ResourceUnavailable(_) => "An audio resource is temporarily unavailable",
            BackendError::ResourceAbsent(_) => "An audio device is absent or was removed",
            BackendError::Unsupported(_) => "The audio backend does not support this configuration",
            BackendError::Failed(_) => "The audio backend failed",
            BackendError::Cancelled => "Audio preparation was cancelled",
            BackendError::WorkerPanicked => "The audio backend worker crashed",
        },
        Diagnostic::Compile(_) => "The audio engine rejected the routing plan",
        Diagnostic::Fault(_) => "The audio engine reported an internal fault",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The zh helper falls back to the English source on a catalog miss, so
    /// equality with the key means the zh-CN catalog has no entry for it.
    fn hits_catalog(key: &str) {
        assert!(!key.is_empty(), "empty cause key");
        assert_ne!(
            crate::i18n::zh(key),
            key,
            "no zh-CN catalog entry for cause key {key:?}"
        );
    }

    #[test]
    fn register_error_variants_map_to_catalog_keys() {
        let variants = [
            RegisterError::InvalidName,
            RegisterError::InvalidCapacity,
            RegisterError::SizeOverflow,
            RegisterError::BudgetExceeded,
            RegisterError::TopicLimit,
            RegisterError::DuplicateName,
            RegisterError::GenerationExhausted,
            RegisterError::PayloadTooLarge,
        ];
        for variant in variants {
            hits_catalog(spectrum_registration(&variant));
        }
    }

    #[test]
    fn unregister_error_variants_map_to_catalog_keys() {
        let variants = [
            UnregisterError::Busy,
            UnregisterError::Closed,
            UnregisterError::ForeignTopic,
        ];
        for variant in variants {
            hits_catalog(spectrum_unregistration(&variant));
        }
    }

    #[test]
    fn diagnostic_kinds_map_to_catalog_keys_without_payload_leakage() {
        let diagnostics = [
            Diagnostic::Backend(BackendError::ResourceUnavailable("probe detail".into())),
            Diagnostic::Backend(BackendError::ResourceAbsent("probe detail".into())),
            Diagnostic::Backend(BackendError::Unsupported("probe detail".into())),
            Diagnostic::Backend(BackendError::Failed("probe detail".into())),
            Diagnostic::Backend(BackendError::Cancelled),
            Diagnostic::Backend(BackendError::WorkerPanicked),
            Diagnostic::Compile("probe detail".into()),
            Diagnostic::Fault("probe detail".into()),
        ];
        for diagnostic in &diagnostics {
            let key = diagnostic_cause(diagnostic);
            hits_catalog(key);
            assert!(
                !key.contains("probe"),
                "diagnostic payload leaked into the cause key: {key:?}"
            );
        }
        hits_catalog(RESOURCE_WAIT_CAUSE);
    }
}
