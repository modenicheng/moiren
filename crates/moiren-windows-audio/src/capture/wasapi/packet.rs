//! A packet stays borrowed only until its complete owner-thread ReleaseBuffer.
use super::api;
use crate::{
    capture::CaptureError,
    capture::PcmRecorder,
    capture_packet::borrow_capture_packet,
    clock_bridge::{CaptureIngress, CapturePacket},
    owner::{LeaseFallback, PacketLease},
    stats::SILENT,
};
use windows::Win32::Media::Audio::IAudioCaptureClient;

#[derive(Default)]
pub(super) struct CaptureTransfer {
    pub native_frames: u32,
    pub accepted_frames: usize,
}

pub(super) fn transfer_packet(
    client: &IAudioCaptureClient,
    capacity: u32,
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
