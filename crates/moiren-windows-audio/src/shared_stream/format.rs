//! Owner-held descriptors; conversion changes the client format, never the
//! interpretation of a native driver buffer after Start.
use super::SharedFormatInfo;
use crate::{catalog::FormatSnapshot, owner::TaskMemory};
use windows::{
    Win32::{
        Foundation::E_INVALIDARG,
        Media::Audio::{WAVEFORMATEX, WAVEFORMATEXTENSIBLE},
    },
    core::{Error, GUID, Result},
};

const EXTENSIBLE: u16 = 0xfffe;
const FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const EXTENSION_BYTES: usize = size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>();

pub(crate) struct PreparedSharedFormat {
    native: TaskMemory<WAVEFORMATEX>,
    converted: WAVEFORMATEX,
    pub mix: SharedFormatInfo,
    client: SharedFormatInfo,
    pub windows_auto_conversion: bool,
}

impl PreparedSharedFormat {
    pub fn from_owned(native: TaskMemory<WAVEFORMATEX>, capture: bool) -> Result<Self> {
        if native.0.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: GetMixFormat owns the base plus cbSize bytes. Copy packed
        // fields before inspecting the extension, without alignment assumptions.
        let base = unsafe { native.0.read_unaligned() };
        let mut valid_bits = None;
        let mut channel_mask = None;
        let floating_point = match base.wFormatTag {
            1 => false,
            3 => true,
            EXTENSIBLE if usize::from(base.cbSize) >= EXTENSION_BYTES => {
                let extended = unsafe { native.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
                valid_bits = Some(unsafe { extended.Samples.wValidBitsPerSample });
                channel_mask = Some(extended.dwChannelMask);
                let sub = extended.SubFormat;
                if sub != FLOAT && sub != PCM {
                    return Err(E_INVALIDARG.into());
                }
                sub == FLOAT
            }
            _ => return Err(E_INVALIDARG.into()),
        };
        let mix = SharedFormatInfo {
            sample_rate: base.nSamplesPerSec,
            channels: base.nChannels,
            container_bits: base.wBitsPerSample,
            valid_bits,
            channel_mask,
            floating_point,
        };
        if !mix.valid_layout(base.nBlockAlign, base.nAvgBytesPerSec) {
            return Err(E_INVALIDARG.into());
        }
        let client = mix
            .select_client(capture)
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        Ok(Self {
            native,
            converted: float_format(client.channels),
            mix,
            client,
            windows_auto_conversion: client != mix,
        })
    }

    pub fn as_ptr(&self) -> *const WAVEFORMATEX {
        if self.windows_auto_conversion {
            &self.converted
        } else {
            self.native.0
        }
    }

    pub fn client(&self) -> SharedFormatInfo {
        self.client
    }
}

fn float_format(channels: u16) -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: channels,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * u32::from(channels) * 4,
        nBlockAlign: channels * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

/// Discovery cannot confirm Windows conversion without opening a stream.
/// None means a valid descriptor that still requires initialization testing.
pub(crate) fn catalog_support(format: &FormatSnapshot, capture: bool) -> (Option<bool>, bool) {
    let float = format.format_tag == 3
        || (format.format_tag == EXTENSIBLE
            && format
                .sub_format
                .as_ref()
                .is_some_and(|s| s.eq_ignore_ascii_case("00000003-0000-0010-8000-00aa00389b71")));
    let pcm = format.format_tag == 1
        || (format.format_tag == EXTENSIBLE
            && format
                .sub_format
                .as_ref()
                .is_some_and(|s| s.eq_ignore_ascii_case("00000001-0000-0010-8000-00aa00389b71")));
    let info = SharedFormatInfo {
        sample_rate: format.sample_rate,
        channels: format.channels,
        container_bits: format.container_bits,
        valid_bits: format.valid_bits,
        channel_mask: format.channel_mask,
        floating_point: float,
    };
    if (!float && !pcm)
        || (format.format_tag == EXTENSIBLE && usize::from(format.extra_bytes) < EXTENSION_BYTES)
        || !info.valid_layout(format.block_align, format.avg_bytes_per_sec)
    {
        return (Some(false), false);
    }
    match info.select_client(capture) {
        None => (Some(false), false),
        Some(client) if client == info => (Some(true), false),
        Some(_) => (None, true),
    }
}

#[cfg(test)]
pub(super) fn test_mix(
    rate: u32,
    channels: u16,
    bits: u16,
    float: bool,
) -> TaskMemory<WAVEFORMATEX> {
    use windows::Win32::System::Com::CoTaskMemAlloc;
    let memory =
        TaskMemory(unsafe { CoTaskMemAlloc(size_of::<WAVEFORMATEX>()) }.cast::<WAVEFORMATEX>());
    assert!(!memory.0.is_null());
    let align = channels * (bits / 8);
    unsafe {
        memory.0.write_unaligned(WAVEFORMATEX {
            wFormatTag: if float { 3 } else { 1 },
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * u32::from(align),
            nBlockAlign: align,
            wBitsPerSample: bits,
            cbSize: 0,
        });
    }
    memory
}
