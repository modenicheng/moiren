use super::*;
use crate::shared_stream::format::{PreparedSharedFormat, catalog_support, test_mix};
use std::rc::Rc;
use windows::{
    Win32::{
        Foundation::HANDLE,
        Media::Audio::{
            AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_SHAREMODE,
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_NOPERSIST,
            AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient_Impl, WAVEFORMATEXTENSIBLE,
            WAVEFORMATEXTENSIBLE_0,
        },
        System::Com::CoTaskMemAlloc,
    },
    core::implement,
};

#[test]
fn catalog_marks_conversion_as_unverified_not_unsupported() {
    for capture in [false, true] {
        let memory = test_mix(96_000, 2, 24, false);
        let snapshot =
            crate::catalog::FormatSnapshot::from_base(unsafe { memory.0.read_unaligned() });
        assert_eq!(catalog_support(&snapshot, capture), (None, true));
        let memory = test_mix(48_000, 2, 32, true);
        let snapshot =
            crate::catalog::FormatSnapshot::from_base(unsafe { memory.0.read_unaligned() });
        assert_eq!(catalog_support(&snapshot, capture), (Some(true), false));
    }
}

#[test]
fn catalog_rejects_a_mix_descriptor_with_inconsistent_byte_rate() {
    let memory = test_mix(48_000, 2, 32, true);
    let mut base = unsafe { memory.0.read_unaligned() };
    base.nAvgBytesPerSec = 1;
    let snapshot = crate::catalog::FormatSnapshot::from_base(base);

    assert_eq!(catalog_support(&snapshot, false), (Some(false), false));
}

#[test]
fn physical_capture_rejects_implicit_surround_downmix() {
    for channels in [4, 6, 8] {
        for float in [false, true] {
            assert!(
                PreparedSharedFormat::from_owned(test_mix(48_000, channels, 32, float), true)
                    .is_err(),
                "capture must reject {channels} channels instead of requesting stereo"
            );
        }
    }
}

#[test]
fn physical_render_requires_a_stereo_endpoint_layout() {
    for channels in [1, 4, 6, 8] {
        assert!(
            PreparedSharedFormat::from_owned(test_mix(48_000, channels, 32, true), false).is_err(),
            "stereo render must not implicitly map to {channels} endpoint channels"
        );
    }
}

#[test]
fn catalog_marks_forbidden_channel_conversion_unsupported() {
    for (capture, channels) in [(true, 4), (true, 6), (true, 8), (false, 1), (false, 6)] {
        let memory = test_mix(48_000, channels, 32, true);
        let snapshot =
            crate::catalog::FormatSnapshot::from_base(unsafe { memory.0.read_unaligned() });
        assert_eq!(catalog_support(&snapshot, capture), (Some(false), false));
    }
}

#[test]
fn converted_mono_capture_preserves_its_single_channel() {
    let format = PreparedSharedFormat::from_owned(test_mix(22_050, 1, 16, false), true).unwrap();
    assert!(format.windows_auto_conversion);
    assert_eq!(format.client().channels, 1);
    let client = unsafe { format.as_ptr().read_unaligned() };
    assert_eq!({ client.nChannels }, 1);
    assert_eq!({ client.nBlockAlign }, 4);
    assert_eq!({ client.nAvgBytesPerSec }, 192_000);
}

#[test]
fn extensible_pcm_valid_bits_are_distinct_from_container_width() {
    for (valid_bits, extra, mask, supported) in [
        (24, 22, 3, true),
        (16, 22, 3, true),
        (0, 22, 3, false),
        (33, 22, 3, false),
        (24, 21, 3, false),
        (24, 22, 4, false),
        (24, 22, 0xc, false),
    ] {
        let memory = TaskMemory(
            unsafe { CoTaskMemAlloc(size_of::<WAVEFORMATEXTENSIBLE>()) }.cast::<WAVEFORMATEX>(),
        );
        assert!(!memory.0.is_null());
        // SAFETY: this test owns the complete extended allocation.
        unsafe {
            memory
                .0
                .cast::<WAVEFORMATEXTENSIBLE>()
                .write_unaligned(WAVEFORMATEXTENSIBLE {
                    Format: WAVEFORMATEX {
                        wFormatTag: 0xfffe,
                        nChannels: 2,
                        nSamplesPerSec: 48_000,
                        nAvgBytesPerSec: 384_000,
                        nBlockAlign: 8,
                        wBitsPerSample: 32,
                        cbSize: extra,
                    },
                    Samples: WAVEFORMATEXTENSIBLE_0 {
                        wValidBitsPerSample: valid_bits,
                    },
                    dwChannelMask: mask,
                    SubFormat: GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71),
                });
        }
        let mut snapshot =
            crate::catalog::FormatSnapshot::from_base(unsafe { memory.0.read_unaligned() });
        snapshot.valid_bits = Some(valid_bits);
        snapshot.channel_mask = Some(mask);
        snapshot.sub_format = Some("00000001-0000-0010-8000-00aa00389b71".into());
        for capture in [false, true] {
            assert_eq!(
                catalog_support(&snapshot, capture),
                if supported {
                    (None, true)
                } else {
                    (Some(false), false)
                }
            );
        }
        let result = PreparedSharedFormat::from_owned(memory, true);
        assert_eq!(result.is_ok(), supported);
        if let Ok(format) = result {
            assert!(format.windows_auto_conversion);
        }
    }
}

