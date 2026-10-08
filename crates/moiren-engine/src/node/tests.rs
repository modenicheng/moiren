use crate::{boundary::*, buffer::*, control::*, node::*, processor::*, sample::ProcessingSample};
use moiren_core::protocol::ProcessorId;

fn input_config(channels: usize) -> InputConfig {
    InputConfig {
        source: InputSource::Software {
            name: "test input".into(),
        },
        channels,
        device: DeviceOptions::default(),
    }
}
fn output_config(channels: usize) -> OutputConfig {
    OutputConfig {
        target: OutputTarget::Software {
            name: "test output".into(),
        },
        channels,
        device: DeviceOptions::default(),
    }
}
fn context(start: u64, frames: usize) -> ProcessContext {
    ProcessContext {
        timeline_epoch: 7,
        timeline_start: start,
        frames,
        processing_sr: 48_000.0,
    }
}
fn make_arena<S: ProcessingSample>(channels: usize) -> BufferArena<S> {
    BufferArena::new(
        &[BufferSlotLayout {
            channels,
            capacity_frames: 8,
        }; 2],
        4096,
    )
    .unwrap()
}
fn process<S: ProcessingSample>(
    node: &mut impl RtProcessor<S>,
    arena: &mut BufferArena<S>,
    io: &PreparedIo,
    ctx: ProcessContext,
) {
    let (_, params) = parameter_channel(vec![], 1, ctx.timeline_epoch, 1, 16).unwrap();
    let bindings = params.bindings(ProcessorId(1));
    node.validate_io(io).unwrap();
    arena
        .with_io(io, 0..ctx.frames, |io| {
            node.process(&ctx, io, params.view(&bindings))
        })
        .unwrap();
}

struct PrefixSource {
    transferred: usize,
}
impl<S: ProcessingSample> RtAudioSource<S> for PrefixSource {
    fn channel_count(&self) -> usize {
        2
    }
    fn read(&mut self, _: &ProcessContext, mut output: AudioBlockMut<'_, S>) -> BoundaryReport {
        // Deliberately dirty the tail: only the reported prefix is valid.
        for (ch, samples) in output.channels_mut().enumerate() {
            samples.fill(S::from_f64((ch + 1) as f64));
        }
        BoundaryReport {
            transferred_frames: self.transferred,
            ..BoundaryReport::default()
        }
    }
}
struct ShortSink;
impl<S: ProcessingSample> RtAudioSink<S> for ShortSink {
    fn channel_count(&self) -> usize {
        2
    }
    fn write(&mut self, _: &ProcessContext, _: AudioBlock<'_, S>) -> BoundaryReport {
        BoundaryReport {
            transferred_frames: 2,
            xruns: 3,
            discontinuity: true,
        }
    }
}

#[test]
fn input_clears_unreported_tail_and_invalid_report_for_both_precisions() {
    fn check<S: ProcessingSample>() {
        for transferred in [0, 2, 4, 5] {
            let (mut input, mut reader) =
                InputNode::new(input_config(2), PrefixSource { transferred }, 2).unwrap();
            let mut arena = make_arena::<S>(2);
            let slot = arena.slot(0).unwrap();
            let write = arena
                .prepare_io(&[PortAccess::Write { port: 0, slot }])
                .unwrap();
            let read = arena
                .prepare_io(&[PortAccess::Read { port: 0, slot }])
                .unwrap();
            process(&mut input, &mut arena, &write, context(10, 4));
            let valid = if transferred > 4 { 0 } else { transferred };
            arena
                .with_io(&read, 0..4, |io| {
                    let ProcessIo::ReadOnly { inputs } = io else {
                        unreachable!()
                    };
                    for ch in 0..2 {
                        let block = inputs.get(0).unwrap();
                        for (frame, sample) in block.channel(ch).iter().enumerate() {
                            assert_eq!(
                                sample.to_f64(),
                                if frame < valid { (ch + 1) as f64 } else { 0.0 }
                            );
                        }
                    }
                })
                .unwrap();
            let status = reader.latest().unwrap();
            assert_eq!(status.report.transferred_frames, valid);
            assert_eq!(status.shortfall_frames, 4 - valid);
            assert_eq!(status.invalid_report, transferred > 4);
            assert_eq!(status.total_invalid_reports, u64::from(transferred > 4));
            assert_eq!(status.total_xruns, u64::from(valid < 4));
            assert_eq!(status.timeline_epoch, 7);
            assert_eq!((status.start_frame, status.end_frame), (10, 14));
        }
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn nodes_reject_wrong_channels_ports_and_in_place() {
    assert!(matches!(
        InputNode::<f32, _>::new(input_config(1), PrefixSource { transferred: 4 }, 1),
        Err(NodeError::ChannelMismatch)
    ));
    assert!(matches!(
        OutputNode::<f32, _>::new(output_config(1), ShortSink, 1),
        Err(NodeError::ChannelMismatch)
    ));
    assert!(matches!(
        InputNode::<f32, _>::new(input_config(2), PrefixSource { transferred: 4 }, 0),
        Err(NodeError::InvalidTelemetryCapacity)
    ));
    let (input, _) = InputNode::new(input_config(2), PrefixSource { transferred: 4 }, 1).unwrap();
    let (output, _) = OutputNode::new(output_config(2), ShortSink, 1).unwrap();
    assert_eq!(input.config(), &input_config(2));
    assert_eq!(output.config(), &output_config(2));
    assert_eq!(RtProcessor::<f32>::role(&input), ProcessorRole::Source);
    assert_eq!(RtProcessor::<f32>::role(&output), ProcessorRole::Sink);
    let arena = make_arena::<f32>(2);
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    for ports in [
        vec![],
        vec![PortAccess::Write { port: 1, slot: a }],
        vec![
            PortAccess::Read { port: 0, slot: a },
            PortAccess::Write { port: 0, slot: b },
        ],
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: a,
        }],
        vec![
            PortAccess::Write { port: 0, slot: a },
            PortAccess::Write { port: 1, slot: b },
        ],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f32>::validate_io(&input, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    for ports in [
        vec![],
        vec![PortAccess::Read { port: 1, slot: a }],
        vec![
            PortAccess::Read { port: 0, slot: a },
            PortAccess::Write { port: 0, slot: b },
        ],
        vec![PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: a,
        }],
        vec![
            PortAccess::Read { port: 0, slot: a },
            PortAccess::Read { port: 1, slot: b },
        ],
    ] {
        let io = arena.prepare_io(&ports).unwrap();
        assert_eq!(
            RtProcessor::<f32>::validate_io(&output, &io),
            Err(ProcessorError::InvalidIo)
        );
    }
    let mismatch = make_arena::<f32>(1);
    let io = mismatch
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: mismatch.slot(0).unwrap(),
        }])
        .unwrap();
    assert_eq!(
        RtProcessor::<f32>::validate_io(&input, &io),
        Err(ProcessorError::InvalidIo)
    );
}

