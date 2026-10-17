use super::*;
use crate::{
    buffer::{BufferArena, BufferSlotLayout, PortAccess, ProcessIo},
    control::parameter_channel,
    processor::{Gain, Pan, ProcessContext},
    sample::ProcessingSample,
};
use moiren_core::protocol::ParamValue;

const PROCESSOR: ProcessorId = ProcessorId(91);
const SAMPLE_RATE: f64 = 1_000.0;

fn render<S: ProcessingSample>(
    rack: &RackSettings,
    samples: &[Vec<f64>],
    chunks: &[usize],
) -> Vec<Vec<f64>> {
    let channels = samples.len();
    let frames = samples[0].len();
    assert!(rack.valid_for(channels));
    assert_eq!(chunks.iter().sum::<usize>(), frames);
    let active: Vec<_> = rack.slots.iter().filter(|slot| !slot.bypass).collect();
    let layouts = vec![
        BufferSlotLayout {
            channels,
            capacity_frames: frames,
        };
        active.len() + 1
    ];
    let budget = layouts.len() * channels * frames * std::mem::size_of::<S>();
    let mut arena = BufferArena::<S>::new(&layouts, budget).unwrap();
    let input = arena
        .prepare_io(&[PortAccess::Write {
            port: 0,
            slot: arena.slot(0).unwrap(),
        }])
        .unwrap();
    arena
        .with_io(&input, 0..frames, |io| {
            let ProcessIo::Separate { mut outputs, .. } = io else {
                panic!("expected input seed buffer");
            };
            for (destination, source) in outputs.get_mut(0).unwrap().channels_mut().zip(samples) {
                for (destination, &source) in destination.iter_mut().zip(source) {
                    *destination = S::from_f64(source);
                }
            }
        })
        .unwrap();

    // Expand into distinct, adjacent buffer bindings, just as the compiler
    // must do. Separate IO also preserves arbitrary channel permutations.
    let mut kernels = Vec::new();
    let mut specs = Vec::new();
    for (index, slot) in active.iter().enumerate() {
        let id = ProcessorId(u64::from(slot.id) + 100);
        let (kernel, parameters) =
            prepare_slot::<S>(&slot.processor, channels, SAMPLE_RATE, id).unwrap();
        specs.extend(parameters);
        let io = arena
            .prepare_io(&[
                PortAccess::Read {
                    port: 0,
                    slot: arena.slot(index).unwrap(),
                },
                PortAccess::Write {
                    port: 0,
                    slot: arena.slot(index + 1).unwrap(),
                },
            ])
            .unwrap();
        kernel.validate_io(&io).unwrap();
        kernels.push((id, kernel, io));
    }
    let (_control, mut parameters) = parameter_channel(specs, 1, 1, 4, frames as u64 + 1).unwrap();
    let mut start = 0;
    for &count in chunks {
        for (id, kernel, io) in &mut kernels {
            let bindings = parameters.bindings(*id);
            let params = parameters.view(&bindings);
            kernel.validate_parameters(params).unwrap();
            arena
                .with_io(io, start..start + count, |io| {
                    kernel.process(
                        &ProcessContext {
                            timeline_epoch: 1,
                            timeline_start: start as u64,
                            frames: count,
                            processing_sr: SAMPLE_RATE,
                        },
                        io,
                        params,
                        crate::signal::ProcessSignals::empty(&mut ()),
                    );
                })
                .unwrap();
        }
        parameters.advance(count);
        start += count;
    }
    let output = arena
        .prepare_io(&[PortAccess::Read {
            port: 0,
            slot: arena.slot(active.len()).unwrap(),
        }])
        .unwrap();
    arena
        .with_io(&output, 0..frames, |io| {
            let ProcessIo::ReadOnly { inputs } = io else {
                panic!("expected chain output buffer");
            };
            inputs
                .get(0)
                .unwrap()
                .channels()
                .map(|samples| samples.iter().map(|sample| sample.to_f64()).collect())
                .collect()
        })
        .unwrap()
}

fn assert_near(actual: &[Vec<f64>], expected: &[Vec<f64>]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
        }
    }
}

