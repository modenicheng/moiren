//! Prepared IO nodes. Device preparation/negotiation and destruction stay non-RT.
use std::marker::PhantomData;

use rtrb::{Consumer, Producer, RingBuffer};
use thiserror::Error;

mod config;
pub use config::*;

use crate::{
    boundary::{
        BoundaryReport, RtAudioSink, RtAudioSource, normalize_report, read_source,
        validate_sink_io, validate_source_io,
    },
    buffer::{PreparedIo, ProcessIo},
    control::ProcessParameters,
    processor::{ProcessContext, ProcessorError, ProcessorRole, RtProcessor},
    sample::ProcessingSample,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NodeError {
    #[error(transparent)]
    Config(#[from] IoConfigError),
    #[error("IO configuration and prepared backend channel counts differ")]
    ChannelMismatch,
    #[error("telemetry capacity is zero or exceeds addressable memory")]
    InvalidTelemetryCapacity,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoundarySnapshot {
    pub timeline_epoch: u64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub report: BoundaryReport,
    pub shortfall_frames: usize,
    pub invalid_report: bool,
    pub total_transferred_frames: u64,
    pub total_shortfall_frames: u64,
    pub total_xruns: u64,
    pub total_discontinuities: u64,
    pub total_invalid_reports: u64,
    pub dropped_snapshots: u64,
}

pub struct BoundaryReader {
    rx: Consumer<BoundarySnapshot>,
}
impl BoundaryReader {
    /// Latest queued snapshot; a full queue drops new reports. Work is bounded
    /// by the initial queue length, not by concurrent producer traffic.
    pub fn latest(&mut self) -> Option<BoundarySnapshot> {
        let count = self.rx.slots();
        let mut latest = None;
        for _ in 0..count {
            if let Ok(snapshot) = self.rx.pop() {
                latest = Some(snapshot);
            }
        }
        latest
    }
}

struct BoundaryTelemetry {
    tx: Producer<BoundarySnapshot>,
    totals: BoundarySnapshot,
    previous_end: Option<(u64, u64)>,
}
impl BoundaryTelemetry {
    fn new(capacity: usize) -> Result<(Self, BoundaryReader), NodeError> {
        if capacity == 0 || capacity > isize::MAX as usize / std::mem::size_of::<BoundarySnapshot>()
        {
            return Err(NodeError::InvalidTelemetryCapacity);
        }
        let (tx, rx) = RingBuffer::new(capacity);
        Ok((
            Self {
                tx,
                totals: BoundarySnapshot::default(),
                previous_end: None,
            },
            BoundaryReader { rx },
        ))
    }
    fn publish(&mut self, ctx: &ProcessContext, mut report: BoundaryReport, invalid: bool) {
        if self
            .previous_end
            .is_some_and(|previous| previous != (ctx.timeline_epoch, ctx.timeline_start))
        {
            report.discontinuity = true;
        }
        let end = ctx.timeline_start.saturating_add(ctx.frames as u64);
        self.previous_end = Some((ctx.timeline_epoch, end));
        let shortfall = ctx.frames.saturating_sub(report.transferred_frames);
        let totals = &mut self.totals;
        totals.timeline_epoch = ctx.timeline_epoch;
        totals.start_frame = ctx.timeline_start;
        totals.end_frame = end;
        totals.report = report;
        totals.shortfall_frames = shortfall;
        totals.invalid_report = invalid;
        totals.total_transferred_frames = totals
            .total_transferred_frames
            .saturating_add(report.transferred_frames as u64);
        totals.total_shortfall_frames = totals
            .total_shortfall_frames
            .saturating_add(shortfall as u64);
        totals.total_xruns = totals.total_xruns.saturating_add(report.xruns);
        totals.total_discontinuities = totals
            .total_discontinuities
            .saturating_add(u64::from(report.discontinuity));
        totals.total_invalid_reports = totals
            .total_invalid_reports
            .saturating_add(u64::from(invalid));
        if self.tx.push(*totals).is_err() {
            totals.dropped_snapshots = totals.dropped_snapshots.saturating_add(1);
        }
    }
}

pub struct InputNode<S: ProcessingSample, T> {
    config: InputConfig,
    source: T,
    telemetry: BoundaryTelemetry,
    sample: PhantomData<S>,
}
impl<S: ProcessingSample, T: RtAudioSource<S>> InputNode<S, T> {
    /// Non-RT only. The source must already emit planar audio at Processing SR;
    /// hardware settings remain backend intent, not work performed by this node.
    pub fn new(
        config: InputConfig,
        source: T,
        telemetry_capacity: usize,
    ) -> Result<(Self, BoundaryReader), NodeError> {
        config.validate()?;
        if config.channels != source.channel_count() {
            return Err(NodeError::ChannelMismatch);
        }
        let (telemetry, reader) = BoundaryTelemetry::new(telemetry_capacity)?;
        Ok((
            Self {
                config,
                source,
                telemetry,
                sample: PhantomData,
            },
            reader,
        ))
    }
    /// Control-side metadata; not queried while processing.
    pub fn config(&self) -> &InputConfig {
        &self.config
    }
}
impl<S: ProcessingSample, T: RtAudioSource<S>> RtProcessor<S> for InputNode<S, T> {
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Source
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        validate_source_io(io, self.config.channels)
    }
    fn process(&mut self, ctx: &ProcessContext, io: ProcessIo<'_, S>, _: ProcessParameters<'_>) {
        if let ProcessIo::Separate { mut outputs, .. } = io {
            let (report, invalid) = read_source(&mut self.source, ctx, &mut outputs);
            self.telemetry.publish(ctx, report, invalid);
        }
    }
}

pub struct OutputNode<S: ProcessingSample, T> {
    config: OutputConfig,
    sink: T,
    telemetry: BoundaryTelemetry,
    sample: PhantomData<S>,
}
impl<S: ProcessingSample, T: RtAudioSink<S>> OutputNode<S, T> {
    /// Non-RT only; the sink synchronously consumes or copies into its own bridge.
    pub fn new(
        config: OutputConfig,
        sink: T,
        telemetry_capacity: usize,
    ) -> Result<(Self, BoundaryReader), NodeError> {
        config.validate()?;
        if config.channels != sink.channel_count() {
            return Err(NodeError::ChannelMismatch);
        }
        let (telemetry, reader) = BoundaryTelemetry::new(telemetry_capacity)?;
        Ok((
            Self {
                config,
                sink,
                telemetry,
                sample: PhantomData,
            },
            reader,
        ))
    }
    pub fn config(&self) -> &OutputConfig {
        &self.config
    }
}
impl<S: ProcessingSample, T: RtAudioSink<S>> RtProcessor<S> for OutputNode<S, T> {
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Sink
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        validate_sink_io(io, self.config.channels)
    }
    fn process(&mut self, ctx: &ProcessContext, io: ProcessIo<'_, S>, _: ProcessParameters<'_>) {
        if let ProcessIo::ReadOnly { inputs } = io {
            let input = inputs.get(0).expect("validated output node input");
            let frames = input.frames();
            let (report, invalid) = normalize_report(self.sink.write(ctx, input), frames);
            self.telemetry.publish(ctx, report, invalid);
        }
    }
}
