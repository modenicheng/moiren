//! Third-party plugin hosting for Moiren: CLAP, VST2, and VST3 binary
//! loading, scanning, and realtime instances.
//!
//! This crate is deliberately a leaf: it knows the plugin ABIs and its own
//! [`spec`] types, nothing about the engine. The engine crate adapts live
//! [`PluginInstance`]s onto its `RtProcessor` contract.
//!
//! # Realtime contract
//!
//! [`PluginInstance::process`] is the only realtime entry point. All host
//! plumbing it touches (pointer tables, event queues, parameter-change
//! containers) is preallocated at instantiation; the host side of the call
//! performs no allocation, locking, or syscalls. A plugin's own RT behavior
//! is its author's responsibility.
//!
//! # Licensing notes
//!
//! CLAP ABI declarations are transcribed from the MIT-licensed clap headers.
//! VST3 interface declarations are transcribed from the Steinberg VST3 SDK
//! pluginterfaces headers (GPLv3/proprietary dual-licensed SDK); only the
//! published binary interface constants and layouts are reproduced, which
//! is the same approach as other independent hosts. The VST2 SDK is no
//! longer distributed; the AEffect interface mirrored here is the public,
//! widely documented binary interface that existing plugins were built
//! against, re-derived from the published specification.

pub mod clap;
pub mod sandbox;
pub mod scan;
pub mod spec;
pub mod vst2;
pub mod vst3;

mod guard;
mod instance;
mod module;

pub use instance::PluginInstance;
pub use spec::{
    PluginDescriptor, PluginError, PluginFormat, PluginParamEvent, PluginParameterSpec,
};

/// Create a live, activated plugin instance from a scanned descriptor.
///
/// Control-side only. `sample_rate`/`max_block_frames` must match the engine
/// configuration that will drive `process`; the instance activates for
/// exactly those bounds. The descriptor's channel counts must satisfy the
/// equal-width main-bus rule (enforced here with
/// [`PluginError::NoAudioPorts`]). A Rust panic unwinding out of any plugin
/// call during instantiation becomes [`PluginError::PluginPanic`]; the host
/// process survives and the half-built plugin object is deliberately leaked
/// (never call back into a plugin that just panicked).
pub fn instantiate(
    descriptor: &PluginDescriptor,
    sample_rate: f64,
    max_block_frames: u32,
) -> Result<Box<dyn PluginInstance>, PluginError> {
    // Every format's instantiate covers all control-side FFI of the setup
    // path (module load, entry init, factory queries, create/init/activate)
    // inside one guard frame.
    let path = descriptor.path.clone();
    let (format, sample_rate, max_block_frames) =
        (descriptor.format, sample_rate, max_block_frames);
    guard::guarded(&path, "instantiate", move || match format {
        PluginFormat::Clap => Ok(Box::new(clap::instantiate(
            descriptor,
            sample_rate,
            max_block_frames,
        )?) as Box<dyn PluginInstance>),
        PluginFormat::Vst3 => Ok(Box::new(vst3::instantiate(
            descriptor,
            sample_rate,
            max_block_frames,
        )?) as Box<dyn PluginInstance>),
        PluginFormat::Vst2 => Ok(Box::new(vst2::instantiate(
            descriptor,
            sample_rate,
            max_block_frames,
        )?) as Box<dyn PluginInstance>),
    })
    .and_then(|instance| instance)
}
