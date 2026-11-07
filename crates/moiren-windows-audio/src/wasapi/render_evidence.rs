//! Lease-local staging: copy before release, publish the exact outcome afterward.
use super::*;
use crate::{owner::LeaseClient, render_evidence::RenderEvidenceWriter};

/// Scalar envelope only: no native calls, allocation or reclamation. The clock
/// closures are never invoked when the existing evidence hook is disabled.
pub(super) struct RenderOperationEvidence {
    enabled: bool,
    pub sample: crate::render_evidence::RenderEventSample,
    pub packet_sequence: Option<u64>,
}

impl RenderOperationEvidence {
    pub fn begin(
        enabled: bool,
        kind: crate::render_evidence::RenderEventKind,
        now: impl FnOnce() -> Option<u64>,
    ) -> Self {
        Self {
            enabled,
            sample: crate::render_evidence::RenderEventSample {
                kind,
                qpc_before_100ns: if enabled { now() } else { None },
                ..Default::default()
            },
            packet_sequence: None,
        }
    }

    pub fn finish(
        mut self,
        writer: Option<&mut RenderEvidenceWriter>,
        failed: bool,
        now: impl FnOnce() -> Option<u64>,
    ) {
        if self.enabled {
            self.sample.qpc_after_100ns = now();
            self.sample.failed = failed;
            if let Some(writer) = writer {
                writer.record_packet_event(self.sample, self.packet_sequence);
            }
        }
    }
}

pub(super) fn negotiated_format(
    codec: SampleCodec,
    sample_rate: u32,
) -> crate::render_evidence::RenderEvidenceFormat {
    let (sample_format, channels, container_bits, valid_bits) = codec.encoded_layout();
    crate::render_evidence::RenderEvidenceFormat {
        sample_rate,
        sample_format,
        channels,
        container_bits,
        valid_bits,
    }
}

pub(super) fn validate_render_evidence_attachment(
    writer: &RenderEvidenceWriter,
    codec: SampleCodec,
    sample_rate: u32,
    max_packet_frames: usize,
    started: bool,
    render: bool,
    attached: bool,
) -> Result<(), BackendError> {
    let summary = writer.snapshot();
    if started
        || !render
        || attached
        || !writer.is_fresh()
        || summary.format != negotiated_format(codec, sample_rate)
        || summary.config.max_packet_frames < max_packet_frames
    {
        return Err(unsupported(
            "render evidence requires a fresh exact-layout writer before Start, sized for the native packet",
        ));
    }
    Ok(())
}

pub(super) fn stage_render_evidence(
    bytes: &[u8],
    frames: usize,
    mut writer: Option<&mut RenderEvidenceWriter>,
) -> bool {
    if let Some(writer) = writer.as_mut() {
        match writer.stage_packet(bytes, frames) {
            Ok(()) => true,
            Err(_) => {
                writer.record_unobserved(frames);
                false
            }
        }
    } else {
        false
    }
}

pub(super) fn release_render_evidence<C: LeaseClient>(
    lease: PacketLease<'_, C>,
    staged: bool,
    writer: Option<&mut RenderEvidenceWriter>,
) -> (windows::core::HRESULT, Option<u64>) {
    // This call must have no reference parameter into the native buffer: the
    // stage helper returns first, ending its borrow before COM invalidates it.
    let hr = lease.release_hresult();
    // Never access bytes here: ReleaseBuffer has invalidated the native lease.
    // Record even a failure before the caller maps it with `?`.
    let packet_sequence = if staged {
        let writer = writer.expect("staged evidence retains its writer");
        writer
            .finish_packet(hr.0)
            .ok()
            .map(|_| writer.snapshot().attempts)
    } else {
        None
    };
    (hr, packet_sequence)
}
