//! A packet stays borrowed only until its complete owner-thread ReleaseBuffer.
use super::api;
use crate::{
    capture::CaptureError,
    capture::PcmRecorder,
    capture_packet::{borrow_capture_packet, packet_byte_len},
    clock_bridge::{CaptureIngress, CapturePacket},
    owner::{LeaseFallback, PacketLease},
    stats::SILENT,
};
use windows::Win32::Media::Audio::IAudioCaptureClient;

pub(super) const PROCESS_PACKET_SAFETY_BOUND_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) enum PacketBound {
    NativeCapacity(u32),
    /// An explicit processing limit; it says nothing about native capacity.
    ProcessSafety,
}

#[derive(Default)]
pub(super) struct CaptureTransfer {
    pub native_frames: u32,
    pub accepted_frames: usize,
}

pub(super) fn transfer_packet(
    client: &IAudioCaptureClient,
    bound: PacketBound,
    ingress: &mut CaptureIngress,
    mut artifact: Option<&mut PcmRecorder>,
    evidence: Option<&mut crate::capture::PcmEvidenceWriter>,
) -> Result<CaptureTransfer, CaptureError> {
    let (mut data, mut frames, mut flags, mut position, mut qpc) =
        (std::ptr::null_mut(), 0, 0, 0, 0);
    api("GetBuffer(capture)", unsafe {
        client.GetBuffer(
            &mut data,
            &mut frames,
            &mut flags,
            Some(&mut position),
            Some(&mut qpc),
        )
    })?;
    // AUDCLNT_S_BUFFER_EMPTY has no lease; outputs other than frames are undefined.
    if frames == 0 {
        return Ok(CaptureTransfer::default());
    }
    let lease = PacketLease::new(client, frames, LeaseFallback::CapturePacket);
    let frame_bytes = ingress
        .input_channels()
        .checked_mul(4)
        .ok_or(CaptureError::InvalidPacket)?;
    let capacity = match bound {
        PacketBound::NativeCapacity(capacity) => capacity,
        PacketBound::ProcessSafety => {
            let packet_bytes = packet_byte_len(frames, frame_bytes).ok();
            if packet_bytes.is_none_or(|bytes| bytes > PROCESS_PACKET_SAFETY_BOUND_BYTES) {
                // Never borrow, copy, or publish an oversized packet, even if
                // SILENT. Release its complete lease once and retain both the
                // primary refusal and any terminal ReleaseBuffer failure.
                let release_hresult = lease.release().err().map(|error| error.code().0);
                return Err(CaptureError::ProcessPacketBound {
                    frames,
                    frame_bytes,
                    packet_bytes,
                    limit_bytes: PROCESS_PACKET_SAFETY_BOUND_BYTES,
                    release_hresult,
                });
            }
            // Only the shared borrowing helper needs a frame bound. This
            // derived processing bound is never published as native timing.
            (PROCESS_PACKET_SAFETY_BOUND_BYTES / frame_bytes) as u32
        }
    };
    // SAFETY: data is borrowed from this live GetBuffer lease until the
    // ReleaseBuffer call below.
    let bytes =
        unsafe { borrow_capture_packet(data, frames, capacity, flags & SILENT != 0, frame_bytes) }
            .map_err(|_| CaptureError::InvalidPacket)?;
    if let Some(artifact) = artifact.as_mut() {
        artifact.record_packet(bytes, frames, flags, position)?;
    }
    if let Some(evidence) = evidence {
        evidence.record_packet(
            bytes,
            crate::capture::PcmEvidencePacket {
                frames,
                flags,
                device_position_frames: position,
                qpc_100ns: qpc,
            },
        )?;
    }
    let accepted_frames = ingress.push_packet(
        bytes,
        CapturePacket {
            frames: frames as usize,
            flags,
            device_position_frames: position,
            qpc_100ns: qpc,
        },
    )?;
    // Release the entire packet even when bridge overflow discarded its tail.
    api("ReleaseBuffer(capture)", lease.release())?;
    Ok(CaptureTransfer {
        native_frames: frames,
        accepted_frames,
    })
}
