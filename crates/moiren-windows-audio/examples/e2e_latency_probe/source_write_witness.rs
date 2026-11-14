//! One explicit source-buffer witness, not delivery or waveform acceptance.
use super::source::SourceProgress;
use moiren_windows_audio::vad::SampleFormat;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub(super) struct PacketWitness {
    pub format: &'static str,
    pub packet_frames: u32,
    pub packet_bytes: usize,
    pub release_flags: u32,
    pub cursor_before: SourceProgress,
    pub nonzero_frame_offset: usize,
    pub source_frame: u64,
    pub native_submission_frame: u64,
    pub byte_offset: usize,
    pub prefix_bytes: usize,
    pub encoded: [u8; 16],
    pub destination: Option<[u8; 16]>,
    pub qpc_frequency_hz: Option<u64>,
    pub before_copy_qpc: Option<u64>,
    pub after_copy_qpc: Option<u64>,
    pub release_return_qpc: Option<u64>,
    pub release_hresult: Option<i32>,
    pub failure: Option<&'static str>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct WriteWitness {
    pub qualified: bool,
    pub attempted: bool,
    pub failed_attempt: Option<PacketWitness>,
    pub confirmed: Option<PacketWitness>,
}

impl WriteWitness {
    /// Selection and all bounds checks run BEFORE obtaining a native lease.
    pub(super) fn prepare(
        &self,
        format: SampleFormat,
        samples: &[f32],
        encoded: &[u8],
        flags: u32,
        cursor: &SourceProgress,
    ) -> Option<PacketWitness> {
        if self.attempted || flags != 0 || !samples.len().is_multiple_of(2) {
            return None;
        }
        let frame_bytes = 2 * format.bytes_per_sample();
        if encoded.len() != samples.len() / 2 * frame_bytes {
            return None;
        }
        let offset = samples
            .as_chunks::<2>()
            .0
            .iter()
            .zip(encoded.chunks_exact(frame_bytes))
            .position(|(samples, bytes)| {
                samples.iter().any(|sample| *sample != 0.) && bytes.iter().any(|byte| *byte != 0)
            })?;
        let byte_offset = offset.checked_mul(frame_bytes)?;
        let prefix_bytes = (encoded.len() - byte_offset).min(16);
        let mut prefix = [0; 16];
        prefix[..prefix_bytes].copy_from_slice(&encoded[byte_offset..byte_offset + prefix_bytes]);
        Some(PacketWitness {
            format: match format {
                SampleFormat::Pcm16 => "pcm16",
                SampleFormat::Float32 => "float32",
            },
            packet_frames: u32::try_from(samples.len() / 2).ok()?,
            packet_bytes: encoded.len(),
            release_flags: flags,
            cursor_before: cursor.clone(),
            nonzero_frame_offset: offset,
            source_frame: cursor
                .acknowledged_source_frames
                .checked_add(offset as u64)?,
            native_submission_frame: cursor.native_submitted_frames.checked_add(offset as u64)?,
            byte_offset,
            prefix_bytes,
            encoded: prefix,
            destination: None,
            qpc_frequency_hz: None,
            before_copy_qpc: None,
            after_copy_qpc: None,
            release_return_qpc: None,
            release_hresult: None,
            failure: None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SubmitError<E> {
    NullBuffer,
    Release(E),
}

/// # Safety
/// A non-null destination must be the writable GetBuffer lease for exactly
/// encoded.len() bytes, nonoverlapping encoded. The packet must come from prepare
/// for this encoded slice. Clock must not panic. Release is always called once.
pub(super) unsafe fn copy_release<E>(
    encoded: &[u8],
    destination: *mut u8,
    mut witness: Option<(&mut WriteWitness, PacketWitness)>,
    mut clock: impl FnMut() -> Option<u64>,
    release: impl FnOnce() -> Result<(), E>,
    code: impl FnOnce(&E) -> i32,
) -> Result<(), SubmitError<E>> {
    if let Some((owner, packet)) = witness.as_mut() {
        owner.attempted = true;
        packet.before_copy_qpc = clock();
    }
    if !destination.is_null() {
        // SAFETY: caller supplies the exact writable, nonoverlapping lease.
        unsafe { std::ptr::copy_nonoverlapping(encoded.as_ptr(), destination, encoded.len()) };
        if let Some((_, packet)) = witness.as_mut() {
            let mut prefix = [0; 16];
            for (index, byte) in prefix.iter_mut().take(packet.prefix_bytes).enumerate() {
                // SAFETY: prepare checked the <=16B prefix before GetBuffer;
                // this thread still owns the lease. Volatile forces a native
                // destination read rather than substituting known source bytes.
                *byte = unsafe { destination.add(packet.byte_offset + index).read_volatile() };
            }
            packet.destination = Some(prefix);
            packet.after_copy_qpc = clock();
        }
    }
    let result = release();
    if let Some((owner, mut packet)) = witness {
        packet.release_return_qpc = clock();
        packet.release_hresult = Some(result.as_ref().err().map_or(0, code));
        if result.is_err() || destination.is_null() {
            packet.failure = Some(if result.is_err() {
                "release_failed"
            } else {
                "null_destination"
            });
            owner.failed_attempt = Some(packet);
        } else {
            owner.confirmed = Some(packet);
        }
    }
    // Never replace a real Release error with a diagnostic/null-buffer error.
    if result.is_ok() && destination.is_null() {
        return Err(SubmitError::NullBuffer);
    }
    result.map_err(SubmitError::Release)
}

#[cfg(test)]
#[path = "source_write_witness/tests.rs"]
mod tests;
