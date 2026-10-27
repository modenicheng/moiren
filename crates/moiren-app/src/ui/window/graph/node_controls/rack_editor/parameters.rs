//! Rack leaf projections reuse node parameter units, domains and scaling.

use super::super::parameter::{self, Parameter};
use super::model::Surface;
use crate::service::{
    BuiltinState, CompressorParameter, CompressorState, GraphEdit, GraphNodeState, RackParameter,
};
use moiren_core::graph::NodeKind;
use moiren_engine::{
    processor::ChannelMapperSettings,
    rack::{RackProcessor, RackSlot},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::ui::window::graph::node_controls) enum Field {
    Scalar(Parameter),
    Weight { input: usize, output: usize },
}

impl Field {
    pub(in crate::ui::window::graph::node_controls) fn label(self) -> &'static str {
        match self {
            Self::Scalar(parameter) => parameter.label(),
            Self::Weight { .. } => "Weight",
        }
    }

    pub(in crate::ui::window::graph::node_controls) fn unit(self) -> &'static str {
        match self {
            Self::Scalar(parameter) => parameter.unit(),
            Self::Weight { .. } => "",
        }
    }

    pub(in crate::ui::window::graph::node_controls) fn parse(self, text: &str) -> Option<f64> {
        parameter::parse(text, self.unit())
    }
}

pub(super) fn stage_node(owner: &GraphNodeState, slot: &RackSlot) -> (NodeKind, GraphNodeState) {
    if let Some(projection) = Projection::new(owner, &slot.processor) {
        return (projection.kind, projection.node);
    }
    let RackProcessor::ChannelMapper(settings) = &slot.processor else {
        unreachable!("all scalar leaves have a projection")
    };
    let mut node = leaf_node(owner);
    node.builtin = BuiltinState::ChannelMapper(settings.clone());
    node.source = None;
    (NodeKind::ChannelMapper, node)
}

pub(super) fn fields(kind: NodeKind, node: &GraphNodeState, surface: Surface) -> Vec<Field> {
    let parameters = if kind == NodeKind::Compressor && surface == Surface::Inspector {
        CompressorParameter::ALL
            .into_iter()
            .map(Parameter::Compressor)
            .collect()
    } else {
        Parameter::primary(kind, node)
    };
    parameters.into_iter().map(Field::Scalar).collect()
}

fn leaf_node(owner: &GraphNodeState) -> GraphNodeState {
    // Do not clone the enclosing Rack (potentially many large matrices) merely
    // to read one scalar. This projection never carries graph ownership.
    GraphNodeState {
        id: owner.id,
        label: owner.label.clone(),
        gain: 1.,
        pan: 0.,
        mono: false,
        swap: false,
        mixer: Default::default(),
        compressor: Default::default(),
        builtin: BuiltinState::None,
        source: None,
        output_endpoint_id: None,
    }
}

/// A disposable leaf view, never a second graph node. Its GraphEdit values are
/// inspected for conversion only; none may be dispatched with the Rack NodeId.
pub(super) struct Projection {
    kind: NodeKind,
    node: GraphNodeState,
}

impl Projection {
    pub(super) fn new(owner: &GraphNodeState, processor: &RackProcessor) -> Option<Self> {
        let mut node = leaf_node(owner);
        let kind = match processor {
            RackProcessor::Gain { initial } => {
                node.gain = *initial;
                NodeKind::Gain
            }
            RackProcessor::Pan {
                initial,
                mono,
                swap,
            } => {
                node.pan = *initial;
                node.mono = *mono;
                node.swap = *swap;
                NodeKind::Pan
            }
            RackProcessor::Compressor(settings) => {
                node.compressor = CompressorState {
                    input_gain_db: settings.input_gain_db,
                    threshold_db: settings.threshold_db,
                    ratio: settings.ratio,
                    attack_ms: settings.attack_ms,
                    release_ms: settings.release_ms,
                    hold_ms: settings.hold_ms,
                    knee_db: settings.knee_db,
                    makeup_gain_db: settings.makeup_gain_db,
                    output_gain_db: settings.output_gain_db,
                    mix: settings.mix,
                };
                NodeKind::Compressor
            }
            RackProcessor::Eq(settings) => {
                node.builtin = BuiltinState::Eq(*settings);
                NodeKind::Eq
            }
            RackProcessor::Gate(settings) => {
                node.builtin = BuiltinState::Gate(*settings);
                NodeKind::Gate
            }
            RackProcessor::Limiter(settings) => {
                node.builtin = BuiltinState::Limiter(*settings);
                NodeKind::Limiter
            }
            RackProcessor::Delay(settings) => {
                node.builtin = BuiltinState::Delay(*settings);
                NodeKind::Delay
            }
            RackProcessor::ChannelMapper(_) => return None,
        };
        Some(Self { kind, node })
    }