#[test]
fn output_diagnostics_accumulate_through_backpressure_and_timeline_changes() {
    let (mut output, mut reader) = OutputNode::new(output_config(2), ShortSink, 1).unwrap();
    let mut arena = make_arena::<f64>(2);
    let io = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    for start in [0, 4, 8] {
        process(&mut output, &mut arena, &io, context(start, 4));
    }
    let first = reader.latest().unwrap();
    assert_eq!(first.total_shortfall_frames, 2);
    assert!(reader.latest().is_none());
    let mut ctx = context(12, 4);
    ctx.timeline_epoch = 8;
    process(&mut output, &mut arena, &io, ctx);
    let status = reader.latest().unwrap();
    assert_eq!(status.timeline_epoch, 8);
    assert_eq!(status.total_transferred_frames, 8);
    assert_eq!(status.total_shortfall_frames, 8);
    assert_eq!(status.total_xruns, 12);
    assert_eq!(status.total_discontinuities, 4);
    assert_eq!(status.dropped_snapshots, 2);
}

#[test]
fn timeline_gap_and_epoch_change_are_reported_for_successful_source() {
    let (mut input, mut reader) =
        InputNode::new(input_config(2), PrefixSource { transferred: 4 }, 1).unwrap();
    let mut arena = make_arena::<f32>(2);
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    for (start, epoch, discontinuity) in
        [(0, 7, false), (4, 7, false), (20, 7, true), (24, 8, true)]
    {
        let mut ctx = context(start, 4);
        ctx.timeline_epoch = epoch;
        process(&mut input, &mut arena, &io, ctx);
        assert_eq!(reader.latest().unwrap().report.discontinuity, discontinuity);
    }
}

#[test]
fn invalid_sink_report_is_diagnosed_without_mutating_audio() {
    struct InvalidSink;
    impl RtAudioSink<f64> for InvalidSink {
        fn channel_count(&self) -> usize {
            2
        }
        fn write(&mut self, _: &ProcessContext, input: AudioBlock<'_, f64>) -> BoundaryReport {
            assert_eq!(input.channel(0), &[0.0; 4]);
            BoundaryReport {
                transferred_frames: usize::MAX,
                xruns: 0,
                discontinuity: false,
            }
        }
    }
    let (mut output, mut reader) = OutputNode::new(output_config(2), InvalidSink, 1).unwrap();
    let mut arena = make_arena::<f64>(2);
    let io = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    process(&mut output, &mut arena, &io, context(0, 4));
    let status = reader.latest().unwrap();
    assert_eq!(status.report.transferred_frames, 0);
    assert_eq!(status.total_invalid_reports, 1);
    assert_eq!(status.total_shortfall_frames, 4);
    assert_eq!(status.total_xruns, 1);
    assert!(status.report.discontinuity);
    assert!(matches!(
        OutputNode::<f64, _>::new(output_config(2), InvalidSink, usize::MAX),
        Err(NodeError::InvalidTelemetryCapacity)
    ));
}
