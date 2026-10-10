use super::support::{EPOCH, GAIN, REV, config, pipeline, request, values};
use moiren_core::protocol::{
    ApplyAt, ParamValue, ParameterRequest, ReplyCode, read_parameter, read_reply, write_parameter,
    write_reply,
};
use moiren_engine::{
    buffer::{BufferArena, BufferSlotLayout, PortAccess},
    control::parameter_channel,
    processor::Gain,
    runtime::{Engine, ExecutionPlan, OpSpec, ProcessorInstance, RtResources, RuntimeError},
};

#[test]
fn stopped_controls_preserve_every_terminal_reply_under_backpressure() {
    let (mut engine, mut control, _, _) = pipeline(true, 4, 2);
    for id in 1..=2 {
        assert_eq!(
            control
                .submit(request(id, ApplyAt::Frame(0), 1.0, 0), 0)
                .code,
            ReplyCode::Accepted
        );
    }
    engine.render(4).unwrap();
    for id in 3..=4 {
        assert_eq!(
            control
                .submit(request(id, ApplyAt::Frame(100), 1.0, 0), 4)
                .code,
            ReplyCode::Accepted
        );
    }
    assert_eq!(engine.retire_controls(), 2);
    assert_eq!(
        control
            .submit(request(5, ApplyAt::Frame(100), 1.0, 0), 4)
            .code,
        ReplyCode::StaleRevision
    );
    let mut replies = Vec::new();
    loop {
        while let Some(reply) = control.poll_applied() {
            replies.push((reply.request_id, reply.code));
        }
        if engine.retire_controls() == 0 {
            break;
        }
    }
    while let Some(reply) = control.poll_applied() {
        replies.push((reply.request_id, reply.code));
    }
    assert_eq!(
        replies,
        [
            (1, ReplyCode::Applied),
            (2, ReplyCode::Applied),
            (3, ReplyCode::StaleRevision),
            (4, ReplyCode::StaleRevision)
        ]
    );
    assert_eq!(engine.retire_controls(), 0);
    assert!(control.poll_applied().is_none());
    assert_eq!(engine.timeline(), 4);
}

#[test]
fn event_at_end_waits_until_next_block_and_ramps_cross_blocks() {
    for in_place in [false, true] {
        let (mut engine, mut control, mut meter, samples) = pipeline(in_place, 16, 16);
        assert_eq!(
            control
                .submit(request(1, ApplyAt::Frame(3), 2.0, 2), 0)
                .code,
            ReplyCode::Accepted
        );
        assert_eq!(
            control
                .submit(request(2, ApplyAt::Frame(8), 0.0, 0), 0)
                .code,
            ReplyCode::Accepted
        );
        assert_eq!(engine.render(4).unwrap().segments, 2);
        engine.render(4).unwrap();
        assert_eq!(
            values(&samples, 8),
            vec![0.25, 0.25, 0.25, 0.375, 0.5, 0.5, 0.5, 0.5]
        );
        assert_eq!(control.poll_applied().unwrap().effective_frame, 3);
        assert!(control.poll_applied().is_none());
        engine.render(1).unwrap();
        assert_eq!(values(&samples, 9)[8], 0.0);
        assert_eq!(control.poll_applied().unwrap().effective_frame, 8);
        let pre_gain = meter.latest().unwrap();
        assert_eq!(pre_gain.peak, 0.25);
        assert_eq!(pre_gain.rms, 0.25);
    }
}

#[test]
fn ingress_rejects_stale_bad_typed_unordered_and_full_requests() {
    let (mut engine, mut control, _, _) = pipeline(true, 2, 2);
    let base = request(1, ApplyAt::Frame(4), 1.0, 0);
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    plan_revision: REV + 1,
                    ..base
                },
                0
            )
            .code,
        ReplyCode::StaleRevision
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    timeline_epoch: EPOCH + 1,
                    ..base
                },
                0
            )
            .code,
        ReplyCode::StaleEpoch
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    value: ParamValue::Bool(true),
                    ..base
                },
                0
            )
            .code,
        ReplyCode::InvalidValue
    );
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    value: ParamValue::Float(f64::NAN),
                    ..base
                },
                0
            )
            .code,
        ReplyCode::InvalidValue
    );
    assert_eq!(
        control
            .submit(request(1, ApplyAt::Frame(2048), 1.0, 0), 0)
            .code,
        ReplyCode::InvalidTime
    );
    assert_eq!(control.submit(base, 0).code, ReplyCode::Accepted);
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(3), 1.0, 0), 0)
            .code,
        ReplyCode::OutOfOrder
    );
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(4), 2.0, 0), 0)
            .code,
        ReplyCode::Accepted
    );
    assert_eq!(
        control
            .submit(request(3, ApplyAt::Frame(4), 3.0, 0), 0)
            .code,
        ReplyCode::QueueFull
    );
    engine.render(8).unwrap();
    assert_eq!(control.poll_applied().unwrap().request_id, 1);
    assert_eq!(control.poll_applied().unwrap().request_id, 2);
}

