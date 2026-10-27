//! Shared parameters with separate gesture travel and exact-entry domains.
use crate::service::{
    BuiltinState, CompressorParameter, GraphEdit, GraphNodeState, InputSelection,
};
use moiren_core::graph::NodeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Parameter {
    Level,
    Balance,
    Frequency,
    Compressor(CompressorParameter),
    EqFrequency,
    EqGain,
    EqQ,
    GateThreshold,
    GateRange,
    GateAttack,
    GateHold,
    GateRelease,
    LimitInput,
    LimitThreshold,
    LimitRelease,
    DelayTime,
    DelayFeedback,
    DelayMix,
}

impl Parameter {
    pub fn primary(kind: NodeKind, node: &GraphNodeState) -> Vec<Self> {
        use CompressorParameter as C;
        use Parameter::*;
        match kind {
            NodeKind::Source if matches!(node.source, Some(InputSelection::Tone { .. })) => {
                vec![Frequency, Level]
            }
            NodeKind::Source | NodeKind::Sink | NodeKind::Gain | NodeKind::Bus => vec![Level],
            NodeKind::Pan => vec![Balance],
            NodeKind::Compressor => [
                C::Threshold,
                C::Ratio,
                C::Attack,
                C::Release,
                C::MakeupGain,
                C::Mix,
            ]
            .map(Compressor)
            .to_vec(),
            NodeKind::Eq => vec![EqFrequency, EqGain, EqQ],
            NodeKind::Gate => vec![GateThreshold, GateRange, GateAttack, GateHold, GateRelease],
            NodeKind::Limiter => vec![LimitInput, LimitThreshold, LimitRelease],
            NodeKind::Delay => vec![DelayTime, DelayFeedback, DelayMix],
            _ => Vec::new(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Level => "Level",
            Self::Balance => "Balance",
            Self::Frequency | Self::EqFrequency => "Frequency",
            Self::Compressor(p) => p.label(),
            Self::EqGain => "Band gain",
            Self::EqQ => "Q",
            Self::GateThreshold | Self::LimitThreshold => "Threshold",
            Self::GateRange => "Range",
            Self::GateAttack => "Attack",
            Self::GateHold => "Hold",
            Self::GateRelease | Self::LimitRelease => "Release",
            Self::LimitInput => "Input gain",
            Self::DelayTime => "Time",
            Self::DelayFeedback => "Feedback",
            Self::DelayMix => "Wet mix",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::Frequency | Self::EqFrequency => "Hz",
            Self::EqQ => "Q",
            Self::Balance | Self::DelayFeedback | Self::DelayMix => "%",
            Self::GateAttack
            | Self::GateHold
            | Self::GateRelease
            | Self::LimitRelease
            | Self::DelayTime => "ms",
            Self::Compressor(p) => match p {
                CompressorParameter::Ratio => ":1",
                CompressorParameter::Attack
                | CompressorParameter::Release
                | CompressorParameter::Hold => "ms",
                CompressorParameter::Mix => "%",
                _ => "dB",
            },
            _ => "dB",
        }
    }

    pub fn range(self, node: &GraphNodeState) -> (f64, f64) {
        match self {
            Self::Level => (-72., 24.082399653118497),
            Self::Balance => (-100., 100.),
            Self::Frequency => (20., 20_000.),
            Self::EqFrequency => (20., 20_000.),
            Self::EqGain => (-24., 24.),
            Self::EqQ => (0.1, 20.),
            Self::GateThreshold => (-120., 0.),
            Self::GateRange => (-120., 0.),
            Self::GateAttack | Self::GateHold | Self::GateRelease | Self::LimitRelease => {
                (0., 5000.)
            }
            Self::LimitInput => (-60., 60.),
            Self::LimitThreshold => (-60., 0.),
            Self::DelayTime => (0., node.builtin.delay().map_or(2000., |s| s.max_delay_ms)),
            Self::DelayFeedback => (-99.9, 99.9),
            Self::DelayMix => (0., 100.),
            Self::Compressor(CompressorParameter::Mix) => (0., 100.),
            Self::Compressor(p) => p.range(),
        }
    }

    pub fn value(self, kind: NodeKind, node: &GraphNodeState) -> f64 {
        match self {
            Self::Level => {
                let gain = if kind == NodeKind::Gain {
                    node.gain
                } else {
                    node.mixer.gain
                };
                if gain <= 0. {
                    f64::NEG_INFINITY
                } else {
                    20. * gain.log10()
                }
            }
            Self::Balance => node.pan * 100.,
            Self::Frequency => match node.source {
                Some(InputSelection::Tone { frequency_hz }) => frequency_hz,
                _ => 440.,
            },
            Self::Compressor(p) => {
                node.compressor.value(p)
                    * if p == CompressorParameter::Mix {
                        100.
                    } else {
                        1.
                    }
            }
            Self::EqFrequency => node.builtin.eq().map_or(1000., |s| s.frequency_hz),
            Self::EqGain => node.builtin.eq().map_or(0., |s| s.gain_db),
            Self::EqQ => node.builtin.eq().map_or(1., |s| s.q),
            Self::GateThreshold => node.builtin.gate().map_or(-40., |s| s.threshold_db),
            Self::GateRange => node.builtin.gate().map_or(-80., |s| s.range_db),
            Self::GateAttack => node.builtin.gate().map_or(1., |s| s.attack_ms),
            Self::GateHold => node.builtin.gate().map_or(10., |s| s.hold_ms),
            Self::GateRelease => node.builtin.gate().map_or(100., |s| s.release_ms),
            Self::LimitInput => node.builtin.limiter().map_or(0., |s| s.input_gain_db),
            Self::LimitThreshold => node.builtin.limiter().map_or(-1., |s| s.threshold_db),
            Self::LimitRelease => node.builtin.limiter().map_or(50., |s| s.release_ms),
            Self::DelayTime => node.builtin.delay().map_or(250., |s| s.delay_ms),
            Self::DelayFeedback => node.builtin.delay().map_or(0., |s| s.feedback * 100.),
            Self::DelayMix => node.builtin.delay().map_or(100., |s| s.mix * 100.),
        }
    }

    fn logarithmic(self) -> bool {
        matches!(
            self,
            Self::Frequency
                | Self::EqFrequency
                | Self::EqQ
                | Self::GateAttack
                | Self::GateHold
                | Self::GateRelease
                | Self::LimitRelease
                | Self::DelayTime
                | Self::Compressor(
                    CompressorParameter::Attack
                        | CompressorParameter::Release
                        | CompressorParameter::Ratio
                )
        )
    }

    pub fn position(self, kind: NodeKind, node: &GraphNodeState) -> f32 {
        let (min, max) = self.range(node);
        let value = self.value(kind, node).clamp(min, max) - min;
        if self.logarithmic() {
            (value.ln_1p() / (max - min).ln_1p()) as f32
        } else {
            (value / (max - min).max(f64::EPSILON)) as f32
        }
    }

    pub fn value_at_position(self, position: f32, node: &GraphNodeState) -> f64 {
        if self == Self::Level && position <= 0. {
            return f64::NEG_INFINITY;
        }
        let (min, max) = self.range(node);
        let t = f64::from(position.clamp(0., 1.));
        if self.logarithmic() {
            min + (t * (max - min).ln_1p()).exp_m1()
        } else {
            min + t * (max - min)
        }
    }

    pub fn default_position(self, kind: NodeKind, node: &GraphNodeState) -> f32 {
        let mut defaults = node.clone();
        defaults.gain = 1.;
        defaults.pan = 0.;
        defaults.mixer = Default::default();
        defaults.compressor = Default::default();
        defaults.builtin = BuiltinState::for_kind(kind);
        if let (Some(actual), BuiltinState::Delay(default)) =
            (node.builtin.delay(), &mut defaults.builtin)
        {
            default.max_delay_ms = actual.max_delay_ms;
            default.delay_ms = default.delay_ms.min(actual.max_delay_ms);
        }
        if self == Self::Frequency {
            defaults.source = Some(InputSelection::Tone { frequency_hz: 440. });
        }
        self.position(kind, &defaults)
    }

    pub fn edit(self, value: f64, kind: NodeKind, node: &GraphNodeState) -> Option<GraphEdit> {
        let (min, max) = self.range(node);
        let silence = self == Self::Level && value == f64::NEG_INFINITY;
        if !silence && (!value.is_finite() || value < min || value > max) {
            return None;
        }
        self.edit_validated(value, kind, node)
    }

    /// Precision entry follows the service/Inspector contract, not the dial's
    /// ergonomic travel. In particular, the existing 16x gesture endpoint stays
    /// valid while typed Level values stop at +24 dB.
    pub fn edit_precision(
        self,
        value: f64,
        kind: NodeKind,
        node: &GraphNodeState,
    ) -> Option<GraphEdit> {
        match self {
            Self::Frequency => {
                if kind != NodeKind::Source
                    || !matches!(node.source, Some(InputSelection::Tone { .. }))
                    || (InputSelection::Tone {
                        frequency_hz: value,
                    })
                    .validate()
                    .is_err()
                {
                    return None;
                }
            }
            Self::Level => {
                if value != f64::NEG_INFINITY
                    && (!value.is_finite() || !(-120. ..=24.).contains(&value))
                {
                    return None;
                }
            }
            _ => return self.edit(value, kind, node),
        }
        self.edit_validated(value, kind, node)
    }

    pub fn precision_error(self, node: &GraphNodeState) -> String {
        match self {
            Self::Frequency => "0 < Frequency < 24000 Hz".into(),
            Self::Level => "-120 .. 24 dB, or -inf".into(),
            _ => {
                let (min, max) = self.range(node);
                format!("{min:.1} .. {max:.1} {}", self.unit())
            }
        }
    }

    fn edit_validated(
        self,
        value: f64,
        kind: NodeKind,
        node: &GraphNodeState,
    ) -> Option<GraphEdit> {
        let silence = self == Self::Level && value == f64::NEG_INFINITY;
        let id = node.id;
        let mut builtin = node.builtin.clone();
        match self {
            Self::Level => {
                let gain = if silence { 0. } else { 10f64.powf(value / 20.) }.min(16.);
                return Some(if kind == NodeKind::Gain {
                    GraphEdit::SetGain { node: id, gain }
                } else {
                    let mut state = node.mixer;
                    state.gain = gain;
                    GraphEdit::SetMixer { node: id, state }
                });
            }
            Self::Balance => {
                return Some(GraphEdit::SetPan {
                    node: id,
                    pan: value / 100.,
                });
            }
            Self::Frequency => {
                return Some(GraphEdit::BindSource {
                    node: id,
                    source: InputSelection::Tone {
                        frequency_hz: value,
                    },
                });
            }
            Self::Compressor(parameter) => {
                return Some(GraphEdit::SetCompressor {
                    node: id,
                    parameter,
                    value: value
                        / if parameter == CompressorParameter::Mix {
                            100.
                        } else {
                            1.
                        },
                });
            }
            Self::EqFrequency | Self::EqGain | Self::EqQ => {
                if let BuiltinState::Eq(s) = &mut builtin {
                    match self {
                        Self::EqFrequency => s.frequency_hz = value,
                        Self::EqGain => s.gain_db = value,
                        _ => s.q = value,
                    }
                } else {
                    return None;
                }
            }
            Self::GateThreshold
            | Self::GateRange
            | Self::GateAttack
            | Self::GateHold
            | Self::GateRelease => {
                if let BuiltinState::Gate(s) = &mut builtin {
                    match self {
                        Self::GateThreshold => s.threshold_db = value,
                        Self::GateRange => s.range_db = value,
                        Self::GateAttack => s.attack_ms = value,
                        Self::GateHold => s.hold_ms = value,
                        _ => s.release_ms = value,
                    }
                } else {
                    return None;
                }
            }
            Self::LimitInput | Self::LimitThreshold | Self::LimitRelease => {
                if let BuiltinState::Limiter(s) = &mut builtin {
                    match self {
                        Self::LimitInput => s.input_gain_db = value,
                        Self::LimitThreshold => s.threshold_db = value,
                        _ => s.release_ms = value,
                    }
                } else {
                    return None;
                }
            }
            Self::DelayTime | Self::DelayFeedback | Self::DelayMix => {
                if let BuiltinState::Delay(s) = &mut builtin {
                    match self {
                        Self::DelayTime => s.delay_ms = value,
                        Self::DelayFeedback => {
                            // Validation already rejected out-of-domain text.
                            // Exact UI endpoints otherwise divide one ULP past
                            // the engine's inclusive +/-0.999 feedback bound.
                            s.feedback = if value == -99.9 {
                                -0.999
                            } else if value == 99.9 {
                                0.999
                            } else {
                                value / 100.
                            };
                        }
                        _ => s.mix = value / 100.,
                    }
                } else {
                    return None;
                }
            }
        }
        Some(GraphEdit::SetBuiltin {
            node: id,
            state: builtin,
        })
    }
}

/// Suffixes are deliberately field-specific; invalid text never becomes zero.
pub(super) fn parse(text: &str, unit: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches(unit).trim();
    if unit == "dB" && matches!(text, "-inf" | "-∞" | "−∞") {
        return Some(f64::NEG_INFINITY);
    }
    let (number, scale) = if unit == "Hz" && (text.ends_with('k') || text.ends_with('K')) {
        (&text[..text.len() - 1], 1000.)
    } else {
        (text, 1.)
    };
    number
        .trim()
        .parse::<f64>()
        .ok()
        .map(|v| v * scale)
        .filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tone_precision_domain_does_not_change_rotary_or_level_endpoints() {
        let graph = crate::service::ServiceGraph::default().snapshot();
        let mut source = graph.node_of_kind(NodeKind::Source).unwrap().1.clone();
        source.source = Some(InputSelection::Tone { frequency_hz: 440. });
        assert_eq!(Parameter::Frequency.range(&source), (20., 20_000.));
        for value in [0.0000001, 1., 23999., 23999.999] {
            assert!(
                Parameter::Frequency
                    .edit_precision(value, NodeKind::Source, &source)
                    .is_some()
            );
            assert!(
                Parameter::Frequency
                    .edit(value, NodeKind::Source, &source)
                    .is_none()
            );
        }
        for value in [0., -1., 24000., f64::NAN, f64::INFINITY] {
            assert!(
                Parameter::Frequency
                    .edit_precision(value, NodeKind::Source, &source)
                    .is_none()
            );
        }
        let (_, gain) = graph.node_of_kind(NodeKind::Gain).unwrap();
        let maximum = Parameter::Level.range(gain).1;
        let Some(GraphEdit::SetGain { gain: actual, .. }) =
            Parameter::Level.edit(maximum, NodeKind::Gain, gain)
        else {
            panic!("the 16x gesture endpoint must remain editable")
        };
        assert_eq!(actual, 16.);
        assert!(
            Parameter::Level
                .edit_precision(maximum, NodeKind::Gain, gain)
                .is_none()
        );
        for value in [-120., -100., 24., f64::NEG_INFINITY] {
            assert!(
                Parameter::Level
                    .edit_precision(value, NodeKind::Gain, gain)
                    .is_some()
            );
        }
        for value in [-120.1, 24.01, f64::NAN, f64::INFINITY] {
            assert!(
                Parameter::Level
                    .edit_precision(value, NodeKind::Gain, gain)
                    .is_none()
            );
        }
    }

    #[test]
    fn precision_delay_feedback_endpoints_match_engine_without_clamping_invalid_text() {
        let mut graph = crate::service::ServiceGraph::default();
        graph
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Delay,
                channels: 2,
            })
            .unwrap();
        let snapshot = graph.snapshot();
        let node = snapshot.nodes.last().unwrap();
        for (text, expected) in [("99.9%", 0.999), ("-99.9%", -0.999)] {
            let value = parse(text, Parameter::DelayFeedback.unit()).unwrap();
            let Some(GraphEdit::SetBuiltin {
                state: BuiltinState::Delay(settings),
                ..
            }) = Parameter::DelayFeedback.edit(value, NodeKind::Delay, node)
            else {
                panic!("valid feedback endpoint")
            };
            assert_eq!(settings.feedback, expected);
            graph
                .edit(GraphEdit::SetBuiltin {
                    node: node.id,
                    state: BuiltinState::Delay(settings),
                })
                .expect("the service accepts the exact engine endpoint");
        }
        for value in [
            99.90000000000002,
            -99.90000000000002,
            f64::INFINITY,
            f64::NAN,
        ] {
            assert!(
                Parameter::DelayFeedback
                    .edit(value, NodeKind::Delay, node)
                    .is_none()
            );
        }
    }
    #[test]
    fn precision_entry_keeps_units_and_rejects_invalid_values() {
        assert_eq!(parse("1.5k Hz", "Hz"), Some(1500.));
        assert_eq!(parse("-12 dB", "dB"), Some(-12.));
        assert_eq!(parse("NaN", "dB"), None);
        assert_eq!(parse("garbage", "ms"), None);
        assert_eq!(parse("-inf dB", "dB"), Some(f64::NEG_INFINITY));
    }
    #[test]
    fn finite_quiet_level_is_not_silence() {
        let graph = crate::service::ServiceGraph::default().snapshot();
        let (_, node) = graph.node_of_kind(NodeKind::Gain).unwrap();
        let Some(GraphEdit::SetGain { gain, .. }) =
            Parameter::Level.edit(-72., NodeKind::Gain, node)
        else {
            panic!("level edit")
        };
        assert!(gain > 0.);
        let Some(GraphEdit::SetGain { gain, .. }) =
            Parameter::Level.edit(f64::NEG_INFINITY, NodeKind::Gain, node)
        else {
            panic!("silence edit")
        };
        assert_eq!(gain, 0.);
        assert!(
            Parameter::EqGain
                .edit(f64::NEG_INFINITY, NodeKind::Eq, node)
                .is_none()
        );
    }
    #[test]
    fn delay_reset_uses_the_nodes_actual_capacity() {
        let mut graph = crate::service::ServiceGraph::default();
        graph
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Delay,
                channels: 2,
            })
            .unwrap();
        let snapshot = graph.snapshot();
        let mut node = snapshot.nodes.last().unwrap().clone();
        let BuiltinState::Delay(settings) = &mut node.builtin else {
            panic!("delay settings")
        };
        settings.max_delay_ms = 500.;
        let position = Parameter::DelayTime.default_position(NodeKind::Delay, &node);
        assert!((Parameter::DelayTime.value_at_position(position, &node) - 250.).abs() < 0.01);
    }
    #[test]
    fn every_processor_primary_parameter_round_trips_and_edits_its_node() {
        let mut graph = crate::service::ServiceGraph::default();
        for kind in [
            NodeKind::Eq,
            NodeKind::Compressor,
            NodeKind::Gate,
            NodeKind::Limiter,
            NodeKind::Delay,
        ] {
            graph
                .edit(GraphEdit::AddNode { kind, channels: 2 })
                .unwrap();
            let snapshot = graph.snapshot();
            let node = snapshot.nodes.last().unwrap();
            for p in Parameter::primary(kind, node) {
                let value = p.value_at_position(p.position(kind, node), node);
                assert!((value - p.value(kind, node)).abs() < 0.01, "{p:?}");
                assert!(p.edit(value, kind, node).is_some(), "{p:?}");
                assert!(p.edit(f64::NAN, kind, node).is_none());
            }
        }
    }
}