#[test]
fn preparation_preserves_parameter_identity_and_pan_switches() {
    let processor = RackProcessor::Pan {
        initial: -0.5,
        mono: true,
        swap: true,
    };
    let (_, parameters) = prepare_slot::<f64>(&processor, 2, 48_000.0, PROCESSOR).unwrap();
    assert_eq!(parameters.len(), 3);
    for spec in &parameters {
        assert_eq!(spec.key.processor, PROCESSOR);
    }
    for (parameter, expected) in [
        (Pan::POSITION, ParamValue::Float(-0.5)),
        (Pan::MONO, ParamValue::Bool(true)),
        (Pan::SWAP, ParamValue::Bool(true)),
    ] {
        assert_eq!(
            parameters
                .iter()
                .find(|spec| spec.key.parameter == parameter)
                .unwrap()
                .initial,
            expected
        );
    }
    let (_, parameters) = prepare_slot::<f64>(
        &RackProcessor::Gain { initial: 0.25 },
        2,
        48_000.0,
        PROCESSOR,
    )
    .unwrap();
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0].key.parameter, Gain::LEVEL);
    assert_eq!(parameters[0].initial, ParamValue::Float(0.25));
}

#[test]
fn every_prepared_kernel_has_the_real_concrete_type_and_valid_parameters() {
    fn check<S: ProcessingSample>() {
        use crate::processor::{ChannelMapper, Compressor, Delay, Eq, Gate, Limiter};
        let types = [
            std::any::TypeId::of::<Gain>(),
            std::any::TypeId::of::<Pan>(),
            std::any::TypeId::of::<Compressor>(),
            std::any::TypeId::of::<Eq>(),
            std::any::TypeId::of::<Gate>(),
            std::any::TypeId::of::<Limiter>(),
            std::any::TypeId::of::<Delay>(),
            std::any::TypeId::of::<ChannelMapper>(),
        ];
        for (processor, expected_type) in processors().into_iter().zip(types) {
            let (kernel, specs) = prepare_slot::<S>(&processor, 2, SAMPLE_RATE, PROCESSOR).unwrap();
            assert_eq!(kernel.state_type_id(), expected_type);
            assert!(specs.iter().all(|spec| spec.key.processor == PROCESSOR));
            let (_control, parameters) = parameter_channel(specs, 1, 1, 1, 8).unwrap();
            let bindings = parameters.bindings(PROCESSOR);
            kernel
                .validate_parameters(parameters.view(&bindings))
                .unwrap();
            let samples = vec![vec![0.1, 0.2, -0.3], vec![0.2, -0.1, 0.4]];
            let output = render::<S>(&rack(processor), &samples, &[1, 2]);
            assert!(output.iter().flatten().all(|sample| sample.is_finite()));
        }
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn prepare_rejects_invalid_context_and_configuration_without_allocating_delay_state() {
    for processor in processors() {
        for channels in [0, 257, usize::MAX] {
            assert!(matches!(
                prepare_slot::<f64>(&processor, channels, SAMPLE_RATE, PROCESSOR),
                Err(ProcessorError::InvalidIo)
            ));
        }
        for sample_rate in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(
                prepare_slot::<f64>(&processor, 2, sample_rate, PROCESSOR),
                Err(ProcessorError::InvalidIo)
            ));
        }
    }
    assert!(matches!(
        prepare_slot::<f64>(
            &RackProcessor::Gain { initial: f64::NAN },
            2,
            SAMPLE_RATE,
            PROCESSOR
        ),
        Err(ProcessorError::InvalidParameterDomain)
    ));
    assert!(matches!(
        prepare_slot::<f64>(
            &RackProcessor::Pan {
                initial: 0.0,
                mono: false,
                swap: false
            },
            1,
            SAMPLE_RATE,
            PROCESSOR
        ),
        Err(ProcessorError::InvalidIo)
    ));
    for sample_rate in [f64::MAX, usize::MAX as f64] {
        assert!(matches!(
            prepare_slot::<f64>(
                &RackProcessor::Delay(DelaySettings::default()),
                2,
                sample_rate,
                PROCESSOR
            ),
            Err(ProcessorError::InvalidIo)
        ));
    }
}

