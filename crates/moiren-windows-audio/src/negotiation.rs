//! Shared WASAPI negotiation kernel.
//!
//! Single source for the pieces the retired dual stacks had duplicated:
//! PCM/floating-point sub-format GUIDs, WAVEFORMATEX descriptor parsing, the
//! IAudioClient3 rejection matrix, endpoint friendly names, and the shared
//! and exclusive initialization sequences (deliberate divergences stay
//! explicit parameters; see decision D4 in
//! `docs/superpowers/plans/2026-10-19-repo-steward-remediation.md`).

#[cfg(feature = "probe-tools")]
use windows::Win32::Media::Audio::EDataFlow;
use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::{E_NOINTERFACE, E_NOTIMPL, E_UNEXPECTED, S_OK},
        Media::Audio::{
            AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_NOPERSIST, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient,
            IAudioClient3, IMMDevice, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
        },
        System::{Com::STGM_READ, Variant::VT_LPWSTR},
    },
    core::GUID,
};

/// KSDATAFORMAT_SUBTYPE_PCM: integer samples in the container.
pub(crate) const SUBFORMAT_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
/// KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: floating-point samples in the container.
pub(crate) const SUBFORMAT_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

/// WAVE_FORMAT_EXTENSIBLE tag.
pub(crate) const EXTENSIBLE_TAG: u16 = 0xfffe;
/// Mandatory EXTENSIBLE extension bytes after the WAVEFORMATEX header.
pub(crate) const EXTENSION_BYTES: usize =
    size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>();

/// One failed WASAPI COM stage: the call-site label and its raw HRESULT.
///
/// The stage text is observable in per-stack reports; every stack converts
/// this into its own error type without rewriting the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StageError {
    pub stage: &'static str,
    pub hresult: i32,
}

impl StageError {
    pub(crate) fn new(stage: &'static str, error: windows::core::Error) -> Self {
        Self {
            stage,
            hresult: error.code().0,
        }
    }
}

/// Label a COM result with its stage: the shared shape behind every stack's
/// `api()` wrapper.
pub(crate) fn stage<T>(
    label: &'static str,
    result: windows::core::Result<T>,
) -> Result<T, StageError> {
    result.map_err(|error| StageError::new(label, error))
}

/// Project a device onto its endpoint data flow. Callers map the two calls
/// through their own stage reporting; this only shares the projection.
#[cfg(feature = "probe-tools")]
pub(crate) fn endpoint_data_flow(device: &IMMDevice) -> windows::core::Result<EDataFlow> {
    let endpoint: IMMEndpoint = device.cast()?;
    unsafe { endpoint.GetDataFlow() }
}

/// A tag-dispatched parse of one owned WAVEFORMATEX allocation.
#[derive(Clone)]
pub(crate) enum ParsedDescriptor {
    /// WAVE_FORMAT_PCM: no extension; the sub-format resolves to PCM.
    Pcm { base: WAVEFORMATEX },
    /// WAVE_FORMAT_IEEE_FLOAT: no extension; the sub-format resolves to FLOAT.
    Float { base: WAVEFORMATEX },
    /// WAVE_FORMAT_EXTENSIBLE with at least the mandatory extension bytes.
    Extensible(WAVEFORMATEXTENSIBLE),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DescriptorError {
    /// Unknown format tag, or EXTENSIBLE with a truncated extension.
    UnsupportedTag,
}

impl ParsedDescriptor {
    /// The packed base header, copied by value.
    #[cfg(test)]
    pub(crate) fn base(&self) -> WAVEFORMATEX {
        match self {
            Self::Pcm { base } | Self::Float { base } => *base,
            Self::Extensible(extended) => extended.Format,
        }
    }

    /// Valid bits per sample; present for EXTENSIBLE only.
    #[cfg(test)]
    pub(crate) fn valid_bits(&self) -> Option<u16> {
        match self {
            Self::Extensible(extended) => {
                // SAFETY: reading the only Windows-initialized member of the
                // by-value extension copy; no native reference escapes.
                Some(unsafe { extended.Samples.wValidBitsPerSample })
            }
            Self::Pcm { .. } | Self::Float { .. } => None,
        }
    }

    /// Channel mask; present for EXTENSIBLE only.
    #[cfg(test)]
    pub(crate) fn channel_mask(&self) -> Option<u32> {
        match self {
            Self::Extensible(extended) => Some(extended.dwChannelMask),
            Self::Pcm { .. } | Self::Float { .. } => None,
        }
    }

    /// The resolved sub-format GUID.
    #[cfg(test)]
    pub(crate) fn sub_format(&self) -> GUID {
        match self {
            Self::Pcm { .. } => SUBFORMAT_PCM,
            Self::Float { .. } => SUBFORMAT_FLOAT,
            Self::Extensible(extended) => extended.SubFormat,
        }
    }