#[implement(IAudioClient)]
struct CompatibilityClient {
    flags: Rc<Cell<u32>>,
    error: Option<HRESULT>,
}

impl IAudioClient_Impl for CompatibilityClient_Impl {
    fn Initialize(
        &self,
        mode: AUDCLNT_SHAREMODE,
        flags: u32,
        duration: i64,
        periodicity: i64,
        format: *const WAVEFORMATEX,
        _: *const GUID,
    ) -> Result<(), Error> {
        assert_eq!(mode, AUDCLNT_SHAREMODE_SHARED);
        assert_eq!((duration, periodicity), (0, 0));
        let base = unsafe { format.read_unaligned() };
        assert_eq!(
            ({ base.nSamplesPerSec }, { base.nChannels }, {
                base.wBitsPerSample
            }),
            (48_000, 2, 32)
        );
        self.flags.set(flags);
        self.error
            .map_or(Ok(()), |code| Err(Error::from_hresult(code)))
    }
    fn GetBufferSize(&self) -> Result<u32, Error> {
        unreachable!()
    }
    fn GetStreamLatency(&self) -> Result<i64, Error> {
        unreachable!()
    }
    fn GetCurrentPadding(&self) -> Result<u32, Error> {
        unreachable!()
    }
    fn IsFormatSupported(
        &self,
        _: AUDCLNT_SHAREMODE,
        _: *const WAVEFORMATEX,
        _: *mut *mut WAVEFORMATEX,
    ) -> HRESULT {
        panic!("AUTOCONVERTPCM must use Initialize")
    }
    fn GetMixFormat(&self) -> Result<*mut WAVEFORMATEX, Error> {
        unreachable!()
    }
    fn GetDevicePeriod(&self, _: *mut i64, _: *mut i64) -> Result<(), Error> {
        unreachable!()
    }
    fn Start(&self) -> Result<(), Error> {
        panic!("preparation must not start")
    }
    fn Stop(&self) -> Result<(), Error> {
        unreachable!()
    }
    fn Reset(&self) -> Result<(), Error> {
        unreachable!()
    }
    fn SetEventHandle(&self, _: HANDLE) -> Result<(), Error> {
        unreachable!()
    }
    fn GetService(&self, _: *const GUID, _: *mut *mut std::ffi::c_void) -> Result<(), Error> {
        unreachable!()
    }
}

#[test]
fn initialization_uses_conversion_flags_and_preserves_driver_failures() {
    for error in [
        None,
        Some(AUDCLNT_E_DEVICE_INVALIDATED),
        Some(AUDCLNT_E_DEVICE_IN_USE),
        Some(AUDCLNT_E_UNSUPPORTED_FORMAT),
        Some(windows::Win32::Foundation::E_ACCESSDENIED),
    ] {
        let flags = Rc::new(Cell::new(0));
        let client: IAudioClient = CompatibilityClient {
            flags: flags.clone(),
            error,
        }
        .into();
        let format =
            PreparedSharedFormat::from_owned(test_mix(96_000, 2, 24, false), false).unwrap();
        let mut report = SharedPeriodReport::default();
        let result = initialize_shared(
            &client,
            &format,
            SharedPeriodPolicy::default(),
            &GUID::zeroed(),
            &mut report,
        );
        assert_eq!(
            flags.get(),
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_NOPERSIST
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
        );
        assert!(report.windows_auto_conversion);
        assert_eq!(
            report.fallback_reason,
            Some(SharedFallbackReason::WindowsFormatConversion)
        );
        assert!(report.client3_failure.is_none());
        assert!(report.requested_period_frames.is_none());
        if let Some(code) = error {
            assert_eq!(result.unwrap_err().error.code(), code);
            assert!(report.initialization_api.is_none());
        } else {
            result.unwrap();
            assert_eq!(
                report.initialization_api,
                Some(SharedInitializationApi::LegacyClient)
            );
            assert!(report.actual_period_frames.is_none());
            assert_eq!(report.actual_period_hresult, Some(E_NOINTERFACE.0));
        }
    }
}
