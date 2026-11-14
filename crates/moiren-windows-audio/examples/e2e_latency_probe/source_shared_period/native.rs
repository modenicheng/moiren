//! Preparation-only COM calls; all allocated descriptors stay on this owner.
use super::*;
use anyhow::Context;
use windows::{
    Win32::{
        Media::Audio::{
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient, IAudioClient3, WAVEFORMATEX,
            WAVEFORMATEXTENSIBLE,
        },
        System::Com::CoTaskMemFree,
    },
    core::{GUID, Interface},
};

pub(super) struct Mix(pub *mut WAVEFORMATEX);
impl Drop for Mix {
    fn drop(&mut self) {
        // SAFETY: GetMixFormat/GetCurrentSharedModeEnginePeriod transfer this
        // allocation to the caller, including allocations returned on failure.
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

/// Read raw engine format evidence before comparing it with the source mix.
pub(super) fn describe(pointer: *const WAVEFORMATEX) -> Result<SharedFormatInfo> {
    ensure!(!pointer.is_null(), "null source Shared format");
    // SAFETY: callers retain the native descriptor allocation for this read.
    let wave = unsafe { pointer.read_unaligned() };
    let bits = wave.wBitsPerSample;
    let (floating_point, valid_bits, channel_mask) = match (wave.wFormatTag, bits) {
        (1, _) => (false, Some(bits), (wave.nChannels == 2).then_some(3)),
        (3, _) => (true, Some(bits), (wave.nChannels == 2).then_some(3)),
        (0xfffe, _) if wave.cbSize == 22 => {
            // SAFETY: extensible tag and size include the complete extension.
            let ext = unsafe { pointer.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
            let subtype = ext.SubFormat;
            let floating = match subtype {
                id if id == GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71) => false,
                id if id == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71) => true,
                _ => anyhow::bail!("unsupported shared mix subtype"),
            };
            (
                floating,
                Some(unsafe { ext.Samples.wValidBitsPerSample }),
                Some(ext.dwChannelMask),
            )
        }
        _ => anyhow::bail!("unsupported shared source mix format"),
    };
    ensure!(
        wave.nChannels > 0
            && wave.nSamplesPerSec > 0
            && bits > 0
            && bits.is_multiple_of(8)
            && u32::from(wave.nBlockAlign) == u32::from(wave.nChannels) * u32::from(bits / 8)
            && u64::from(wave.nAvgBytesPerSec)
                == u64::from(wave.nSamplesPerSec) * u64::from(wave.nBlockAlign),
        "invalid shared engine alignment"
    );
    Ok(SharedFormatInfo {
        sample_rate: wave.nSamplesPerSec,
        channels: wave.nChannels,
        container_bits: bits,
        valid_bits,
        channel_mask,
        floating_point,
    })
}

pub(crate) fn initialize(
    client: &IAudioClient,
    wave: *const WAVEFORMATEX,
    requested: u32,
    format: SharedFormatInfo,
    report: &mut SharedPeriodReport,
) -> Result<()> {
    let client3 = match client.cast::<IAudioClient3>() {
        Ok(client3) => client3,
        Err(error) => {
            *report = SharedPeriodReport {
                requested_period_frames: Some(requested),
                mix_format: Some(format),
                client_format: Some(format),
                client3_failure: Some(SharedInitializationFailure {
                    stage: SharedInitializationStage::InterfaceQuery,
                    hresult: error.code().0,
                }),
                ..Default::default()
            };
            return Err(error).context("source IAudioClient3 required; no legacy fallback");
        }
    };
    super::prepare_exact(
        requested,
        format,
        report,
        || {
            let mut caps = SharedPeriodCaps {
                default_frames: 0,
                fundamental_frames: 0,
                min_frames: 0,
                max_frames: 0,
            };
            // SAFETY: mix descriptor and output storage live through this call.
            unsafe {
                client3.GetSharedModeEnginePeriod(
                    wave,
                    &mut caps.default_frames,
                    &mut caps.fundamental_frames,
                    &mut caps.min_frames,
                    &mut caps.max_frames,
                )
            }
            .map_err(|error| error.code().0)?;
            Ok(caps)
        },
        |period| {
            // Explicit requests never retry a locked engine at another period.
            unsafe {
                client3.InitializeSharedAudioStream(
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                    period,
                    wave,
                    None,
                )
            }
            .map_err(|error| error.code().0)
        },
        || {
            let mut pointer = std::ptr::null_mut();
            let mut frames = 0;
            let result =
                unsafe { client3.GetCurrentSharedModeEnginePeriod(&mut pointer, &mut frames) };
            let memory = Mix(pointer);
            result.map_err(|error| ReadbackError::Native(error.code().0))?;
            let format =
                describe(memory.0).map_err(|error| ReadbackError::Descriptor { frames, error })?;
            Ok((frames, format))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_shared_period_descriptor_reads_actual_format_without_substituting_mix() {
        let mut wave = WAVEFORMATEX {
            wFormatTag: 3,
            nChannels: 1,
            nSamplesPerSec: 96_000,
            nAvgBytesPerSec: 384_000,
            nBlockAlign: 4,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let actual = describe(&wave).unwrap();
        assert_eq!(actual.sample_rate, 96_000);
        assert_eq!(actual.channels, 1);
        assert_eq!(actual.channel_mask, None);
        assert!(actual.floating_point);
        wave.nBlockAlign = 2;
        assert!(describe(&wave).is_err());
        assert!(describe(std::ptr::null()).is_err());
    }
}