    /// Materialize the descriptor as WAVEFORMATEXTENSIBLE, synthesizing the
    /// extension for the bare PCM/IEEE-float tags.
    pub(crate) fn into_extensible(self) -> WAVEFORMATEXTENSIBLE {
        match self {
            Self::Extensible(extended) => extended,
            Self::Pcm { base } => synthesize_extensible(base, SUBFORMAT_PCM),
            Self::Float { base } => synthesize_extensible(base, SUBFORMAT_FLOAT),
        }
    }
}

fn synthesize_extensible(base: WAVEFORMATEX, sub_format: GUID) -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: base,
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: base.wBitsPerSample,
        },
        dwChannelMask: 0,
        SubFormat: sub_format,
    }
}

/// Parse the format-tag dispatch of an owned descriptor allocation.
///
/// # Safety
///
/// `pointer` must be valid for reads of `size_of::<WAVEFORMATEX>()` and,
/// when the tag is EXTENSIBLE with sufficient `cbSize`, of the extension
/// bytes. Both stacks only pass GetMixFormat-owned or test-owned complete
/// allocations; fields are read packed by value so no reference to an
/// unaligned native field escapes.
pub(crate) unsafe fn parse_descriptor(
    pointer: *const WAVEFORMATEX,
) -> Result<ParsedDescriptor, DescriptorError> {
    // SAFETY: caller guarantees the complete allocation (see above).
    let base = unsafe { pointer.read_unaligned() };
    match base.wFormatTag {
        1 => Ok(ParsedDescriptor::Pcm { base }),
        3 => Ok(ParsedDescriptor::Float { base }),
        EXTENSIBLE_TAG if usize::from(base.cbSize) >= EXTENSION_BYTES => {
            // SAFETY: the EXTENSIBLE branch requires the extension bytes.
            Ok(ParsedDescriptor::Extensible(unsafe {
                pointer.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned()
            }))
        }
        _ => Err(DescriptorError::UnsupportedTag),
    }
}

/// HRESULTs that abandon the IAudioClient3 path in favor of the legacy
/// shared initialization after a capability query (and, in the professional
/// stack, after the interface query as well): the endpoint does not
/// implement, or rejects the format for, the Client3 negotiation.
///
/// The legacy stack additionally treats a plain `E_NOINTERFACE` from the
/// interface query alone as the narrower `Client3Unavailable` fallback; that
/// stage strictness stays at its call site.
pub(crate) fn client3_rejects_format(code: windows::core::HRESULT) -> bool {
    matches!(
        code,
        E_NOINTERFACE | E_NOTIMPL | AUDCLNT_E_UNSUPPORTED_FORMAT
    )
}

/// Reaction to [`AUDCLNT_E_ENGINE_PERIODICITY_LOCKED`] from
/// `InitializeSharedAudioStream` (decision D4: the stacks intentionally
/// diverge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnginePeriodicityPolicy {
    /// Exact-or-fail: an explicit Shared period is a hard contract, so the
    /// HRESULT surfaces unchanged. Used by the professional stack, which
    /// also rejects out-of-range periods before Initialize instead of
    /// rounding them to a legal one.
    ExactPeriod,
}

/// GetSharedModeEnginePeriod frame bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EnginePeriods {
    pub default_frames: u32,
    pub fundamental_frames: u32,
    pub min_frames: u32,
    pub max_frames: u32,
}

/// Query the shared-mode engine period capabilities for one client format.
///
/// # Safety
///
/// `format` must be valid for reads of the complete base+cbSize allocation
/// for the duration of the call.
pub(crate) unsafe fn shared_engine_periods(
    client3: &IAudioClient3,
    format: *const WAVEFORMATEX,
) -> windows::core::Result<EnginePeriods> {
    let mut periods = EnginePeriods {
        default_frames: 0,
        fundamental_frames: 0,
        min_frames: 0,
        max_frames: 0,
    };
    // SAFETY: caller guarantees the descriptor (see above); the owner
    // retains its validated client through preparation, and this query
    // follows SetClientProperties for each stream.
    unsafe {
        client3.GetSharedModeEnginePeriod(
            format,
            &mut periods.default_frames,
            &mut periods.fundamental_frames,
            &mut periods.min_frames,
            &mut periods.max_frames,
        )?;
    }
    Ok(periods)
}

/// Explicit legacy (`IAudioClient::Initialize`) flag selection (decision D4:
/// parameterized, not silently unified).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LegacySharedInit {
    /// Add AUTOCONVERTPCM | SRC_DEFAULT_QUALITY so Windows converts between
    /// the client format and the endpoint mix format.
    pub auto_convert: bool,
    /// Add NOPERSIST. The legacy shared stack opts out of persisting session
    /// state across reboots; the professional stack keeps Windows' default
    /// persisted-session behavior.
    pub nopersist: bool,
}

