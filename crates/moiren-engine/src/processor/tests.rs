use super::*;
use crate::{
    buffer::{BufferArena, BufferSlotLayout, PortAccess},
    control::{ParamDomain, ParamSpec, parameter_channel},
    meter::level_meter,
    runtime::{Engine, EngineConfig, ExecutionPlan, OpSpec, ProcessorInstance, RtResources},
};
use moiren_core::protocol::{ParamValue, ParameterKey, ProcessorId};

const PROCESSOR: ProcessorId = ProcessorId(1);
const METER: ProcessorId = ProcessorId(2);

mod contracts;
mod gain;
mod pan;
mod sum;

const CONTEXT: ProcessContext = ProcessContext {
    timeline_epoch: 1,
    timeline_start: 17,
    frames: 4,
    processing_sr: 48000.0,
};

fn arena<S: ProcessingSample>(channels: &[usize]) -> BufferArena<S> {
    let layouts = channels
        .iter()
        .map(|&channels| BufferSlotLayout {
            channels,
            capacity_frames: 4,
        })
        .collect::<Vec<_>>();
    BufferArena::new(&layouts, 4096).unwrap()
}

fn seed<S: ProcessingSample>(arena: &mut BufferArena<S>, slot: usize, samples: &[&[f64]]) {
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(slot).unwrap(),
        }])
        .unwrap();
    arena
        .with_io(&io, 0..4, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                panic!("expected separate IO");
            };
            let mut block = outputs.get_mut(0).unwrap();
            assert_eq!(block.channel_count(), samples.len());
            for (channel, samples) in block.channels_mut().zip(samples) {
                assert_eq!(channel.len(), samples.len());
                for (dst, &src) in channel.iter_mut().zip(*samples) {
                    *dst = S::from_f64(src);
                }
            }
        })
        .unwrap();
}

fn assert_samples<S: ProcessingSample>(
    arena: &mut BufferArena<S>,
    slot: usize,
    expected: &[&[f64]],
) {
    let io = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: arena.slot(slot).unwrap(),
        }])
        .unwrap();
    arena
        .with_io(&io, 0..4, |io| {
            let ProcessIo::ReadOnly { inputs } = io else {
                panic!("expected read-only IO");
            };
            let block = inputs.get(0).unwrap();
            assert_eq!(block.channel_count(), expected.len());
            for (channel, expected) in block.channels().zip(expected) {
                assert_eq!(channel.len(), expected.len());
                for (&sample, &expected) in channel.iter().zip(*expected) {
                    assert_eq!(sample.to_f64(), expected);
                }
            }
        })
        .unwrap();
}
