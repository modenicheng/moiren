//! Validation and borrowing for an already-acquired WASAPI capture packet.
//!
//! The COM owner remains responsible for the packet lease and ReleaseBuffer.
//! This helper only centralizes the byte-width and pointer checks that must
//! happen while the native packet is borrowed.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapturePacketError {
    ExceedsCapacity,
    NullData,
    ZeroFrameBytes,
    SizeOverflow,
}

pub(crate) fn packet_byte_len(
    frames: u32,
    frame_bytes: usize,
) -> Result<usize, CapturePacketError> {
    if frame_bytes == 0 {
        return Err(CapturePacketError::ZeroFrameBytes);
    }
    (frames as usize)
        .checked_mul(frame_bytes)
        .filter(|&bytes| bytes <= isize::MAX as usize)
        .ok_or(CapturePacketError::SizeOverflow)
}

/// Borrow the bytes covered by a successful WASAPI GetBuffer call.
///
/// # Safety
///
/// data must point to the native packet returned by the matching GetBuffer
/// call and remain valid for frames * frame_bytes bytes until the caller
/// releases that packet. The caller must keep the returned slice within the
/// lease lifetime and must call ReleaseBuffer exactly once.
pub(crate) unsafe fn borrow_capture_packet<'a>(
    data: *const u8,
    frames: u32,
    capacity: u32,
    silent: bool,
    frame_bytes: usize,
) -> Result<&'a [u8], CapturePacketError> {
    if frames > capacity {
        return Err(CapturePacketError::ExceedsCapacity);
    }
    let bytes = packet_byte_len(frames, frame_bytes)?;
    if frames == 0 || silent {
        return Ok(&[]);
    }
    if data.is_null() {
        return Err(CapturePacketError::NullData);
    }
    // SAFETY: The caller provides the live GetBuffer lease and its validated
    // frame count; the checked length is bounded by isize::MAX.
    Ok(unsafe { std::slice::from_raw_parts(data, bytes) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_packet_uses_the_supplied_frame_width() {
        let data = [1_u8, 2, 3, 4, 5, 6];
        let packet = unsafe { borrow_capture_packet(data.as_ptr(), 2, 2, false, 3) }.unwrap();
        assert_eq!(packet, &data);
    }

    #[test]
    fn borrowed_packet_rejects_capacity_null_and_size_errors() {
        let data = [0_u8; 4];
        assert_eq!(
            unsafe { borrow_capture_packet(data.as_ptr(), 2, 1, false, 2) },
            Err(CapturePacketError::ExceedsCapacity)
        );
        assert_eq!(
            unsafe { borrow_capture_packet(std::ptr::null(), 1, 1, false, 4) },
            Err(CapturePacketError::NullData)
        );
        assert_eq!(
            packet_byte_len(u32::MAX, usize::MAX),
            Err(CapturePacketError::SizeOverflow)
        );
        assert_eq!(
            packet_byte_len(1, 0),
            Err(CapturePacketError::ZeroFrameBytes)
        );
    }

    #[test]
    fn silent_packet_does_not_borrow_or_require_a_data_pointer() {
        let packet = unsafe { borrow_capture_packet(std::ptr::null(), 480, 480, true, 8) }.unwrap();
        assert!(packet.is_empty());
    }
}
