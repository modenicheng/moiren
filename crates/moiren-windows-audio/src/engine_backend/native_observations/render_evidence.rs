//! Exactly-once control reader pinned to one immutable native owner.
use super::*;
use crate::render_evidence::{
    RenderEvidenceConfig, RenderEvidenceFormat, RenderEvidenceFrame, RenderEvidenceReader,
    RenderEvidenceSummary,
};

#[derive(Debug, Clone, Serialize)]
pub struct NativeRenderEvidenceOrigin {
    pub schema_version: u32,
    pub owner_generation: u64,
    #[serde(serialize_with = "serialize_node")]
    pub node: NodeId,
    pub kind: NativeStreamKind,
    pub format: RenderEvidenceFormat,
    pub config: RenderEvidenceConfig,
}

#[derive(Clone)]
pub(crate) struct RenderEvidenceSource {
    origin: NativeRenderEvidenceOrigin,
    reader: Arc<Mutex<Option<RenderEvidenceReader>>>,
}
impl RenderEvidenceSource {
    #[cfg(any(windows, test))]
    pub(crate) fn new(origin: NativeRenderEvidenceOrigin, reader: RenderEvidenceReader) -> Self {
        Self {
            origin,
            reader: Arc::new(Mutex::new(Some(reader))),
        }
    }
}

pub struct NativeRenderEvidenceReader {
    origin: NativeRenderEvidenceOrigin,
    reader: RenderEvidenceReader,
}
impl NativeRenderEvidenceReader {
    pub fn pop_event(&mut self) -> Option<crate::render_evidence::RenderEvent> {
        self.reader.pop_event()
    }
    pub fn events_finished(&self) -> bool {
        self.reader.events_finished()
    }
    pub fn origin(&self) -> &NativeRenderEvidenceOrigin {
        &self.origin
    }
    pub fn pop(&mut self) -> Option<RenderEvidenceFrame> {
        self.reader.pop()
    }
    pub fn is_finished(&self) -> bool {
        self.reader.is_finished()
    }
    pub fn final_summary(&self) -> Option<RenderEvidenceSummary> {
        self.reader.final_summary()
    }
}
impl NativeObservationReadHandle {
    pub fn render_evidence_origins(&self) -> Vec<NativeRenderEvidenceOrigin> {
        self.0
            .render_evidence
            .iter()
            .map(|source| source.origin.clone())
            .collect()
    }
    pub fn render_evidence_reader(
        &self,
        node: NodeId,
        generation: u64,
    ) -> Result<NativeRenderEvidenceReader, &'static str> {
        if generation != self.0.generation {
            return Err("render evidence owner generation mismatch");
        }
        let source = self
            .0
            .render_evidence
            .iter()
            .find(|source| source.origin.node == node)
            .ok_or("render evidence output identity mismatch")?;
        let reader = source
            .reader
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or("render evidence disabled or already claimed")?;
        Ok(NativeRenderEvidenceReader {
            origin: source.origin.clone(),
            reader,
        })
    }
}
