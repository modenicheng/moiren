//! Capture descriptors read actual service settings, never editor defaults.
use crate::service::{AppSnapshot, BuiltinState};
use gpui_kit::Bounds;
use moiren_core::graph::NodeId;
use moiren_engine::recorder::RecorderSettings;

pub(super) type Parameter = (&'static str, &'static str, f64);

pub(super) fn parameters(state: &BuiltinState) -> Option<(&'static str, Vec<Parameter>)> {
    Some(match state {
        BuiltinState::Eq(s) => (
            "eq",
            vec![
                ("EqFrequency", "Hz", s.frequency_hz),
                ("EqGain", "dB", s.gain_db),
                ("EqQ", "Q", s.q),
            ],
        ),
        BuiltinState::Gate(s) => (
            "gate",
            vec![
                ("GateThreshold", "dB", s.threshold_db),
                ("GateRange", "dB", s.range_db),
                ("GateAttack", "ms", s.attack_ms),
                ("GateHold", "ms", s.hold_ms),
                ("GateRelease", "ms", s.release_ms),
            ],
        ),
        BuiltinState::Limiter(s) => (
            "limiter",
            vec![
                ("LimitInput", "dB", s.input_gain_db),
                ("LimitThreshold", "dB", s.threshold_db),
                ("LimitRelease", "ms", s.release_ms),
            ],
        ),
        BuiltinState::Delay(s) => (
            "delay",
            vec![
                ("DelayTime", "ms", s.delay_ms),
                ("DelayFeedback", "%", s.feedback * 100.),
                ("DelayMix", "%", s.mix * 100.),
            ],
        ),
        _ => return None,
    })
}

pub(super) struct IntegerField {
    pub name: &'static str,
    pub value: usize,
    pub maximum: usize,
    pub busy: bool,
}

pub(super) fn integer(snapshot: &AppSnapshot, node: NodeId) -> Option<IntegerField> {
    let topology = snapshot.graph.graph.get_node(node).ok()?;
    let state = snapshot.graph.node(node)?.builtin.resolved(topology.kind());
    let (name, value, maximum) = match state {
        BuiltinState::Meter(s) if s.valid() => ("MeterWindow", s.window_frames, 1_048_576),
        BuiltinState::Recorder(s) if s.valid_for_channels(topology.channels()) => (
            "RecorderCapacity",
            s.capacity_frames,
            RecorderSettings::max_capacity_frames(topology.channels())?,
        ),
        _ => return None,
    };
    Some(IntegerField {
        name,
        value,
        maximum,
        busy: name == "RecorderCapacity"
            && snapshot
                .recordings
                .iter()
                .any(|r| r.node == node && r.phase.is_busy()),
    })
}

/// Gate wraps five dials into two rows. Align within each observed row, while
/// also rejecting overlapping rows or horizontally overlapping fields.
pub(super) fn verify_rows(cells: &[(Bounds<f32>, Bounds<f32>)]) -> anyhow::Result<usize> {
    let mut cells = cells.to_vec();
    cells.sort_by(|a, b| {
        a.0.top()
            .total_cmp(&b.0.top())
            .then_with(|| a.0.left().total_cmp(&b.0.left()))
    });
    let (mut row_top, mut row_bottom, mut right, mut baseline) = (0., 0., 0., 0.);
    let mut rows = 0;
    for (field, input) in cells {
        if rows == 0 || (field.top() - row_top).abs() > 1. {
            anyhow::ensure!(
                rows == 0 || field.top() >= row_bottom,
                "scalar dial rows overlap"
            );
            rows += 1;
            row_top = field.top();
            baseline = input.center().y;
        } else {
            anyhow::ensure!(
                field.left() >= right && (input.center().y - baseline).abs() <= 1.,
                "scalar dial row overlaps or numeric baselines differ"
            );
        }
        row_bottom = row_bottom.max(field.bottom());
        right = field.right();
    }
    Ok(rows)
}