/// Compose the legacy initialization flags (pure; unit-tested).
pub(crate) fn legacy_shared_flags(init: LegacySharedInit) -> u32 {
    let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    if init.nopersist {
        flags |= AUDCLNT_STREAMFLAGS_NOPERSIST;
    }
    if init.auto_convert {
        flags |= AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    }
    flags
}

/// The exclusive event-callback flag used by both the diagnostic probe and
/// the professional WASAPI owner.
pub(crate) const fn exclusive_stream_flags() -> u32 {
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK
}

/// Convert an exclusive buffer size to the WASAPI 100-ns duration unit.
/// Callers validate that the negotiated sample rate is non-zero.
pub(crate) fn exclusive_frames_duration(frames: u32, rate: u32) -> i64 {
    ((u64::from(frames) * 10_000_000 + u64::from(rate) / 2) / u64::from(rate)) as i64
}

/// Probe exclusive support while preserving non-format HRESULT failures for
/// the caller's own error/reporting policy.
pub(crate) fn exclusive_format_supported(
    client: &IAudioClient,
    format: *const WAVEFORMATEX,
) -> windows::core::Result<bool> {
    let result = unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, format, None) };
    if result == AUDCLNT_E_UNSUPPORTED_FORMAT {
        return Ok(false);
    }
    result.ok()?;
    Ok(result == S_OK)
}

/// Initialize an exclusive event-driven client. The caller keeps the format
/// descriptor alive and maps the returned HRESULT to its own error type.
///
/// # Safety
///
/// `format` must point to a valid WAVEFORMATEX descriptor for the duration of
/// the COM call.
pub(crate) unsafe fn initialize_exclusive(
    client: &IAudioClient,
    format: *const WAVEFORMATEX,
    duration: i64,
) -> windows::core::Result<()> {
    // SAFETY: upheld by the function contract.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            exclusive_stream_flags(),
            duration,
            duration,
            format,
            None,
        )
    }
}

/// Legacy shared initialization through `IAudioClient::Initialize`.
///
/// IsFormatSupported does not account for AUTOCONVERTPCM; Initialize is
/// authoritative. The stage label and error mapping stay with the caller.
///
/// # Safety
///
/// `format` must be valid for reads of the complete base+cbSize allocation
/// for the duration of the call.
pub(crate) unsafe fn initialize_legacy_shared(
    client: &IAudioClient,
    format: *const WAVEFORMATEX,
    session_guid: Option<&GUID>,
    init: LegacySharedInit,
) -> windows::core::Result<()> {
    // SAFETY: caller guarantees the descriptor (see above).
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            legacy_shared_flags(init),
            0,
            0,
            format,
            session_guid.map(std::ptr::from_ref),
        )
    }
}

/// Read `PKEY_Device_FriendlyName` from an endpoint.
pub(crate) fn friendly_name(device: &IMMDevice) -> windows::core::Result<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ)?;
        // windows 0.62 owns this returned PROPVARIANT and clears it on Drop.
        let property = store.GetValue(&PKEY_Device_FriendlyName)?;
        let value = &property.Anonymous.Anonymous;
        if value.vt != VT_LPWSTR {
            return Err(windows::core::Error::from_hresult(E_UNEXPECTED));
        }
        Ok(value.Anonymous.pwszVal.to_string()?)
    }
}

#[cfg(test)]
mod tests {
    use super::{EXTENSION_BYTES, LegacySharedInit, SUBFORMAT_FLOAT, SUBFORMAT_PCM};
    use windows::{
        Win32::{
            Foundation::{E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL},
            Media::Audio::{
                AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_ENGINE_PERIODICITY_LOCKED,
                AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_NOPERSIST,
                AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
                WAVEFORMATEXTENSIBLE_0,
            },
            System::Com::CoTaskMemAlloc,
        },
        core::GUID,
    };

    fn owned(format: WAVEFORMATEXTENSIBLE) -> crate::owner::TaskMemory<WAVEFORMATEX> {
        let memory = crate::owner::TaskMemory(
            unsafe { CoTaskMemAlloc(size_of::<WAVEFORMATEXTENSIBLE>()) }.cast::<WAVEFORMATEX>(),
        );
        assert!(!memory.0.is_null());
        // SAFETY: this test owns the complete extended allocation.
        unsafe {
            memory
                .0
                .cast::<WAVEFORMATEXTENSIBLE>()
                .write_unaligned(format)
        };
        memory
    }

