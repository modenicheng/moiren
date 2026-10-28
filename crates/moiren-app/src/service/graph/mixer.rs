use std::collections::BTreeSet;

use moiren_engine::processor::{ChannelMode, StripConfig};
use serde::{Deserialize, Serialize};

use super::{GraphNodeState, GraphPreparation, LogicalGraph, NodeId, ServiceGraph};

/// One strip control intent, resolved against the service's latest candidate.
/// Absolute values use linear gain and signed balance, not dB or slider units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MixerEdit {
    SetGain(f64),
    SetBalance(f64),
    AdjustBalance(f64),
    ToggleMute,
    ToggleSolo,
    ToggleDim,
    CycleMode,
}

impl MixerEdit {
    pub(super) fn apply(self, state: &mut MixerState, channels: usize) -> Result<(), String> {
        match self {
            Self::SetGain(value) => state.gain = value,
            Self::SetBalance(value) => state.balance = value,
            Self::AdjustBalance(delta) => {
                // Reject infinities before clamping; otherwise an invalid intent
                // would silently become a valid endpoint and enter undo history.
                if !delta.is_finite() {
                    return Err("Mixer balance adjustment must be finite".into());
                }
                state.balance = (state.balance + delta).clamp(-1.0, 1.0);
            }
            Self::ToggleMute => state.muted = !state.muted,
            Self::ToggleSolo => state.solo = !state.solo,
            Self::ToggleDim => state.dimmed = !state.dimmed,
            Self::CycleMode => *state = state.next_mode(channels),
        }
        Ok(())
    }

    pub(super) fn undo_key(self) -> Option<&'static str> {
        match self {
            Self::SetGain(_) => Some("gain"),
            Self::SetBalance(_) | Self::AdjustBalance(_) => Some("balance"),
            Self::ToggleMute | Self::ToggleSolo | Self::ToggleDim | Self::CycleMode => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MixerState {
    pub gain: f64,
    pub balance: f64,
    pub muted: bool,
    pub dimmed: bool,
    pub solo: bool,
    pub channel_mode: usize,
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            gain: 1.0,
            balance: 0.0,
            muted: false,
            dimmed: false,
            solo: false,
            channel_mode: 0,
        }
    }
}

impl MixerState {
    pub fn valid(self, channels: usize) -> bool {
        self.gain.is_finite()
            && (0.0..=16.0).contains(&self.gain)
            && self.balance.is_finite()
            && (-1.0..=1.0).contains(&self.balance)
            && self.channel_mode <= channels + 2
    }

    pub fn mode_label(self, channels: usize) -> String {
        match self.channel_mode {
            0 if channels == 2 => "ST".into(),
            0 => format!("{channels} CH"),
            1 => "MONO".into(),
            mode if mode == channels + 2 => "SWAP".into(),
            2 if channels == 2 => "L".into(),
            3 if channels == 2 => "R".into(),
            mode => format!("CH {} ONLY", mode - 1),
        }
    }

    pub fn next_mode(mut self, channels: usize) -> Self {
        self.channel_mode = (self.channel_mode + 1) % (channels + 3);
        self
    }

    fn config(self, channels: usize, audible: bool) -> StripConfig {
        let mode = match self.channel_mode {
            0 => ChannelMode::Normal,
            1 => ChannelMode::Mono,
            mode if mode == channels + 2 => ChannelMode::Swap,
            mode => ChannelMode::Solo(mode - 2),
        };
        StripConfig {
            balance: self.balance,
            gain: self.gain
                * if self.dimmed {
                    10f64.powf(-12. / 20.)
                } else {
                    1.
                },
            muted: self.muted || !audible,
            mode,
        }
    }
}

impl GraphPreparation {
    pub(crate) fn mixer_configs(&self) -> Vec<(NodeId, StripConfig)> {
        mixer_configs(&self.graph, &self.nodes)
    }
}

impl ServiceGraph {
    /// Project only effective strip values; do not clone unrelated DSP payloads.
    pub(crate) fn mixer_configs(&self) -> Vec<(NodeId, StripConfig)> {
        mixer_configs(&self.snapshot.graph, &self.snapshot.nodes)
    }
}