    pub(super) fn fields(&self, surface: Surface) -> Vec<Field> {
        fields(self.kind, &self.node, surface)
    }

    fn parameter(&self, field: Field) -> Option<Parameter> {
        let Field::Scalar(parameter) = field else {
            return None;
        };
        self.fields(Surface::Inspector)
            .contains(&field)
            .then_some(parameter)
    }

    pub(super) fn value(&self, field: Field) -> Option<f64> {
        Some(self.parameter(field)?.value(self.kind, &self.node))
    }

    pub(super) fn range(&self, field: Field) -> Option<(f64, f64)> {
        Some(self.parameter(field)?.range(&self.node))
    }

    pub(super) fn position(&self, field: Field) -> Option<f32> {
        let position = self.parameter(field)?.position(self.kind, &self.node);
        // A saved zero-capacity Delay has a valid constant time domain.
        Some(if position.is_finite() { position } else { 0. })
    }

    pub(super) fn value_at_position(&self, field: Field, position: f32) -> Option<f64> {
        if !position.is_finite() {
            return None;
        }
        let parameter = self.parameter(field)?;
        let (min, max) = parameter.range(&self.node);
        // Logarithmic interpolation can overshoot by an ULP at full scale.
        // Only dial positions use exact endpoints; typed values stay validated.
        Some(if position >= 1. {
            max
        } else if position <= 0. && parameter != Parameter::Level {
            min
        } else {
            parameter.value_at_position(position, &self.node)
        })
    }

    pub(super) fn reset(&self, field: Field) -> Option<f32> {
        let position = self
            .parameter(field)?
            .default_position(self.kind, &self.node);
        Some(if position.is_finite() { position } else { 0. })
    }

    /// Translate a single leaf value to the engine's typed parameter contract.
    /// This deliberately never returns a replacement RackSettings payload.
    pub(super) fn scalar_intent(&self, field: Field, value: f64) -> Option<(RackParameter, f64)> {
        let parameter = self.parameter(field)?;
        let edit = parameter.edit(value, self.kind, &self.node)?;
        let (id, value) = match edit {
            GraphEdit::SetGain { gain, .. } => (RackParameter::Gain, gain),
            GraphEdit::SetPan { pan, .. } => (RackParameter::Pan, pan),
            GraphEdit::SetCompressor {
                parameter, value, ..
            } => (RackParameter::Compressor(parameter), value),
            GraphEdit::SetBuiltin { state, .. } => match (parameter, state) {
                (Parameter::EqFrequency, BuiltinState::Eq(s)) => {
                    (RackParameter::EqFrequency, s.frequency_hz)
                }
                (Parameter::EqGain, BuiltinState::Eq(s)) => (RackParameter::EqGain, s.gain_db),
                (Parameter::EqQ, BuiltinState::Eq(s)) => (RackParameter::EqQ, s.q),
                (Parameter::GateThreshold, BuiltinState::Gate(s)) => {
                    (RackParameter::GateThreshold, s.threshold_db)
                }
                (Parameter::GateRange, BuiltinState::Gate(s)) => {
                    (RackParameter::GateRange, s.range_db)
                }
                (Parameter::GateAttack, BuiltinState::Gate(s)) => {
                    (RackParameter::GateAttack, s.attack_ms)
                }
                (Parameter::GateHold, BuiltinState::Gate(s)) => {
                    (RackParameter::GateHold, s.hold_ms)
                }
                (Parameter::GateRelease, BuiltinState::Gate(s)) => {
                    (RackParameter::GateRelease, s.release_ms)
                }
                (Parameter::LimitInput, BuiltinState::Limiter(s)) => {
                    (RackParameter::LimitInput, s.input_gain_db)
                }
                (Parameter::LimitThreshold, BuiltinState::Limiter(s)) => {
                    (RackParameter::LimitThreshold, s.threshold_db)
                }
                (Parameter::LimitRelease, BuiltinState::Limiter(s)) => {
                    (RackParameter::LimitRelease, s.release_ms)
                }
                (Parameter::DelayTime, BuiltinState::Delay(s)) => {
                    (RackParameter::DelayTime, s.delay_ms)
                }
                (Parameter::DelayFeedback, BuiltinState::Delay(s)) => {
                    // The UI value was validated above. Dividing its exact
                    // +/-99.9% endpoints by 100 otherwise exceeds +/-0.999.
                    let feedback = if value == -99.9 {
                        -0.999
                    } else if value == 99.9 {
                        0.999
                    } else {
                        s.feedback
                    };
                    (RackParameter::DelayFeedback, feedback)
                }
                (Parameter::DelayMix, BuiltinState::Delay(s)) => (RackParameter::DelayMix, s.mix),
                _ => return None,
            },
            _ => return None,
        };
        Some((id, value))
    }
}

