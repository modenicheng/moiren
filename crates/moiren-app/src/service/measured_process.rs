//! Session-local measured selections; no trusted evidence is deserialized.
use super::{InputSelection, SessionGeneration};
use moiren_core::graph::NodeId;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasuredProcessRequest {
    pub node: NodeId,
    pub session_generation: SessionGeneration,
    pub native_owner_generation: u64,
    pub graph_revision: u64,
    pub capacity_frames: u32,
    pub target_frames: u32,
    pub jitter_frames: u32,
}

impl MeasuredProcessRequest {
    pub fn validate(self) -> Result<(), String> {
        if self.session_generation.0 == 0
            || self.native_owner_generation == 0
            || self.graph_revision == 0
            || !(4..=262_144).contains(&self.capacity_frames)
            || !(2..self.capacity_frames).contains(&self.target_frames)
            || self.jitter_frames >= self.capacity_frames
        {
            return Err("invalid measured Process origin or bounded target".into());
        }
        Ok(())
    }
}

impl Serialize for MeasuredProcessRequest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("MeasuredProcessRequest", 7)?;
        value.serialize_field("node", &format_args!("{:?}", self.node))?;
        value.serialize_field("session_generation", &self.session_generation.0)?;
        value.serialize_field("native_owner_generation", &self.native_owner_generation)?;
        value.serialize_field("graph_revision", &self.graph_revision)?;
        value.serialize_field("capacity_frames", &self.capacity_frames)?;
        value.serialize_field("target_frames", &self.target_frames)?;
        value.serialize_field("jitter_frames", &self.jitter_frames)?;
        value.end()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MeasuredDeliveryBound {
    pub sample_rate: u32,
    /// Whole native drain, not only its largest constituent packet.
    pub max_packet_frames: u32,
    pub max_gap_frames: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeasuredProcessSummary {
    pub selection_id: u64,
    pub request: MeasuredProcessRequest,
    pub process_pid: u32,
    pub process_creation_time_100ns: u64,
    pub delivery: MeasuredDeliveryBound,
}

#[derive(Debug, Clone)]
pub(crate) struct MeasuredProcessProfile {
    summary: MeasuredProcessSummary,
    #[cfg(windows)]
    evidence: moiren_windows_audio::process_loopback::MeasuredProcessDelivery,
}

impl MeasuredProcessProfile {
    pub(crate) fn summary(&self) -> &MeasuredProcessSummary {
        &self.summary
    }

    pub(crate) fn validate_source(&self, source: &InputSelection) -> Result<(), String> {
        if *source
            != (InputSelection::Process {
                pid: self.summary.process_pid,
                creation_time_100ns: self.summary.process_creation_time_100ns,
            })
        {
            return Err("measured Process source was removed or replaced".into());
        }
        Ok(())
    }

    #[cfg(windows)]
    fn policy(
        request: MeasuredProcessRequest,
    ) -> moiren_windows_audio::latency::transport::TransportInputPolicy {
        moiren_windows_audio::latency::transport::TransportInputPolicy::Target {
            capacity_frames: request.capacity_frames,
            target_frames: request.target_frames,
            jitter_frames: request.jitter_frames,
        }
    }

    #[cfg(windows)]
    pub(crate) fn from_observation(
        selection_id: u64,
        request: MeasuredProcessRequest,
        observation: &moiren_windows_audio::engine_backend::NativeProcessDeliveryObservation,
    ) -> Result<Self, String> {
        request.validate()?;
        if selection_id == 0 || observation.node != request.node {
            return Err("measured Process node or selection identity mismatch".into());
        }
        let evidence = observation
            .measured(&observation.process, request.native_owner_generation)
            .map_err(|error| error.to_string())?;
        let options = evidence
            .admit_for_consumer(
                &observation.process,
                request.native_owner_generation,
                Self::policy(request),
                observation.consumer_sample_rate,
                observation.prepared_consumer_demand_frames,
            )
            .map_err(|error| error.to_string())?;
        let bound = options
            .delivery
            .expect("typed measured evidence requires delivery");
        Ok(Self {
            summary: MeasuredProcessSummary {
                selection_id,
                request,
                process_pid: observation.process.pid,
                process_creation_time_100ns: observation.process.creation_time_100ns,
                delivery: MeasuredDeliveryBound {
                    sample_rate: bound.sample_rate,
                    max_packet_frames: bound.max_packet_frames,
                    max_gap_frames: bound.max_gap_frames,
                },
            },
            evidence,
        })
    }

    #[cfg(windows)]
    pub(crate) fn configure<S: moiren_engine::sample::ProcessingSample>(
        &self,
        backend: moiren_windows_audio::engine_backend::ProfessionalBackend<S>,
    ) -> Result<
        moiren_windows_audio::engine_backend::ProfessionalBackend<S>,
        moiren_engine::host::BackendError,
    > {
        backend.with_measured_process_bridge(
            self.summary.request.node,
            self.evidence.process(),
            self.summary.request.native_owner_generation,
            self.evidence.clone(),
            Self::policy(self.summary.request),
        )
    }
}

#[cfg(all(test, windows))]
pub(crate) mod tests;
