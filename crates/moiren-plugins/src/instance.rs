//! The format-agnostic realtime instance contract.

use crate::spec::{PluginDescriptor, PluginError, PluginParamEvent};

/// A live plugin instance driven by the engine's audio thread.
///
/// Lifecycle: [`crate::instantiate`] creates, initializes and activates the
/// plugin; `Drop` deactivates, destroys and unloads it. All non-`process`
/// methods are control-side only and are panic-guarded by the host
/// (a Rust panic unwinding out of the plugin becomes
/// [`PluginError::PluginPanic`]); `Drop` teardown is deliberately not
/// guarded, because there is no caller to report to and calling back into
/// a panicking plugin is unsound. `process` is the realtime contract: it
/// must not allocate, lock, log or call the OS. The plugin's own
/// allocations are its author's responsibility — this crate only
/// guarantees that the host plumbing around it allocates nothing per call.
///
/// # Why `process` has no panic guard
///
/// The plugin entry points are `extern "C"`/`extern "system"` functions,
/// and the Rust `"C"` ABI is defined to abort on unwind: a Rust plugin
/// compiled with today's toolchains never legitimately unwinds across
/// `process`, and an unwind that nevertheless crosses an `extern "C"`
/// frame without aborting is undefined behavior that `catch_unwind`
/// cannot soundly intercept (the abort shim lives at that boundary).
/// Catching would also be useless as a safety story — segfaults and SEH-
/// translated C++ exceptions never were Rust unwinds — and `catch_unwind`'s
/// panic path allocates, which the RT contract forbids. A `panic=abort`
/// host profile changes none of this reasoning: it converts unwinds into
/// aborts, so there is still nothing to catch. Runtime crash isolation is
/// therefore only achievable by instantiating plugins out of process; see
/// the crate guide's follow-up line (2026-11-11 audit, M3).
pub trait PluginInstance: Send {
    /// Live descriptor; the channel counts reflect what the plugin actually
    /// negotiated, unlike a scanned descriptor which may be partial.
    fn descriptor(&self) -> &PluginDescriptor;
    fn parameters(&self) -> &[crate::spec::PluginParameterSpec];
    /// Plugin-reported latency in frames; constant while activated.
    fn latency_frames(&self) -> u32 {
        0
    }
    /// One audio block. `inputs`/`outputs` hold the main bus channel slices
    /// (the width equals the descriptor's channel counts). `events` apply at
    /// the first frame of the block.
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        events: &[PluginParamEvent],
    ) -> Result<(), PluginError>;
    /// Opaque plugin state chunk; control-side only. `None` when the plugin
    /// (or format) cannot store state.
    fn save_state(&self) -> Result<Option<Vec<u8>>, PluginError> {
        Ok(None)
    }
    fn load_state(&mut self, _state: &[u8]) -> Result<(), PluginError> {
        Ok(())
    }
    /// Factory preset names in selection order, as the plugin itself spells
    /// them. Collected by the scan probe and rendered by the preset picker;
    /// empty when the format (or this plugin) exposes no factory presets —
    /// VST3 programs and CLAP preset discovery are separate, format-specific
    /// surfaces that are not probed yet.
    fn preset_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// Select a factory preset by its index into [`Self::preset_names`].
    /// Control-side; an error means the format or plugin cannot select
    /// presets (callers only pass indexes a scan actually reported). On
    /// success the reported parameter table reflects the selected state, so
    /// hosts diffing against it never swallow the first automation.
    fn select_preset(&mut self, _index: u32) -> Result<(), PluginError> {
        Err(PluginError::State(
            "plugin exposes no selectable presets".into(),
        ))
    }
}