#[test]
fn nonlinear_chain_obeys_slot_order_and_bypass_in_both_precisions() {
    fn check<S: ProcessingSample>() {
        let limiter = RackProcessor::Limiter(LimiterSettings {
            threshold_db: 0.0,
            release_ms: 0.0,
            input_gain_db: 0.0,
        });
        // Deliberately descending IDs: vector order defines the audio chain.
        let mut rack = RackSettings {
            slots: vec![
                slot(70, RackProcessor::Gain { initial: 2.0 }),
                slot(3, limiter),
            ],
        };
        let samples = vec![vec![0.75, -0.75, 0.25]];
        assert_near(
            &render::<S>(&rack, &samples, &[1, 2]),
            &[vec![1.0, -1.0, 0.5]],
        );
        rack.slots.reverse();
        assert_near(&render::<S>(&rack, &samples, &[3]), &[vec![1.5, -1.5, 0.5]]);
        rack.slots.reverse();
        rack.slots[1].bypass = true;
        assert_near(&render::<S>(&rack, &samples, &[3]), &[vec![1.5, -1.5, 0.5]]);
        rack.slots[0].bypass = true;
        assert_eq!(render::<S>(&rack, &samples, &[3]), samples);
        assert_eq!(
            render::<S>(&RackSettings::default(), &samples, &[3]),
            samples
        );
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn mapper_and_pan_form_a_real_stereo_chain() {
    fn check<S: ProcessingSample>() {
        let mut rack = RackSettings {
            slots: vec![
                slot(
                    1,
                    RackProcessor::ChannelMapper(ChannelMapperSettings {
                        mapping: vec![1, 0],
                        ..ChannelMapperSettings::default()
                    }),
                ),
                slot(
                    2,
                    RackProcessor::Pan {
                        initial: 0.5,
                        mono: false,
                        swap: false,
                    },
                ),
            ],
        };
        let samples = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        assert_near(
            &render::<S>(&rack, &samples, &[2]),
            &[vec![1.5, 2.0], vec![1.0, 2.0]],
        );
        rack.slots.reverse();
        assert_near(
            &render::<S>(&rack, &samples, &[2]),
            &[vec![3.0, 4.0], vec![0.5, 1.0]],
        );
        rack.slots[0].processor = RackProcessor::Pan {
            initial: 0.5,
            mono: true,
            swap: true,
        };
        assert_near(
            &render::<S>(&rack, &samples, &[1, 1]),
            &[vec![1.75, 2.5], vec![1.75, 2.5]],
        );
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn weighted_mapping_processes_after_gain_with_signed_and_silent_rows() {
    fn check<S: ProcessingSample>() {
        let rack = RackSettings {
            slots: vec![
                slot(1, RackProcessor::Gain { initial: 2.0 }),
                slot(
                    2,
                    RackProcessor::ChannelMapper(ChannelMapperSettings {
                        matrix: vec![vec![0.5, -0.25], vec![0.0, 0.0]],
                        ..ChannelMapperSettings::default()
                    }),
                ),
            ],
        };
        let samples = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        assert_near(
            &render::<S>(&rack, &samples, &[1, 1]),
            &[vec![-0.5, 0.0], vec![0.0, 0.0]],
        );
    }
    check::<f32>();
    check::<f64>();
}

#[test]
fn delay_state_is_preserved_across_chain_blocks() {
    fn check<S: ProcessingSample>() {
        let rack = RackSettings {
            slots: vec![
                slot(1, RackProcessor::Gain { initial: 2.0 }),
                slot(
                    2,
                    RackProcessor::Delay(DelaySettings {
                        delay_ms: 1.0,
                        feedback: 0.0,
                        mix: 1.0,
                        max_delay_ms: 4.0,
                    }),
                ),
                slot(3, RackProcessor::Gain { initial: 0.25 }),
            ],
        };
        let samples = vec![vec![1.0, 0.0, 0.5, 0.0]];
        let split = render::<S>(&rack, &samples, &[1, 2, 1]);
        assert_near(&split, &[vec![0.0, 0.5, 0.0, 0.25]]);
        assert_eq!(split, render::<S>(&rack, &samples, &[4]));
    }
    check::<f32>();
    check::<f64>();
}
