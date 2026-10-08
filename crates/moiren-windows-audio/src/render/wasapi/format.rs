//! Reject conversion before borrowing driver buffers as native stereo f32.
use super::api;
use crate::{
    owner::TaskMemory,
    render::{CHANNELS, RenderError, RenderReport, SAMPLE_RATE},
};
use windows::{
    Win32::Media::{
        Audio::{IAudioClient, WAVEFORMATEX, WAVEFORMATEXTENSIBLE},
        Multimedia::WAVE_FORMAT_IEEE_FLOAT,
    },
    core::GUID,
};

// WAVEFORMATEX is packed; derive the extension length from the binding layout
// so a truncated driver allocation is never read as WAVEFORMATEXTENSIBLE.
const EXTENSION_BYTES: usize = size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>();
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;

pub(super) fn mix_format(
    client: &IAudioClient,
    report: &mut RenderReport,
) -> Result<TaskMemory<WAVEFORMATEX>, RenderError> {
    let memory = TaskMemory(api("GetMixFormat", unsafe { client.GetMixFormat() })?);
    if memory.0.is_null() {
        return Err(RenderError::UnsupportedFormat);
    }
    // SAFETY: GetMixFormat returns an owned WAVEFORMATEX allocation, with cbSize
    // describing the trailing extension; TaskMemory releases it after use.
    let base = unsafe { memory.0.read_unaligned() };
    report.native_mix_sample_rate = Some(base.nSamplesPerSec);
    report.native_mix_channels = Some(base.nChannels);
    report.native_mix_container_bits = Some(base.wBitsPerSample);
    let (float, valid_bits, mask) = if base.wFormatTag == WAVE_FORMAT_EXTENSIBLE
        && usize::from(base.cbSize) >= EXTENSION_BYTES
    {
        let extended = unsafe { memory.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
        let sub_format = extended.SubFormat;
        (
            sub_format == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71),
            Some(unsafe { extended.Samples.wValidBitsPerSample }),
            Some(extended.dwChannelMask),
        )
    } else {
        (base.wFormatTag == WAVE_FORMAT_IEEE_FLOAT as u16, None, None)
    };
    validate_format(base, float, valid_bits, mask)?;
    report.format_supported = true;
    Ok(memory)
}

pub(super) fn validate_format(
    base: WAVEFORMATEX,
    float: bool,
    valid_bits: Option<u16>,
    mask: Option<u32>,
) -> Result<(), RenderError> {
    if !float
        || base.nSamplesPerSec != SAMPLE_RATE
        || base.nChannels != CHANNELS as u16
        || base.wBitsPerSample != 32
        || base.nBlockAlign != 8
        || base.nAvgBytesPerSec != SAMPLE_RATE * 8
        || valid_bits.is_some_and(|bits| bits != 32)
        || mask.is_some_and(|mask| mask != 0 && mask != 3)
    {
        return Err(RenderError::UnsupportedFormat);
    }
    Ok(())
}
