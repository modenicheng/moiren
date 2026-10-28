//! Probe-owned graph rendering over the production native stream contract.
//! All native objects stay on their creating thread; graph/staging ownership
//! returns to the control joiner after Stop, including failed preparation.
#[cfg(windows)]
#[path = "render/owner.rs"]
mod owner;
#[path = "render/renderer.rs"]
mod renderer;

#[cfg(windows)]
pub(super) use owner::{
    PreparedNativeRender, RenderOptions, RenderReport, RenderSession,
    prepare_render_native_with_gate_returning_with_abort,
};
#[cfg(windows)]
pub(super) use renderer::{DemandRenderer, TimelineObserver};
