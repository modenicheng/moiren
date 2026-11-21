//! The undo/redo journal: deep-copied snapshot entries whose revisions and
//! runtime confirmations stay live on the snapshot itself.
use super::{
    ExportIntent, GraphNodeState, GraphPreparation, GraphSnapshot, LogicalGraph, NodeId,
    ServiceGraph,
};
use std::collections::BTreeSet;

/// How long successive same-key edits stay merged into one undo step.
const UNDO_COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
/// Upper bound on remembered undo states; oldest entries drop off first.
const UNDO_DEPTH: usize = 100;

/// Snapshot journal for undo/redo. Entries hold only the editable candidate
/// content; revision and runtime confirmation fields stay with the live
/// snapshot so an undo can never fabricate or hide an owner confirmation.
#[derive(Default)]
pub(super) struct UndoJournal {
    pub(super) undo: Vec<GraphSnapshot>,
    pub(super) redo: Vec<GraphSnapshot>,
    last: Option<(String, std::time::Instant)>,
}

impl UndoJournal {
    pub(super) fn record(&mut self, before: GraphSnapshot, key: Option<String>) {
        let now = std::time::Instant::now();
        let coalesced = key.as_ref().is_some_and(|key| {
            self.last.as_ref().is_some_and(|(last, at)| {
                *last == *key && now.duration_since(*at) < UNDO_COALESCE_WINDOW
            })
        });
        if !coalesced {
            self.undo.push(before);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.last = key.map(|key| (key, now));
    }
}

/// Nodes a topology change touches: added or removed nodes plus both
/// endpoints of every edge that appeared or disappeared.
fn structural_diff_nodes(before: &LogicalGraph, after: &LogicalGraph) -> Vec<NodeId> {
    let mut touched = BTreeSet::new();
    let edge_change = |from: &LogicalGraph, to: &LogicalGraph, touched: &mut BTreeSet<NodeId>| {
        let kept: BTreeSet<_> = to.edges().iter().map(|edge| edge.id()).collect();
        for edge in from.edges() {
            if !kept.contains(&edge.id()) {
                touched.insert(edge.src());
                touched.insert(edge.dst());
            }
        }
    };
    edge_change(before, after, &mut touched);
    edge_change(after, before, &mut touched);
    let after_nodes: BTreeSet<_> = after.nodes().iter().map(|node| node.id()).collect();
    for node in before.nodes() {
        if !after_nodes.contains(&node.id()) {
            touched.insert(node.id());
        }
    }
    let before_nodes: BTreeSet<_> = before.nodes().iter().map(|node| node.id()).collect();
    touched.extend(
        after_nodes
            .into_iter()
            .filter(|id| !before_nodes.contains(id)),
    );
    touched.into_iter().collect()
}

impl ServiceGraph {
    /// A loaded document creates a new candidate. Disk contents cannot confirm
    /// native activity, and stale in-flight preparations never confirm this edit.
    pub fn replace(
        &mut self,
        graph: LogicalGraph,
        nodes: Vec<GraphNodeState>,
    ) -> Result<(), String> {
        self.replace_with_exports(graph, nodes, Vec::new())
    }

    pub fn replace_with_exports(
        &mut self,
        graph: LogicalGraph,
        nodes: Vec<GraphNodeState>,
        exports: Vec<ExportIntent>,
    ) -> Result<(), String> {
        GraphPreparation {
            exports: exports.clone(),
            graph: graph.clone(),
            nodes: nodes.clone(),
            revision: 0,
            plugins: super::super::plugins::empty_library(),
        }
        .validate_state()?;
        let revision = self
            .snapshot
            .candidate_revision
            .checked_add(1)
            .ok_or("The graph revision space is exhausted")?;
        self.journal.record(self.snapshot.clone(), None);
        self.snapshot.graph = graph;
        self.snapshot.exports = exports;
        self.snapshot.nodes = nodes;
        self.snapshot.candidate_revision = revision;
        self.snapshot.last_error = None;
        self.snapshot.can_undo = !self.journal.undo.is_empty();
        self.snapshot.can_redo = !self.journal.redo.is_empty();
        // The replaced candidate advances the revision: a real content change.
        self.bump();
        Ok(())
    }
    /// Restore the editable content recorded before the last edit. Revision
    /// and runtime confirmation fields stay live; the restored content gets a
    /// fresh candidate revision so consumers never see a revision go back.
    pub fn undo(&mut self) -> bool {
        self.step_journal(true)
    }
    pub fn redo(&mut self) -> bool {
        self.step_journal(false)
    }
    fn step_journal(&mut self, back: bool) -> bool {
        let entry = if back {
            self.journal.undo.pop()
        } else {
            self.journal.redo.pop()
        };
        let Some(entry) = entry else {
            return false;
        };
        if back {
            self.journal.redo.push(self.snapshot.clone());
        } else {
            self.journal.undo.push(self.snapshot.clone());
        }
        self.journal.last = None;
        let revision = self
            .snapshot
            .candidate_revision
            .checked_add(1)
            .expect("revision headroom retained by undo depth");
        let can_undo = !self.journal.undo.is_empty();
        let can_redo = !self.journal.redo.is_empty();
        // A topology step re-prepares the engine; mark what it changed so
        // cards keep the loading state through the re-apply, and never
        // inherit a stale set from the journal entry.
        self.snapshot.preparing_nodes = if self.snapshot.graph != entry.graph {
            structural_diff_nodes(&self.snapshot.graph, &entry.graph)
        } else {
            Vec::new()
        };
        self.snapshot.exports = entry.exports;
        self.snapshot.graph = entry.graph;
        self.snapshot.nodes = entry.nodes;
        self.snapshot.last_error = None;
        self.snapshot.faults.clear();
        self.snapshot.candidate_revision = revision;
        self.snapshot.can_undo = can_undo;
        self.snapshot.can_redo = can_redo;
        self.bump();
        true
    }
}
