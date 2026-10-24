use std::collections::{BTreeMap, BTreeSet};

use moiren_core::graph::{LogicalGraph, NodeId};
use moiren_engine::node::VirtualEndpointId;
use serde::{Deserialize, Serialize};

use super::{PersistenceError, ProjectDocument};
use crate::service::{
    ExportDirection, ExportIntent, GraphPreparation, GraphSnapshot, MAX_EXPORTS, validate_exports,
};

/// Portable desired configuration. Default-device authorization and live
/// driver/session identities have no representation in this document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportDocument {
    pub endpoint_id: VirtualEndpointId,
    pub direction: ExportDirection,
    pub name: String,
    pub channels: usize,
    pub enabled: bool,
    /// Document-local IO node ordinal, remapped when the topology is restored.
    pub binding: Option<u64>,
}

impl ExportDocument {
    pub(super) fn from_snapshot(graph: &GraphSnapshot) -> Result<Vec<Self>, PersistenceError> {
        validate_exports(&graph.exports, &graph.graph)
            .map_err(|error| PersistenceError::invalid("exports", error))?;
        let ids: BTreeMap<_, _> = graph
            .graph
            .nodes()
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id(), index as u64))
            .collect();
        Ok(graph
            .exports
            .iter()
            .map(|export| Self {
                endpoint_id: export.endpoint_id,
                direction: export.direction,
                name: export.name.clone(),
                channels: export.channels,
                enabled: export.enabled,
                binding: export.binding.map(|node| ids[&node]),
            })
            .collect())
    }
}

impl ProjectDocument {
    pub(super) fn restore_exports(
        &self,
        graph: &LogicalGraph,
        node_ids: &BTreeMap<u64, NodeId>,
    ) -> Result<Vec<ExportIntent>, PersistenceError> {
        if self.exports.len() > MAX_EXPORTS {
            return Err(PersistenceError::invalid(
                "exports",
                "at most 64 exports are supported",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut exports = Vec::with_capacity(self.exports.len());
        for export in &self.exports {
            if !ids.insert(export.endpoint_id) {
                return Err(PersistenceError::invalid(
                    "exports",
                    "duplicate virtual endpoint identity",
                ));
            }
            exports.push(ExportIntent {
                endpoint_id: export.endpoint_id,
                direction: export.direction,
                name: export.name.clone(),
                channels: export.channels,
                enabled: export.enabled,
                binding: export
                    .binding
                    .map(|id| {
                        node_ids.get(&id).copied().ok_or_else(|| {
                            PersistenceError::invalid("exports.binding", "missing graph IO node")
                        })
                    })
                    .transpose()?,
            });
        }
        validate_exports(&exports, graph)
            .map_err(|error| PersistenceError::invalid("exports", error))?;
        Ok(exports)
    }

    pub fn restore_graph(&self) -> Result<GraphPreparation, PersistenceError> {
        self.validate()?;
        let document = self
            .graph
            .as_ref()
            .ok_or_else(|| PersistenceError::invalid("graph", "project has no graph"))?;
        let (restored, nodes) = document.restore_with_ids()?;
        let exports = self.restore_exports(&restored.graph, &restored.node_ids)?;
        Ok(GraphPreparation {
            graph: restored.graph,
            nodes,
            exports,
            revision: 0,
        })
    }
}
