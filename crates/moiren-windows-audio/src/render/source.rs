//! Native render callback polymorphism. Only Master contains an Engine; a
//! follower can only consume its own prepared continuously adapted queue.
use super::{DemandCounters, DemandRenderer, DemandReport, RenderError};
use crate::clock_bridge::ClockSource;

pub(crate) trait SampleRenderer {
    fn timeline(&self) -> u64;
    fn counters(&self) -> DemandCounters;
    fn render_interleaved(&mut self, samples: &mut [f32]) -> Result<DemandReport, RenderError>;
    fn acknowledge_submission(&self);
    fn bridge_snapshot(&self) -> Option<crate::clock_bridge::BridgeSnapshot> {
        None
    }
}
impl SampleRenderer for DemandRenderer {
    fn timeline(&self) -> u64 {
        self.timeline()
    }
    fn counters(&self) -> DemandCounters {
        self.counters()
    }
    fn render_interleaved(&mut self, samples: &mut [f32]) -> Result<DemandReport, RenderError> {
        self.render_interleaved(samples)
    }
    fn acknowledge_submission(&self) {
        self.acknowledge_submission();
    }
}
pub(crate) struct FollowerRenderer {
    source: ClockSource,
    frames: u64,
}
impl FollowerRenderer {
    pub(crate) fn new(source: ClockSource) -> Self {
        Self { source, frames: 0 }
    }
}
impl SampleRenderer for FollowerRenderer {
    fn bridge_snapshot(&self) -> Option<crate::clock_bridge::BridgeSnapshot> {
        Some(self.source.snapshot())
    }
    fn timeline(&self) -> u64 {
        self.frames
    }
    fn counters(&self) -> DemandCounters {
        DemandCounters::default()
    }
    fn render_interleaved(&mut self, samples: &mut [f32]) -> Result<DemandReport, RenderError> {
        self.source
            .read_interleaved(samples)
            .map_err(|_| RenderError::InvalidSamples)?;
        let frames = samples.len() / 2;
        self.frames = self.frames.saturating_add(frames as u64);
        Ok(DemandReport {
            frames,
            ..DemandReport::default()
        })
    }
    fn acknowledge_submission(&self) {}
}
pub(crate) enum NativeRenderer {
    Master(Box<DemandRenderer>),
    Follower(Box<FollowerRenderer>),
}
impl NativeRenderer {
    pub(crate) fn into_master(self) -> Option<DemandRenderer> {
        match self {
            Self::Master(renderer) => Some(*renderer),
            Self::Follower(_) => None,
        }
    }
}
impl SampleRenderer for NativeRenderer {
    fn bridge_snapshot(&self) -> Option<crate::clock_bridge::BridgeSnapshot> {
        match self {
            Self::Master(renderer) => renderer.bridge_snapshot(),
            Self::Follower(renderer) => renderer.bridge_snapshot(),
        }
    }
    fn timeline(&self) -> u64 {
        match self {
            Self::Master(renderer) => renderer.timeline(),
            Self::Follower(renderer) => renderer.timeline(),
        }
    }
    fn counters(&self) -> DemandCounters {
        match self {
            Self::Master(renderer) => renderer.counters(),
            Self::Follower(renderer) => renderer.counters(),
        }
    }
    fn render_interleaved(&mut self, samples: &mut [f32]) -> Result<DemandReport, RenderError> {
        match self {
            Self::Master(renderer) => renderer.render_interleaved(samples),
            Self::Follower(renderer) => renderer.render_interleaved(samples),
        }
    }
    fn acknowledge_submission(&self) {
        match self {
            Self::Master(renderer) => renderer.acknowledge_submission(),
            Self::Follower(renderer) => renderer.acknowledge_submission(),
        }
    }
}