    fn extensible(sub_format: GUID, cb_size: u16) -> WAVEFORMATEXTENSIBLE {
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: super::EXTENSIBLE_TAG,
                nChannels: 2,
                nSamplesPerSec: 48_000,
                nAvgBytesPerSec: 384_000,
                nBlockAlign: 8,
                wBitsPerSample: 32,
                cbSize: cb_size,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: 24,
            },
            dwChannelMask: 3,
            SubFormat: sub_format,
        }
    }

    #[test]
    fn subformat_guids_match_the_ksdataformat_subtypes() {
        assert_eq!(
            SUBFORMAT_PCM,
            GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71)
        );
        assert_eq!(
            SUBFORMAT_FLOAT,
            GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71)
        );
        assert_ne!(SUBFORMAT_PCM, SUBFORMAT_FLOAT);
    }

    #[test]
    fn parse_dispatches_bare_and_extensible_tags() {
        let memory = owned(extensible(SUBFORMAT_PCM, EXTENSION_BYTES as u16));
        // SAFETY: the allocation above is complete and test-owned.
        let parsed = unsafe { super::parse_descriptor(memory.0) }.unwrap();
        assert_eq!(parsed.sub_format(), SUBFORMAT_PCM);
        assert_eq!(parsed.valid_bits(), Some(24));
        assert_eq!(parsed.channel_mask(), Some(3));
        let base = parsed.base();
        assert_eq!({ base.nSamplesPerSec }, 48_000);

        let mut bare = extensible(SUBFORMAT_PCM, 0);
        bare.Format.wFormatTag = 1;
        let memory = owned(bare);
        // SAFETY: test-owned allocation.
        let parsed = unsafe { super::parse_descriptor(memory.0) }.unwrap();
        assert_eq!(parsed.sub_format(), SUBFORMAT_PCM);
        assert_eq!(parsed.valid_bits(), None);
        assert_eq!(parsed.channel_mask(), None);
        let extended = parsed.into_extensible();
        assert_eq!({ extended.SubFormat }, SUBFORMAT_PCM);
        assert_eq!({ extended.dwChannelMask }, 0);
        assert_eq!({ unsafe { extended.Samples.wValidBitsPerSample } }, 32);

        let mut floating = extensible(SUBFORMAT_FLOAT, 0);
        floating.Format.wFormatTag = 3;
        let memory = owned(floating);
        // SAFETY: test-owned allocation.
        let parsed = unsafe { super::parse_descriptor(memory.0) }.unwrap();
        assert_eq!(parsed.sub_format(), SUBFORMAT_FLOAT);
        let extended = parsed.into_extensible();
        assert_eq!({ extended.SubFormat }, SUBFORMAT_FLOAT);
    }

    #[test]
    fn parse_rejects_unknown_tags_and_truncated_extensions() {
        for (tag, cb_size) in [
            (0u16, 0u16),
            (super::EXTENSIBLE_TAG, 0),
            (super::EXTENSIBLE_TAG, 21),
        ] {
            let mut format = extensible(SUBFORMAT_PCM, cb_size);
            format.Format.wFormatTag = tag;
            let memory = owned(format);
            // SAFETY: test-owned allocation.
            assert!(unsafe { super::parse_descriptor(memory.0) }.is_err());
        }
    }

    #[test]
    fn client3_rejection_covers_missing_and_unsupported_queries() {
        for code in [E_NOINTERFACE, E_NOTIMPL, AUDCLNT_E_UNSUPPORTED_FORMAT] {
            assert!(super::client3_rejects_format(code));
        }
        for code in [
            AUDCLNT_E_DEVICE_INVALIDATED,
            E_INVALIDARG,
            AUDCLNT_E_ENGINE_PERIODICITY_LOCKED,
        ] {
            assert!(!super::client3_rejects_format(code));
        }
    }

    #[test]
    fn legacy_flags_compose_each_explicit_decision() {
        assert_eq!(
            super::legacy_shared_flags(LegacySharedInit {
                auto_convert: false,
                nopersist: false,
            }),
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        );
        assert_eq!(
            super::legacy_shared_flags(LegacySharedInit {
                auto_convert: false,
                nopersist: true,
            }),
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_NOPERSIST
        );
        assert_eq!(
            super::legacy_shared_flags(LegacySharedInit {
                auto_convert: true,
                nopersist: true,
            }),
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_NOPERSIST
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
        );
    }

    #[test]
    fn exclusive_helpers_keep_event_mode_and_round_frame_duration() {
        assert_eq!(
            super::exclusive_stream_flags(),
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        );
        assert_eq!(super::exclusive_frames_duration(480, 48_000), 100_000);
        assert_eq!(super::exclusive_frames_duration(1, 44_100), 227);
    }
}