pub(super) const PAGE_SIZE: usize = 4;

/// Pages are zero-based (output row page, input column page), not cell offsets.
pub(super) fn weight_fields(
    inputs: usize,
    outputs: usize,
    page: (usize, usize),
) -> ((usize, usize), Vec<Field>) {
    if inputs == 0 || outputs == 0 || inputs > 256 || outputs > 256 {
        return ((0, 0), Vec::new());
    }
    let page = (
        page.0.min((outputs - 1) / PAGE_SIZE),
        page.1.min((inputs - 1) / PAGE_SIZE),
    );
    let output = page.0 * PAGE_SIZE;
    let input = page.1 * PAGE_SIZE;
    let fields = (output..(output + PAGE_SIZE).min(outputs))
        .flat_map(|output| {
            (input..(input + PAGE_SIZE).min(inputs))
                .map(move |input| Field::Weight { input, output })
        })
        .collect();
    (page, fields)
}

pub(super) fn weight(
    settings: &ChannelMapperSettings,
    inputs: usize,
    outputs: usize,
    input: usize,
    output: usize,
) -> Option<f64> {
    if input >= inputs || output >= outputs || inputs > 256 || outputs > 256 {
        return None;
    }
    if !settings.matrix.is_empty() {
        if !settings.mapping.is_empty() || settings.matrix.len() != outputs {
            return None;
        }
        let row = settings.matrix.get(output)?;
        let value = *row.get(input)?;
        return (row.len() == inputs && value.is_finite()).then_some(value);
    }
    if (settings.mapping.is_empty() && inputs != outputs)
        || (!settings.mapping.is_empty() && settings.mapping.len() != outputs)
    {
        return None;
    }
    // resolve_layout already validates the whole Rack. Read only this cell,
    // rather than rescan a 256x256 matrix for each of the page's 16 fields.
    let source = settings.mapping.get(output).copied().unwrap_or(output);
    (source < inputs).then_some(f64::from(u8::from(source == input)))
}