fn mixer_configs(graph: &LogicalGraph, nodes: &[GraphNodeState]) -> Vec<(NodeId, StripConfig)> {
    let solos: BTreeSet<_> = nodes
        .iter()
        .filter(|state| state.mixer.solo)
        .map(|state| state.id)
        .collect();
    let reachable = |upstream: bool| {
        let mut included = solos.clone();
        let mut pending: Vec<_> = solos.iter().copied().collect();
        while let Some(node) = pending.pop() {
            for edge in graph.edges() {
                let (from, to) = if upstream {
                    (edge.dst(), edge.src())
                } else {
                    (edge.src(), edge.dst())
                };
                if from == node && included.insert(to) {
                    pending.push(to);
                }
            }
        }
        included
    };
    let mut audible = reachable(true);
    audible.extend(reachable(false));
    graph
        .nodes()
        .iter()
        .map(|node| {
            let state = nodes
                .iter()
                .find(|state| state.id == node.id())
                .expect("validated mixer state");
            (
                node.id(),
                state.mixer.config(
                    node.channels(),
                    solos.is_empty() || audible.contains(&node.id()),
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{GraphEdit, ServiceGraph};
    use moiren_core::graph::{NodeKind, SendParams};

    #[test]
    fn stereo_balance_round_trips_and_reaches_prepared_strip() {
        let state: MixerState = serde_json::from_value(serde_json::json!({
            "balance": 0.5,
            "gain": 0.25,
            "dimmed": true,
            "channel_mode": 3
        }))
        .unwrap();
        assert!(state.valid(2));
        assert_eq!(serde_json::to_value(state).unwrap()["balance"], 0.5);
        let config = state.config(2, true);
        assert_eq!(config.balance, 0.5);
        assert_eq!(config.mode, ChannelMode::Solo(1));
        assert!((config.gain - 0.25 * 10f64.powf(-12. / 20.)).abs() < 1e-12);
    }

    #[test]
    fn balance_validation_rejects_invalid_edits_without_changing_the_graph() {
        let mut service = ServiceGraph::default();
        let before = service.snapshot();
        let node = before.nodes[0].id;
        for balance in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.01, 1.01] {
            assert!(
                service
                    .edit(GraphEdit::SetMixer {
                        node,
                        state: MixerState {
                            balance,
                            ..Default::default()
                        },
                    })
                    .is_err()
            );
            assert_eq!(service.snapshot().nodes[0].mixer, before.nodes[0].mixer);
        }
        for balance in [-1.0, 0.0, 1.0] {
            assert!(
                MixerState {
                    balance,
                    ..Default::default()
                }
                .valid(2)
            );
        }
    }

    #[test]
    fn stereo_channel_modes_use_compact_labels_without_changing_indices() {
        let mut state = MixerState::default();
        for label in ["ST", "MONO", "L", "R", "SWAP"] {
            assert_eq!(state.mode_label(2), label);
            state = state.next_mode(2);
        }
        assert_eq!(state.channel_mode, 0);
    }

    #[test]
    fn solo_keeps_ancestors_and_descendants_but_silences_parallel_sources() {
        let mut service = ServiceGraph::default();
        service
            .edit(GraphEdit::AddNode {
                kind: NodeKind::Source,
                channels: 2,
            })
            .unwrap();
        let before = service.snapshot();
        let gain = before
            .graph
            .nodes()
            .iter()
            .find(|node| node.kind() == NodeKind::Gain)
            .unwrap()
            .id();
        let other = before.nodes.last().unwrap().id;
        service
            .edit(GraphEdit::SetMixer {
                node: gain,
                state: MixerState {
                    solo: true,
                    gain: 0.5,
                    ..Default::default()
                },
            })
            .unwrap();
        let configs = service.preparation().mixer_configs();
        assert!(configs.iter().find(|(id, _)| *id == other).unwrap().1.muted);
        assert!(
            configs
                .iter()
                .filter(|(id, _)| *id != other)
                .all(|(_, config)| !config.muted)
        );
        assert_eq!(service.snapshot().graph.edges(), before.graph.edges());
        assert_eq!(SendParams::default().gain, 1.);
    }

    #[test]
    fn channel_mode_cycles_each_channel_and_rejects_out_of_range() {
        for channels in [1, 2, 6] {
            let mut state = MixerState::default();
            let mut labels = BTreeSet::new();
            for _ in 0..channels + 3 {
                assert!(state.valid(channels));
                assert!(labels.insert(state.mode_label(channels)));
                state = state.next_mode(channels);
            }
            assert_eq!(state.channel_mode, 0);
            state.channel_mode = channels + 3;
            assert!(!state.valid(channels));
        }
    }
}
