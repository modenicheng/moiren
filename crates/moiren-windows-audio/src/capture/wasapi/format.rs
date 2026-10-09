//! Validate the owned native format before reading any packet bytes.
use super::api;
use crate::{capture::CaptureError, owner::TaskMemory};
use windows::{
    Win32::Media::{
        Audio::{IAudioClient, WAVEFORMATEX, WAVEFORMATEXTENSIBLE},
        Multimedia::WAVE_FORMAT_IEEE_FLOAT,
    },
    core::GUID,
};

const EXTENSION_BYTES: usize = size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>();

pub(super) fn mix_format(client: &IAudioClient) -> Result<TaskMemory<WAVEFORMATEX>, CaptureError> {
    let memory = TaskMemory(api("GetMixFormat(capture)", unsafe {
        client.GetMixFormat()
    })?);
    if memory.0.is_null() {
        return Err(CaptureError::UnsupportedFormat);
    }
    // SAFETY: the owned GetMixFormat allocation includes the base and cbSize
    // extension. Packed structures must be read without alignment assumptions.
    let base = unsafe { memory.0.read_unaligned() };
    let (float, bits, mask) =
        if base.wFormatTag == 0xfffe && usize::from(base.cbSize) >= EXTENSION_BYTES {
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
    validate_format(base, float, bits, mask)?;
    Ok(memory)
}
pub(super) fn validate_format(
    base: WAVEFORMATEX,
    float: bool,
    bits: Option<u16>,
    mask: Option<u32>,
) -> Result<(), CaptureError> {
    let valid_mask = if base.nChannels == 1 { 4 } else { 3 };
    let rate = base.nSamplesPerSec;
    let channels = base.nChannels;
    if !float
        || ![44_100, 48_000].contains(&rate)
        || !(1..=2).contains(&channels)
        || base.wBitsPerSample != 32
        || base.nBlockAlign != base.nChannels * 4
        || base.nAvgBytesPerSec != base.nSamplesPerSec * u32::from(base.nChannels) * 4
        || bits.is_some_and(|bits| bits != 32)
        || mask.is_some_and(|mask| mask != 0 && mask != valid_mask)
    {
        return Err(CaptureError::UnsupportedFormat);
    }
    Ok(())
}
