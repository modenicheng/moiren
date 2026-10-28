//! Prepared virtual boundaries and the single Engine's virtual realtime clock.
//!
//! Transport factories run on control, transport workers own driver operations,
//! and graph callbacks see only bounded preallocated audio bridges. A virtual
//! clock is selected only when the project has no physical render master.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectClock {
    Physical(moiren_core::graph::NodeId),
    VirtualRealtime,
}

mod boundary;
mod budget;
mod clock;
mod provider;
mod reprime;

pub use boundary::{
    BoundaryProbe, BoundaryProbeSample, VirtualMicrophoneSink, VirtualPlaybackSource,
};
pub use budget::{PreparedVirtualBudget, VirtualBufferOptions};
#[allow(unused_imports)]
pub use clock::{ClockError, ClockSchedule, ClockTick, DEBT_CAP_PERIODS};
#[allow(unused_imports)]
pub use provider::{
    PreparedVirtualMicrophone, PreparedVirtualPlayback, VirtualBoundaryProvider,
    VirtualBoundaryRequest, VirtualResourceSession, VirtualSessions, bind_microphone_sink,
    bind_playback_source,
};
pub use reprime::PreparedMicrophoneReprime;

#[cfg(windows)]
mod owner;
#[cfg(windows)]
mod pacer;
#[cfg(windows)]
pub use owner::VirtualPreparedTransport;
#[cfg(windows)]
pub use pacer::{ClockWake, QpcClockFactory, VirtualClockFactory, VirtualRealtimeClock};
