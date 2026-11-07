//! The format-agnostic realtime instance contract.

use crate::spec::{PluginDescriptor, PluginError, PluginParamEvent};

/// A live plugin instance driven by the engine's audio thread.
///
/// Lifecycle: [`crate::instantiate`] creates, initializes and activates the
/// plugin; `Drop` deactivates, destroys and unloads it. All non-`process`
/// methods are control-side only. `process` is the realtime contract: it must
/// not allocate, lock, log or call the OS. The plugin's own allocations are
/// its author's responsibility — this crate only guarantees that the host
/// plumbing around it allocates nothing per call.
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
}
