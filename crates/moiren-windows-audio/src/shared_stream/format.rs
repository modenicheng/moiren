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

impl SharedFormatInfo {
    pub(crate) fn native_supported(self, capture: bool) -> bool {
        self.floating_point
            && self.container_bits == 32
            && self.valid_bits.is_none_or(|bits| bits == 32)
            && if capture {
                [44_100, 48_000].contains(&self.sample_rate)
                    && (1..=2).contains(&self.channels)
                    && self.channel_mask.is_none_or(|mask| {
                        mask == 0 || mask == if self.channels == 1 { 4 } else { 3 }
                    })
            } else {
                self.sample_rate == 48_000
                    && self.channels == 2
                    && self.channel_mask.is_none_or(|mask| mask == 0 || mask == 3)
            }
    }
}

pub(crate) struct PreparedSharedFormat {
    native: TaskMemory<WAVEFORMATEX>,
    converted: WAVEFORMATEX,
    pub mix: SharedFormatInfo,
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
        validate_layout(mix, base.nBlockAlign, base.nAvgBytesPerSec)?;
        Ok(Self {
            native,
            converted: float_stereo_format(),
            mix,
            windows_auto_conversion: !mix.native_supported(capture),
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
        if self.windows_auto_conversion {
            SharedFormatInfo {
                sample_rate: 48_000,
                channels: 2,
                container_bits: 32,
                valid_bits: None,
                channel_mask: None,
                floating_point: true,
            }
        } else {
            self.mix
        }
    }
}

pub(crate) fn float_stereo_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 384_000,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

fn validate_layout(info: SharedFormatInfo, align: u16, bytes_per_second: u32) -> Result<()> {
    let valid_container = if info.floating_point {
        [32, 64].contains(&info.container_bits)
    } else {
        [8, 16, 24, 32].contains(&info.container_bits)
    };
    let expected_align = info.channels.checked_mul(info.container_bits / 8);
    if info.sample_rate == 0
        || !(1..=32).contains(&info.channels)
        || !valid_container
        || info.valid_bits.is_some_and(|bits| {
            bits == 0
                || bits > info.container_bits
                || (info.floating_point && bits != info.container_bits)
        })
        || info
            .channel_mask
            .is_some_and(|mask| mask != 0 && mask.count_ones() != u32::from(info.channels))
        || expected_align != Some(align)
        || info.sample_rate.checked_mul(u32::from(align)) != Some(bytes_per_second)
    {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    Ok(())
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
        || validate_layout(info, format.block_align, format.avg_bytes_per_sec).is_err()
    {
        return (Some(false), false);
    }
    if info.native_supported(capture) {
        (Some(true), false)
    } else {
        (None, true)
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
