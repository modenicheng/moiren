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
mod clock;
mod provider;

pub use boundary::{
    BoundaryProbe, BoundaryProbeSample, VirtualMicrophoneSink, VirtualPlaybackSource,
};
#[allow(unused_imports)]
pub use clock::{ClockError, ClockSchedule, ClockTick};
#[allow(unused_imports)]
pub use provider::{
    PreparedVirtualMicrophone, PreparedVirtualPlayback, VirtualBoundaryProvider,
    VirtualBoundaryRequest, VirtualResourceSession, VirtualSessions,
};

#[cfg(windows)]
mod owner;
#[cfg(windows)]
mod pacer;
#[cfg(windows)]
pub use owner::VirtualPreparedTransport;
#[cfg(windows)]
pub use pacer::{ClockWake, QpcClockFactory, VirtualClockFactory, VirtualRealtimeClock};
