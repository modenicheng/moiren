use super::*;
use crate::{control::parameter_channel, processor::Gain};
use moiren_core::protocol::{ApplyAt, ParamValue, ParameterRequest, ReplyCode};

#[test]
fn exhausted_execution_generation_stays_unknown_across_render_and_plan_swap() {
    fn engine(revision: u64) -> (Engine<f64>, crate::control::ControlPort) {
        let (control, parameters) = parameter_channel(
            vec![Gain::parameter(ProcessorId(1), 1.0)],
            revision,
            1,
            8,
            64,
        )
        .unwrap();
        let resources = RtResources::new(Vec::new()).unwrap();
        let arena = BufferArena::new(&[], 4096).unwrap();
        let plan = ExecutionPlan::prepare(
            arena,
            Vec::new(),
            &resources,
            &parameters,
            EngineConfig {
                processing_sr: 48_000.0,
                max_block_frames: 8,
                max_events_per_block: 8,
            },
        )
        .unwrap();
        (Engine::new(plan, resources, parameters).unwrap(), control)
    }
    let (mut active, mut control) = engine(1);
    let mut plans = active.enable_plan_switching(2).unwrap();
    active.control_generation = Some(u64::MAX - 1);
    for frame in [0, 1] {
        assert_eq!(
            control
                .submit(
                    ParameterRequest {
                        request_id: frame + 1,
                        plan_revision: 1,
                        timeline_epoch: 1,
                        target: Gain::parameter(ProcessorId(1), 1.0).key,
                        at: ApplyAt::Frame(frame),
                        value: ParamValue::Float(0.5),
                        ramp_frames: 0,
                    },
                    0
                )
                .code,
            ReplyCode::Accepted
        );
    }
    active.render(1).unwrap();
    assert_eq!(active.control_generation, Some(u64::MAX));
    active.render(1).unwrap();
    assert_eq!(active.control_generation, None);
    active.render(1).unwrap();
    assert_eq!(active.control_generation, None);
    let (candidate, _control) = engine(2);
    plans
        .publish(PreparedPlan::new(candidate).unwrap())
        .unwrap();
    active.render(1).unwrap();
    assert_eq!(active.control_generation, None);
    drop(plans.poll_retired().unwrap());
}
