//! Canvas-knob value math shared by projection, painting, and gestures
//! (design 18 B3). The knob works in a control's native domain — dB for
//! gain, linear for pan, log-Hz for tone frequency, ms for delay — and
//! every adjustment the gestures produce goes through the same `GraphEdit`
//! the inspector dispatches, so undo coalescing and live-parameter routing
//! behave identically on both surfaces.

use crate::service::{BuiltinState, GraphEdit, GraphNodeState, InputSelection};
use moiren_core::graph::NodeId;

/// Full vertical sweep of a knob drag, in window pixels.
pub const DRAG_SWEEP_PX: f64 = 200.;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnobControl {
    /// Gain processor level, worked in dB.
    GainLevel,
    /// Pan processor position, -1..1.
    PanPosition,
    /// Tone generator frequency, log domain.
    ToneFrequency,
    /// Delay processor time in ms.
    DelayTime,
}

/// One knob's writable projection: what the card paints and where the
/// value goes. Built during projection; gestures only do the math here.
#[derive(Debug, Clone)]
pub struct CardKnob {
    pub control: KnobControl,
    pub label: String,
    pub value_text: String,
    /// 0..1 across the knob's sweep, for the painted arc.
    pub position: f32,
}

pub struct KnobSpec {
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Wheel notch size in the working domain (multiplicative for the
    /// log-domain tone knob).
    pub step: f64,
    pub log: bool,
}

impl KnobControl {
    pub fn spec(&self) -> KnobSpec {
        match self {
            Self::GainLevel => KnobSpec {
                min: -60.,
                max: 24.,
                default: 0.,
                step: 1.,
                log: false,
            },
            Self::PanPosition => KnobSpec {
                min: -1.,
                max: 1.,
                default: 0.,
                step: 0.05,
                log: false,
            },
            Self::ToneFrequency => KnobSpec {
                min: 1.,
                max: 23_999.,
                default: 440.,
                step: 2f64.powf(1. / 12.),
                log: true,
            },
            Self::DelayTime => KnobSpec {
                min: 0.,
                max: 2_000.,
                default: 250.,
                step: 10.,
                log: false,
            },
        }
    }

    /// The knob's current value from the node state. Digital zero gain
    /// reads as the sweep's floor; the inspector keeps -inf editing.
    pub fn current(&self, state: &GraphNodeState) -> f64 {
        match self {
            Self::GainLevel => {
                if state.gain <= 0. {
                    -60.
                } else {
                    (20. * state.gain.log10()).clamp(-60., 24.)
                }
            }
            Self::PanPosition => state.pan.clamp(-1., 1.),
            Self::ToneFrequency => match &state.source {
                Some(InputSelection::Tone { frequency_hz }) => frequency_hz.clamp(1., 23_999.),
                _ => 440.,
            },
            Self::DelayTime => state
                .builtin
                .delay()
                .map_or(250., |settings| settings.delay_ms.clamp(0., 2_000.)),
        }
    }

    /// The edit a new working value produces, reading companion fields
    /// (feedback/mix for delay) from the current state.
    pub fn edit(&self, node: NodeId, state: &GraphNodeState, value: f64) -> Option<GraphEdit> {
        let spec = self.spec();
        let value = value.clamp(spec.min, spec.max);
        Some(match self {
            Self::GainLevel => GraphEdit::SetGain {
                node,
                gain: 10f64.powf(value / 20.),
            },
            Self::PanPosition => GraphEdit::SetPan { node, pan: value },
            Self::ToneFrequency => GraphEdit::BindSource {
                node,
                source: InputSelection::Tone {
                    frequency_hz: value,
                },
            },
            Self::DelayTime => {
                let settings = state.builtin.delay()?;
                GraphEdit::SetBuiltin {
                    node,
                    state: BuiltinState::Delay(moiren_engine::processor::DelaySettings {
                        delay_ms: value,
                        ..*settings
                    }),
                }
            }
        })
    }

    pub fn value_text(&self, value: f64) -> String {
        match self {
            Self::GainLevel => format!("{value:+.1} dB"),
            Self::PanPosition => {
                if value < -0.005 {
                    format!("L {:.0}", -value * 100.)
                } else if value > 0.005 {
                    format!("R {:.0}", value * 100.)
                } else {
                    "C".into()
                }
            }
            Self::ToneFrequency => format!("{value:.0} Hz"),
            Self::DelayTime => format!("{value:.0} ms"),
        }
    }

