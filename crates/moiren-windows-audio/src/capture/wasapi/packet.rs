//! A packet stays borrowed only until its complete owner-thread ReleaseBuffer.
use super::api;
use crate::{
    capture::CaptureError,
    clock_bridge::{CaptureIngress, CapturePacket},
    owner::PacketLease,
    stats::SILENT,
};
use windows::Win32::Media::Audio::IAudioCaptureClient;

pub(super) fn transfer_packet(
    client: &IAudioCaptureClient,
    capacity: u32,
    ingress: &mut CaptureIngress,
) -> Result<usize, CaptureError> {
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
        return Ok(0);
    }
    let lease = PacketLease {
        client,
        frames,
        released: false,
    };
    if frames > capacity || (flags & SILENT == 0 && data.is_null()) {
        return Err(CaptureError::InvalidPacket);
    }
    let bytes = if flags & SILENT != 0 {
        &[][..]
    } else {
        let count = (frames as usize)
            .checked_mul(ingress.input_channels() * 4)
            .filter(|&n| n <= isize::MAX as usize)
            .ok_or(CaptureError::InvalidPacket)?;
        // SAFETY: validated native f32 layout and bounded frame count describe
        // this live WASAPI lease. Decoder uses bytes, not aligned f32 references.
        unsafe { std::slice::from_raw_parts(data, count) }
    };
    ingress.push_packet(
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
    Ok(frames as usize)
}
