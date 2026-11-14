//! Execution identity belongs to rendered segments, not preparation intent.
use moiren_core::protocol::{
    ApplyAt, ParamValue, ParameterKey, ParameterRequest, ProcessorId, ReplyCode,
};
use moiren_engine::{
    buffer::{BufferArena, BufferSlotLayout, IoMode, PortAccess, PreparedIo, ProcessIo},
    control::{ControlPort, ProcessParameters, parameter_channel},
    processor::{Gain, ProcessContext, ProcessorError, ProcessorRole, RtProcessor},
    runtime::{Engine, ExecutionPlan, OpSpec, PreparedPlan, ProcessorInstance, RtResources},
    signal::ProcessSignals,
};
use std::sync::{Arc, Mutex};

struct Capture(Arc<Mutex<Vec<ProcessContext>>>);
impl RtProcessor<f64> for Capture {
    fn role(&self) -> ProcessorRole {
        ProcessorRole::Source
    }
    fn validate_io(&self, io: &PreparedIo) -> Result<(), ProcessorError> {
        assert_eq!(io.mode(), IoMode::Separate);
        Ok(())
    }
    fn process(
        &mut self,
        ctx: &ProcessContext,
        _: ProcessIo<'_, f64>,
        _: ProcessParameters<'_>,
        _: ProcessSignals<'_>,
    ) {
        self.0.lock().unwrap().push(*ctx);
    }
}

fn engine(revision: u64, capture: &Arc<Mutex<Vec<ProcessContext>>>) -> (Engine<f64>, ControlPort) {
    engine_with_capacity(revision, capture, 8)
}

fn engine_with_capacity(
    revision: u64,
    capture: &Arc<Mutex<Vec<ProcessContext>>>,
    capacity: usize,
) -> (Engine<f64>, ControlPort) {
    let id = ProcessorId(4);
    let resources =
        RtResources::new(vec![ProcessorInstance::new(id, Capture(capture.clone()))]).unwrap();
    let (control, parameters) =
        parameter_channel(vec![Gain::parameter(id, 1.0)], revision, 7, capacity, 1024).unwrap();
    let arena = BufferArena::new(
        &[BufferSlotLayout {
            channels: 1,
            capacity_frames: 32,
        }],
        4096,
    )
    .unwrap();
    let io = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    let plan = ExecutionPlan::prepare(
        arena,
        vec![OpSpec { processor: id, io }],
        &resources,
        &parameters,
        moiren_engine::runtime::EngineConfig {
            max_block_frames: 32,
            ..super::common::engine_config()
        },
    )
    .unwrap();
    (Engine::new(plan, resources, parameters).unwrap(), control)
}

fn event(control: &mut ControlPort, revision: u64, at: u64) {
    assert_eq!(
        control
            .submit(
                ParameterRequest {
                    request_id: at + 1,
                    plan_revision: revision,
                    timeline_epoch: 7,
                    target: ParameterKey {
                        processor: ProcessorId(4),
                        parameter: Gain::LEVEL
                    },
                    at: ApplyAt::Frame(at),
                    value: ParamValue::Float(0.5),
                    ramp_frames: 4,
                },
                0
            )
            .code,
        ReplyCode::Accepted
    );
}

#[test]
fn execution_stamp_tracks_actual_segments_and_survives_plan_swap_without_reset() {
    let capture = Arc::new(Mutex::new(Vec::with_capacity(16)));
    let (mut active, mut control) = engine(11, &capture);
    let mut plans = active.enable_plan_switching(4).unwrap();
    event(&mut control, 11, 8);
    event(&mut control, 11, 16);
    active.render(24).unwrap();
    active.render(8).unwrap();
    let (candidate, mut next_control) = engine(19, &capture);
    plans
        .publish(PreparedPlan::new(candidate).unwrap())
        .unwrap();
    event(&mut next_control, 19, 34);
    active.render(8).unwrap();
    let rows = capture.lock().unwrap();
    let observed: Vec<_> = rows
        .iter()
        .map(|ctx| {
            let stamp = ctx
                .execution
                .expect("actual runtime must stamp each segment");
            (
                ctx.timeline_start,
                ctx.frames,
                stamp.plan_revision,
                stamp.control_generation,
            )
        })
        .collect();
    assert_eq!(
        observed,
        [
            (0, 8, 11, 0),
            (8, 8, 11, 1),
            (16, 8, 11, 2),
            (24, 8, 11, 2),
            (32, 2, 19, 2),
            (34, 6, 19, 3)
        ]
    );
    drop(rows);
    drop(plans.poll_retired().unwrap());
}

#[test]
fn execution_stamp_counts_applied_not_accepted_deferred_or_rejected_controls() {
    let capture = Arc::new(Mutex::new(Vec::with_capacity(16)));
    let (mut active, mut control) = engine_with_capacity(11, &capture, 1);
    event(&mut control, 11, 0);
    let (rendered, heap) = super::allocation::track_heap(|| active.render(4));
    assert_eq!(heap, (0, 0, 0));
    assert_eq!(rendered.unwrap().applied_events, 1);
    event(&mut control, 11, 4);
    let (rendered, heap) = super::allocation::track_heap(|| active.render(4));
    assert_eq!(heap, (0, 0, 0));
    assert_eq!(rendered.unwrap().applied_events, 0);
    assert_eq!(control.poll_applied().unwrap().code, ReplyCode::Applied);
    let (rendered, heap) = super::allocation::track_heap(|| active.render(4));
    assert_eq!(heap, (0, 0, 0));
    assert_eq!(rendered.unwrap().applied_events, 1);
    assert_eq!(control.poll_applied().unwrap().code, ReplyCode::AppliedLate);
    let rejected = control.submit(
        ParameterRequest {
            request_id: 99,
            plan_revision: 99,
            timeline_epoch: 7,
            target: ParameterKey {
                processor: ProcessorId(4),
                parameter: Gain::LEVEL,
            },
            at: ApplyAt::NextBlock,
            value: ParamValue::Float(1.0),
            ramp_frames: 0,
        },
        12,
    );
    assert_eq!(rejected.code, ReplyCode::StaleRevision);
    event(&mut control, 11, 16);
    active.render(4).unwrap();
    active.render(1).unwrap();
    let rows = capture.lock().unwrap();
    let observed: Vec<_> = rows
        .iter()
        .map(|ctx| ctx.execution.unwrap().control_generation)
        .collect();
    assert_eq!(observed, [1, 1, 2, 2, 3]);
}