    /// Position on the 0..1 sweep, log-aware.
    pub fn position(&self, value: f64) -> f32 {
        let spec = self.spec();
        let normalize = |value: f64| {
            if spec.log {
                ((value.max(1e-6).ln() - spec.min.max(1.).ln())
                    / (spec.max.ln() - spec.min.max(1.).ln()))
                .clamp(0., 1.)
            } else {
                ((value - spec.min) / (spec.max - spec.min)).clamp(0., 1.)
            }
        };
        normalize(value) as f32
    }

    /// One wheel notch (positive = up = increase).
    pub fn wheel(&self, value: f64, notches: f64) -> f64 {
        let spec = self.spec();
        let stepped = if spec.log {
            value * spec.step.powf(notches)
        } else {
            value + spec.step * notches
        };
        stepped.clamp(spec.min, spec.max)
    }

    /// A vertical drag: full sweep across [`DRAG_SWEEP_PX`] pixels, up
    /// increases, log-aware.
    pub fn drag(&self, start: f64, dy: f64) -> f64 {
        let spec = self.spec();
        let position = (self.position(start) as f64 - dy / DRAG_SWEEP_PX).clamp(0., 1.);
        let value = if spec.log {
            (spec.min.max(1.).ln() + position * (spec.max.ln() - spec.min.max(1.).ln())).exp()
        } else {
            spec.min + position * (spec.max - spec.min)
        };
        value.clamp(spec.min, spec.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_round_trips_through_db_and_respects_the_domain() {
        let control = KnobControl::GainLevel;
        let state = GraphNodeState {
            gain: 0.5,
            ..sample_state()
        };
        let db = control.current(&state);
        assert!((db - (-6.0206)).abs() < 0.001);
        let edit = control.edit(test_node_id(), &state, db).unwrap();
        let GraphEdit::SetGain { gain, .. } = edit else {
            panic!("gain knob edits gain");
        };
        assert!((gain - 0.5).abs() < 0.001);
        // Digital zero reads as the floor and the wheel clamps there.
        let state = GraphNodeState {
            gain: 0.,
            ..sample_state()
        };
        assert_eq!(control.current(&state), -60.);
        assert_eq!(control.wheel(-60., -1.), -60.);
        assert!((control.wheel(-60., 1.) - -59.).abs() < 0.001);
    }

    #[test]
    fn tone_drag_and_wheel_stay_log_uniform_and_inside_the_domain() {
        let control = KnobControl::ToneFrequency;
        // Dragging the dial to the top of the sweep reaches the ceiling
        // from anywhere; twelve wheel semitones make an octave.
        let to_top = -(1. - control.position(110.) as f64) * DRAG_SWEEP_PX;
        let low = control.drag(110., to_top);
        assert!((low - 23_999.).abs() < 1.);
        let octave = control.wheel(440., 12.);
        assert!((octave - 880.).abs() < 0.5, "{octave}");
        assert_eq!(control.wheel(440., 0.), 440.);
        // Drags stay inside 1 Hz .. 23 999 Hz.
        assert_eq!(control.drag(440., DRAG_SWEEP_PX), 1.);
    }

    #[test]
    fn pan_and_delay_wheel_and_default_reset() {
        let pan = KnobControl::PanPosition;
        assert!((pan.wheel(0., 1.) - 0.05).abs() < 1e-9);
        assert_eq!(pan.wheel(0.9, 100.), 1.);
        assert_eq!(pan.spec().default, 0.);
        let delay = KnobControl::DelayTime;
        assert!((delay.wheel(250., -1.) - 240.).abs() < 1e-9);
        assert_eq!(delay.wheel(250., -1e5), 0.);
    }

    fn sample_state() -> GraphNodeState {
        use crate::service::{CompressorState, MixerState};
        GraphNodeState {
            id: test_node_id(),
            label: "n".into(),
            gain: 1.,
            pan: 0.,
            mono: false,
            swap: false,
            mixer: MixerState::default(),
            compressor: CompressorState::default(),
            builtin: BuiltinState::None,
            source: None,
            output_endpoint_id: None,
            tint: None,
        }
    }

    fn test_node_id() -> NodeId {
        let mut graph = moiren_core::graph::LogicalGraph::new();
        graph
            .create_node(moiren_core::graph::NodeKind::Gain, 2)
            .unwrap()
    }
}