#[test]
fn reply_backpressure_never_stalls_audio_or_loses_an_applied_ack() {
    let (mut engine, mut control, _, samples) = pipeline(true, 4, 1);
    control.submit(request(1, ApplyAt::Frame(0), 2.0, 0), 0);
    engine.render(4).unwrap();
    // Applied reply is still queued. A second command is accepted but deferred.
    assert_eq!(
        control
            .submit(request(2, ApplyAt::Frame(4), 0.0, 0), 4)
            .code,
        ReplyCode::Accepted
    );
    assert_eq!(engine.render(4).unwrap().applied_events, 0);
    assert_eq!(engine.timeline(), 8);
    assert_eq!(values(&samples, 8), vec![0.5; 8]);
    assert_eq!(control.poll_applied().unwrap().request_id, 1);
    engine.render(1).unwrap();
    let ack = control.poll_applied().unwrap();
    assert_eq!(ack.request_id, 2);
    assert_eq!(ack.code, ReplyCode::AppliedLate);
    assert_eq!(ack.effective_frame, 8);
    assert_eq!(values(&samples, 9)[8], 0.0);
}

#[test]
fn work_budget_bounds_segmentation_and_defers_excess_events() {
    let (mut engine, mut control, _, _) = pipeline(true, 1, 8);
    for frame in 1..=3 {
        control.submit(request(frame, ApplyAt::Frame(frame), frame as f64, 0), 0);
    }
    let report = engine.render(8).unwrap();
    assert_eq!(report.applied_events, 1);
    assert_eq!(report.segments, 2);
    engine.render(8).unwrap();
    control.poll_applied().unwrap();
    let late = control.poll_applied().unwrap();
    assert_eq!(late.code, ReplyCode::AppliedLate);
    assert_eq!(late.effective_frame, 8);
}

#[test]
fn prepared_tables_cannot_be_replaced_by_same_shaped_foreign_tables() {
    let (_, params) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, 8, 64).unwrap();
    let (_, foreign) =
        parameter_channel(vec![Gain::parameter(GAIN, 1.0)], REV, EPOCH, 8, 64).unwrap();
    let resources = RtResources::<f64>::new(vec![ProcessorInstance::new(GAIN, Gain)]).unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 16,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::InPlace {
            input: 0,
            output: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    let plan = ExecutionPlan::prepare(
        arena,
        vec![OpSpec {
            processor: GAIN,
            io,
        }],
        &resources,
        &params,
        config(8),
    )
    .unwrap();
    assert!(matches!(
        Engine::new(plan, resources, foreign),
        Err(RuntimeError::ForeignParameters)
    ));
}

#[test]
fn decoding_happens_before_spsc_and_applied_reply_roundtrips() {
    let (mut engine, mut control, _, _) = pipeline(false, 8, 8);
    let mut wire = Vec::new();
    write_parameter(&mut wire, request(10, ApplyAt::NextBlock, 0.5, 0)).unwrap();
    let decoded = read_parameter(&mut wire.as_slice()).unwrap();
    let accepted = control.submit(decoded, engine.timeline());
    assert_eq!(accepted.code, ReplyCode::Accepted);
    engine.render(1).unwrap();
    let applied = control.poll_applied().unwrap();
    let mut response = Vec::new();
    write_reply(&mut response, applied).unwrap();
    assert_eq!(
        read_reply(&mut response.as_slice()).unwrap().code,
        ReplyCode::Applied
    );
}

#[test]
fn engine_and_queue_endpoints_move_to_distinct_threads_without_shared_arena() {
    let (engine, mut control, _, samples) = pipeline(true, 8, 8);
    control.submit(request(1, ApplyAt::Frame(0), 2.0, 0), 0);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let mut engine = engine;
        engine.render(8).unwrap();
        // Test synchronization occurs OUTSIDE render, not in a processor.
        ready_tx.send(()).unwrap();
        continue_rx.recv().unwrap();
        engine.render(8).unwrap();
        engine
    });
    ready_rx.recv().unwrap();
    assert_eq!(control.poll_applied().unwrap().effective_frame, 0);
    control.submit(request(2, ApplyAt::Frame(8), 0.0, 0), 8);
    continue_tx.send(()).unwrap();
    let engine = join.join().unwrap();
    assert_eq!(engine.timeline(), 16);
    assert_eq!(values(&samples, 16), [vec![0.5; 8], vec![0.0; 8]].concat());
    // Owned resources return to the non-RT owner before destruction.
    drop(engine.into_parts());
}
