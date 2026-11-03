//! Lease-local staging: copy before release, publish the exact outcome afterward.
use super::*;
use crate::{owner::LeaseClient, render_evidence::RenderEvidenceWriter};

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
) -> windows::core::HRESULT {
    // This call must have no reference parameter into the native buffer: the
    // stage helper returns first, ending its borrow before COM invalidates it.
    let hr = lease.release_hresult();
    // Never access bytes here: ReleaseBuffer has invalidated the native lease.
    // Record even a failure before the caller maps it with `?`.
    if staged {
        let _ = writer.unwrap().finish_packet(hr.0);
    }
    hr
}