pub(super) fn valid_weight(value: f64) -> bool {
    // The engine accepts finite signed weights, not an arbitrary +/-1 UI range.
    ChannelMapperSettings {
        mapping: Vec::new(),
        matrix: vec![vec![value]],
    }
    .valid_layout(1, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::RackEdit;
    use moiren_engine::processor::{ChannelMapperSettings, CompressorSettings, DelaySettings};
    use moiren_engine::rack::RackSettings;

    fn owner() -> GraphNodeState {
        crate::service::ServiceGraph::default().snapshot().nodes[0].clone()
    }

    fn engine_valid_intent(
        processor: RackProcessor,
        field: Field,
        value: f64,
    ) -> (RackParameter, f64) {
        let projection = Projection::new(&owner(), &processor).unwrap();
        let (parameter, value) = projection
            .scalar_intent(field, value)
            .unwrap_or_else(|| panic!("{field:?}: adapter rejected UI endpoint {value:?}"));
        let settings = RackSettings {
            slots: vec![RackSlot {
                id: 7,
                bypass: false,
                processor,
            }],
            ..Default::default()
        };
        let edit = RackEdit::SetParameter {
            slot: 7,
            parameter,
            value,
        };
        let preview = edit.preview(&settings, &[2], &[2]);
        assert!(
            preview.is_ok(),
            "{field:?}: engine rejected converted endpoint {value:?}: {preview:?}"
        );
        (parameter, value)
    }

    fn assert_feedback_endpoint(text: &str, expected: f64) {
        let field = Field::Scalar(Parameter::DelayFeedback);
        let value = field.parse(text).expect("valid percentage input");
        assert_eq!(
            engine_valid_intent(RackProcessor::Delay(Default::default()), field, value),
            (RackParameter::DelayFeedback, expected),
        );
    }

    #[test]
    fn rack_adapter_endpoint_feedback_negative_is_engine_valid() {
        assert_feedback_endpoint("-99.9 %", -0.999);
    }

    #[test]
    fn rack_adapter_endpoint_feedback_positive_is_engine_valid() {
        assert_feedback_endpoint("99.9 %", 0.999);
    }

    #[test]
    fn rack_adapter_endpoint_logarithmic_maxima_are_exact_and_engine_valid() {
        for (processor, parameter) in [
            (
                RackProcessor::Eq(Default::default()),
                Parameter::EqFrequency,
            ),
            (RackProcessor::Eq(Default::default()), Parameter::EqQ),
            (
                RackProcessor::Gate(Default::default()),
                Parameter::GateAttack,
            ),
            (RackProcessor::Gate(Default::default()), Parameter::GateHold),
            (
                RackProcessor::Gate(Default::default()),
                Parameter::GateRelease,
            ),
            (
                RackProcessor::Limiter(Default::default()),
                Parameter::LimitRelease,
            ),
            (
                RackProcessor::Compressor(Default::default()),
                Parameter::Compressor(CompressorParameter::Ratio),
            ),
            (
                RackProcessor::Compressor(Default::default()),
                Parameter::Compressor(CompressorParameter::Attack),
            ),
            (
                RackProcessor::Compressor(Default::default()),
                Parameter::Compressor(CompressorParameter::Release),
            ),
            (
                RackProcessor::Delay(Default::default()),
                Parameter::DelayTime,
            ),
        ] {
            let field = Field::Scalar(parameter);
            let projection = Projection::new(&owner(), &processor).unwrap();
            let maximum = projection.range(field).unwrap().1;
            let value = projection.value_at_position(field, 1.).unwrap();
            assert_eq!(
                value, maximum,
                "{field:?}: full-scale dial must land on the exact domain endpoint"
            );
            engine_valid_intent(processor, field, value);
        }
    }

    #[test]
    fn rack_adapter_endpoint_short_delay_reset_is_exact_and_engine_valid() {
        let processor = RackProcessor::Delay(DelaySettings {
            max_delay_ms: 80.,
            delay_ms: 20.,
            ..Default::default()
        });
        let field = Field::Scalar(Parameter::DelayTime);
        let projection = Projection::new(&owner(), &processor).unwrap();
        let reset = projection.reset(field).unwrap();
        assert_eq!(
            reset, 1.,
            "default delay must stop at this stage's 80 ms capacity"
        );
        let value = projection.value_at_position(field, reset).unwrap();
        assert_eq!(
            value, 80.,
            "reset must not overshoot the prepared delay capacity"
        );
        assert_eq!(
            engine_valid_intent(processor, field, value),
            (RackParameter::DelayTime, 80.)
        );
    }

    #[test]
    fn rack_adapter_endpoint_normalization_does_not_clamp_invalid_text() {
        for (processor, parameter, text) in [
            (
                RackProcessor::Delay(Default::default()),
                Parameter::DelayFeedback,
                "99.90000000000002 %",
            ),
            (
                RackProcessor::Delay(Default::default()),
                Parameter::DelayFeedback,
                "-99.90000000000002 %",
            ),
            (
                RackProcessor::Delay(Default::default()),
                Parameter::DelayFeedback,
                "100 %",
            ),
            (
                RackProcessor::Delay(Default::default()),
                Parameter::DelayFeedback,
                "-100 %",
            ),
            (
                RackProcessor::Eq(Default::default()),
                Parameter::EqFrequency,
                "20000.000000000015 Hz",
            ),
            (
                RackProcessor::Gate(Default::default()),
                Parameter::GateRelease,
                "5000.000000000003 ms",
            ),
            (
                RackProcessor::Delay(DelaySettings {
                    max_delay_ms: 80.,
                    delay_ms: 20.,
                    ..Default::default()
                }),
                Parameter::DelayTime,
                "80.00000000000003 ms",
            ),
        ] {
            let field = Field::Scalar(parameter);
            let projection = Projection::new(&owner(), &processor).unwrap();
            let value = field.parse(text).expect("numeric text with valid units");
            let (min, max) = projection.range(field).unwrap();
            assert!(
                value < min || value > max,
                "{text} must be outside the f64 domain"
            );
            assert!(
                projection.scalar_intent(field, value).is_none(),
                "{field:?}: out-of-range text must not be clamped: {text}"
            );
        }
    }

    #[test]
    fn rack_adapter_endpoint_normalization_preserves_interior_precision_and_silence() {
        let processor = RackProcessor::Delay(Default::default());
        let projection = Projection::new(&owner(), &processor).unwrap();
        let field = Field::Scalar(Parameter::DelayFeedback);
        for value in [
            -99.89999999999999,
            -12.3456789012345,
            12.3456789012345,
            99.89999999999999,
        ] {
            assert_eq!(
                engine_valid_intent(processor.clone(), field, value),
                (RackParameter::DelayFeedback, value / 100.)
            );
        }
        for position in [0.125, 0.5, 0.875] {
            assert_eq!(
                projection.value_at_position(Field::Scalar(Parameter::DelayTime), position),
                Some(Parameter::DelayTime.value_at_position(position, &projection.node))
            );
        }
        for position in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            assert!(projection.value_at_position(field, position).is_none());
        }
        for value in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
            assert!(projection.scalar_intent(field, value).is_none());
        }
        let gain = Projection::new(&owner(), &RackProcessor::Gain { initial: 1. }).unwrap();
        assert_eq!(
            gain.value_at_position(Field::Scalar(Parameter::Level), 0.),
            Some(f64::NEG_INFINITY)
        );
        let finite_floor = gain
            .scalar_intent(Field::Scalar(Parameter::Level), -72.)
            .unwrap();
        assert_eq!(finite_floor.0, RackParameter::Gain);
        assert!(finite_floor.1 > 0.);
    }

    #[test]
    fn rack_adapter_endpoint_feedback_accepts_equivalent_f64_text() {
        let field = Field::Scalar(Parameter::DelayFeedback);
        for (text, endpoint, expected) in [
            ("99.90000000000001 %", 99.9_f64, 0.999),
            ("-99.90000000000001 %", -99.9_f64, -0.999),
        ] {
            let value = field.parse(text).unwrap();
            assert_eq!(value.to_bits(), endpoint.to_bits());
            assert_eq!(
                engine_valid_intent(RackProcessor::Delay(Default::default()), field, value),
                (RackParameter::DelayFeedback, expected)
            );
        }
        let next = field.parse("99.90000000000002 %").unwrap();
        assert_eq!(next.to_bits(), 99.9_f64.to_bits() + 1);
    }

    #[test]
    fn scalar_projection_covers_seven_leaves_and_all_inspector_compressor_fields() {
        let owner = owner();
        for (processor, count) in [
            (RackProcessor::Gain { initial: 1. }, 1),
            (
                RackProcessor::Pan {
                    initial: 0.,
                    mono: true,
                    swap: true,
                },
                1,
            ),
            (RackProcessor::Eq(Default::default()), 3),
            (RackProcessor::Compressor(Default::default()), 6),
            (RackProcessor::Gate(Default::default()), 5),
            (RackProcessor::Limiter(Default::default()), 3),
            (RackProcessor::Delay(Default::default()), 3),
        ] {
            let projection = Projection::new(&owner, &processor).unwrap();
            let fields = projection.fields(Surface::Card);
            assert_eq!(fields.len(), count);
            for field in fields {
                let value = projection.value(field).unwrap();
                assert!(
                    projection.scalar_intent(field, value).is_some(),
                    "{field:?}"
                );
                assert!(projection.scalar_intent(field, f64::NAN).is_none());
                let position = projection.position(field).unwrap();
                let restored = projection.value_at_position(field, position).unwrap();
                assert!((restored - value).abs() < 0.01, "{field:?}");
            }
        }
        let projection = Projection::new(
            &owner,
            &RackProcessor::Compressor(CompressorSettings::default()),
        )
        .unwrap();
        assert_eq!(projection.fields(Surface::Inspector).len(), 10);
        assert!(
            projection
                .fields(Surface::Inspector)
                .contains(&Field::Scalar(Parameter::Compressor(
                    CompressorParameter::Hold
                )))
        );
        assert!(
            Projection::new(&owner, &RackProcessor::ChannelMapper(Default::default())).is_none()
        );
    }

    #[test]
    fn scalar_intents_use_leaf_units_not_node_or_percentage_units() {
        let owner = owner();
        let gain = Projection::new(&owner, &RackProcessor::Gain { initial: 1. }).unwrap();
        assert_eq!(
            gain.scalar_intent(Field::Scalar(Parameter::Level), f64::NEG_INFINITY),
            Some((RackParameter::Gain, 0.))
        );
        let Some((_, quiet)) = gain.scalar_intent(Field::Scalar(Parameter::Level), -72.) else {
            panic!("finite gain intent")
        };
        assert!(quiet > 0.);
        assert!(
            gain.scalar_intent(Field::Scalar(Parameter::EqGain), 3.)
                .is_none()
        );
        let pan = Projection::new(
            &owner,
            &RackProcessor::Pan {
                initial: 0.,
                mono: true,
                swap: true,
            },
        )
        .unwrap();
        assert_eq!(
            pan.scalar_intent(Field::Scalar(Parameter::Balance), -25.),
            Some((RackParameter::Pan, -0.25))
        );
        let compressor =
            Projection::new(&owner, &RackProcessor::Compressor(Default::default())).unwrap();
        assert_eq!(
            compressor.scalar_intent(
                Field::Scalar(Parameter::Compressor(CompressorParameter::Mix)),
                25.
            ),
            Some((RackParameter::Compressor(CompressorParameter::Mix), 0.25))
        );
    }

    #[test]
    fn delay_projection_preserves_capacity_and_rejects_overflow() {
        let owner = owner();
        let processor = RackProcessor::Delay(DelaySettings {
            max_delay_ms: 80.,
            delay_ms: 20.,
            feedback: -0.5,
            mix: 0.25,
        });
        let projection = Projection::new(&owner, &processor).unwrap();
        let field = Field::Scalar(Parameter::DelayTime);
        assert_eq!(projection.range(field), Some((0., 80.)));
        assert!(projection.scalar_intent(field, 81.).is_none());
        assert!(
            (projection
                .value_at_position(field, projection.reset(field).unwrap())
                .unwrap()
                - 80.)
                .abs()
                < 1e-6
        );
        assert_eq!(
            projection.scalar_intent(Field::Scalar(Parameter::DelayFeedback), -50.),
            Some((RackParameter::DelayFeedback, -0.5))
        );
    }

    #[test]
    fn mapper_reads_identity_mapping_and_signed_weights_without_normalizing() {
        let settings = ChannelMapperSettings::default();
        assert_eq!(weight(&settings, 2, 2, 0, 0), Some(1.));
        assert_eq!(weight(&settings, 2, 2, 1, 0), Some(0.));
        let settings = ChannelMapperSettings {
            mapping: vec![1, 1],
            matrix: Vec::new(),
        };
        assert_eq!(weight(&settings, 2, 2, 1, 0), Some(1.));
        let settings = ChannelMapperSettings {
            mapping: Vec::new(),
            matrix: vec![vec![-2.5, 3.75], vec![0., 1.]],
        };
        assert_eq!(weight(&settings, 2, 2, 0, 0), Some(-2.5));
        assert_eq!(weight(&settings, 2, 2, 1, 0), Some(3.75));
        assert_eq!(weight(&settings, 2, 2, 2, 0), None);
        assert!(valid_weight(-2.5));
        assert!(valid_weight(3.75));
        assert!(!valid_weight(f64::INFINITY));
        assert!(!valid_weight(f64::NAN));
    }

    #[test]
    fn mapper_pages_reach_all_channels_with_at_most_sixteen_fields() {
        let mut visited = std::collections::BTreeSet::new();
        for output in 0..64 {
            for input in 0..64 {
                let (page, fields) = weight_fields(256, 256, (output, input));
                assert_eq!(page, (output, input));
                assert_eq!(fields.len(), 16);
                visited.extend(fields);
            }
        }
        assert_eq!(visited.len(), 256 * 256);
        let (page, fields) = weight_fields(9, 10, (usize::MAX, usize::MAX));
        assert_eq!(page, (2, 2));
        assert_eq!(
            fields,
            vec![
                Field::Weight {
                    input: 8,
                    output: 8
                },
                Field::Weight {
                    input: 8,
                    output: 9
                }
            ]
        );
        assert!(weight_fields(0, 2, (0, 0)).1.is_empty());
    }
}
