//! Prepare the client format before borrowing any driver packet.
use crate::{
    owner::TaskMemory,
    render::{RenderError, RenderReport},
    shared_stream::format::PreparedSharedFormat,
};
use windows::Win32::Media::Audio::WAVEFORMATEX;

pub(super) fn mix_format(
    memory: TaskMemory<WAVEFORMATEX>,
    report: &mut RenderReport,
) -> Result<PreparedSharedFormat, RenderError> {
    if memory.0.is_null() {
        return Err(RenderError::UnsupportedFormat);
    }
    // SAFETY: GetMixFormat returns an owned WAVEFORMATEX allocation, with cbSize
    // describing the trailing extension; TaskMemory releases it after use.
    let base = unsafe { memory.0.read_unaligned() };
    report.native_mix_sample_rate = Some(base.nSamplesPerSec);
    report.native_mix_channels = Some(base.nChannels);
    report.native_mix_container_bits = Some(base.wBitsPerSample);
    PreparedSharedFormat::from_owned(memory, false).map_err(|_| RenderError::UnsupportedFormat)
}
