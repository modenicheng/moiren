use super::*;
use moiren_engine::spectrum::SpectrumSettings;

fn fixture() -> (ServiceGraph, NodeId) {
    let mut graph = ServiceGraph::default();
    graph
        .edit(GraphEdit::AddNode {
            kind: NodeKind::SpectrumAnalyzer,
            channels: 2,
        })
        .unwrap();
    let node = graph
        .snapshot()
        .node_of_kind(NodeKind::SpectrumAnalyzer)
        .unwrap()
        .0
        .id();
    graph.journal = UndoJournal::default();
    (graph, node)
}

fn settings(graph: &ServiceGraph, node: NodeId) -> SpectrumSettings {
    *graph
        .snapshot()
        .node(node)
        .unwrap()
        .builtin
        .spectrum()
        .unwrap()
}

#[test]
fn spectrum_queued_fields_merge_latest_and_fft_clamps_only_current_hop() {
    let (mut graph, node) = fixture();
    assert!(
        graph
            .edit(GraphEdit::SetSpectrumHop {
                node,
                hop_frames: 128
            })
            .unwrap()
    );
    assert!(
        graph
            .edit(GraphEdit::SetSpectrumFft {
                node,
                fft_size: 2048
            })
            .unwrap()
    );
    assert_eq!(
        settings(&graph, node),
        SpectrumSettings {
            fft_size: 2048,
            hop_frames: 128
        }
    );
    graph
        .edit(GraphEdit::SetSpectrumHop {
            node,
            hop_frames: 1024,
        })
        .unwrap();
    graph
        .edit(GraphEdit::SetSpectrumFft {
            node,
            fft_size: 256,
        })
        .unwrap();
    assert_eq!(
        settings(&graph, node),
        SpectrumSettings {
            fft_size: 256,
            hop_frames: 256
        }
    );
    graph
        .edit(GraphEdit::SetSpectrumHop {
            node,
            hop_frames: 64,
        })
        .unwrap();
    assert_eq!(
        settings(&graph, node),
        SpectrumSettings {
            fft_size: 256,
            hop_frames: 64
        }
    );
}

#[test]
fn spectrum_invalid_sizes_hops_and_wrong_node_reject_without_history_mutation() {
    let (mut graph, node) = fixture();
    let wrong = graph
        .snapshot()
        .node_of_kind(NodeKind::Gain)
        .unwrap()
        .0
        .id();
    for edit in [
        GraphEdit::SetSpectrumFft { node, fft_size: 0 },
        GraphEdit::SetSpectrumFft { node, fft_size: 17 },
        GraphEdit::SetSpectrumFft {
            node,
            fft_size: 16384,
        },
        GraphEdit::SetSpectrumHop {
            node,
            hop_frames: 0,
        },
        GraphEdit::SetSpectrumHop {
            node,
            hop_frames: 1025,
        },
        GraphEdit::SetSpectrumFft {
            node: wrong,
            fft_size: 1024,
        },
        GraphEdit::SetSpectrumHop {
            node: wrong,
            hop_frames: 128,
        },
    ] {
        let before = graph.snapshot();
        assert!(graph.edit(edit).is_err());
        let after = graph.snapshot();
        assert_eq!(after.nodes, before.nodes);
        assert_eq!(after.candidate_revision, before.candidate_revision);
        assert!(graph.journal.undo.is_empty());
    }
}

#[test]
fn spectrum_undo_separates_fields_and_restores_fft_induced_hop_clamp() {
    let (mut graph, node) = fixture();
    for hop_frames in [256, 128] {
        graph
            .edit(GraphEdit::SetSpectrumHop { node, hop_frames })
            .unwrap();
    }
    graph
        .edit(GraphEdit::SetSpectrumFft { node, fft_size: 64 })
        .unwrap();
    assert_eq!(graph.journal.undo.len(), 2);
    assert!(graph.undo());
    assert_eq!(
        settings(&graph, node),
        SpectrumSettings {
            fft_size: 1024,
            hop_frames: 128
        }
    );
    assert!(graph.undo());
    assert_eq!(settings(&graph, node), SpectrumSettings::default());
}
