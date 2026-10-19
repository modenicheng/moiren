//! A render packet is released even when validation aborts the submission.
use super::api;
use crate::{
    owner::{LeaseFallback, PacketLease},
    render::{CHANNELS, RenderError},
};
use windows::Win32::{Foundation::E_POINTER, Media::Audio::IAudioRenderClient};

pub(super) fn submit(client: &IAudioRenderClient, samples: &[f32]) -> Result<(), RenderError> {
    if samples.is_empty() {
        return Ok(());
    }
    if !samples.len().is_multiple_of(CHANNELS) {
        return Err(RenderError::InvalidSamples);
    }
    let frames =
        u32::try_from(samples.len() / CHANNELS).map_err(|_| RenderError::InvalidSamples)?;
    let data = api("GetBuffer(render)", unsafe { client.GetBuffer(frames) })?;
    let lease = PacketLease::new(client, frames, LeaseFallback::RenderEmpty);
    if data.is_null() {
        return Err(RenderError::Api {
            stage: "null render buffer",
            hresult: E_POINTER.0,
        });
    }
    // SAFETY: A validated native f32 stereo WASAPI lease provides frames * 8
    // writable bytes on this owner thread. Source is separately owned staging;
    // neither pointer escapes and the full packet is released below.
    unsafe {
        std::ptr::copy_nonoverlapping(
            samples.as_ptr().cast::<u8>(),
            data,
            std::mem::size_of_val(samples),
        );
    }
    api("ReleaseBuffer(render)", lease.release())
}
