//! Shared WASAPI negotiation kernel.
//!
//! The professional and legacy stacks each negotiate shared-mode streams;
//! this module is the single source for the pieces they had duplicated:
//! PCM/floating-point sub-format GUIDs, WAVEFORMATEX descriptor parsing, the
//! IAudioClient3 rejection matrix, endpoint friendly names, the legacy
//! initialization flags, and the directional open sequence. COM call order
//! and HRESULT handling stay verbatim per stack; deliberate divergences are
//! explicit parameters instead of silent unification (decision D4 in
//! `docs/superpowers/plans/2026-10-19-repo-steward-remediation.md`).

use crate::owner::{OwnedHandle, TaskMemory};
use crate::shared_stream::format::PreparedSharedFormat;
use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::{E_NOINTERFACE, E_NOTIMPL, E_UNEXPECTED, S_OK},
        Media::Audio::{
            AUDCLNT_E_ENGINE_PERIODICITY_LOCKED, AUDCLNT_E_UNSUPPORTED_FORMAT,
            AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_NOPERSIST, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            AudioCategory_Other, AudioClientProperties, IAudioClient, IAudioClient2, IAudioClient3,
            IMMDevice, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, WAVEFORMATEX,
            WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0, eCapture, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, CoCreateGuid, CoCreateInstance, STGM_READ},
            Variant::VT_LPWSTR,
        },
    },
    core::{GUID, HSTRING, Interface},
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
    pub(crate) fn base(&self) -> WAVEFORMATEX {
        match self {
            Self::Pcm { base } | Self::Float { base } => *base,
            Self::Extensible(extended) => extended.Format,
        }
    }

    /// Valid bits per sample; present for EXTENSIBLE only.
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
    pub(crate) fn channel_mask(&self) -> Option<u32> {
        match self {
            Self::Extensible(extended) => Some(extended.dwChannelMask),
            Self::Pcm { .. } | Self::Float { .. } => None,
        }
    }

    /// The resolved sub-format GUID.
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
    /// Retry exactly once with the engine's current period when that period
    /// stays legal for the negotiated format. Used by the legacy shared
    /// stack, whose desired period is advisory.
    EnginePeriodRetry,
}

/// Whether `code` may be retried with the engine's current period under
/// `policy`; every other failure stays terminal.
pub(crate) fn periodicity_locked_retry(
    policy: EnginePeriodicityPolicy,
    code: windows::core::HRESULT,
) -> bool {
    policy == EnginePeriodicityPolicy::EnginePeriodRetry
        && code == AUDCLNT_E_ENGINE_PERIODICITY_LOCKED
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

/// Stage labels for the shared open sequence. Every label is observable in
/// per-stack error reports; each stack passes its own historic strings.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SharedOpenStages {
    pub enumerator: &'static str,
    pub device: &'static str,
    pub endpoint: &'static str,
    pub data_flow: &'static str,
    pub event: &'static str,
    pub activate: &'static str,
    pub client2: &'static str,
    pub client_properties: &'static str,
    pub mix_format: &'static str,
    pub session_guid: &'static str,
    pub set_event: &'static str,
}

impl SharedOpenStages {
    /// Historic labels for the physical capture owner. These strings are
    /// surfaced in capture reports, so the shared table centralizes the
    /// construction without changing the established diagnostics.
    pub(crate) const fn capture() -> Self {
        Self {
            enumerator: "CoCreateInstance(capture)",
            device: "GetDevice(capture pinned)",
            endpoint: "IMMEndpoint(capture)",
            data_flow: "GetDataFlow(capture)",
            event: "CreateEvent(capture audio)",
            activate: "Activate(capture)",
            client2: "IAudioClient2(capture)",
            client_properties: "SetClientProperties(capture)",
            mix_format: "GetMixFormat(capture)",
            session_guid: "CoCreateGuid(capture)",
            set_event: "SetEventHandle(capture)",
        }
    }

    /// Historic labels for the shared render owner. These strings are
    /// surfaced in render reports, so the shared table centralizes the
    /// construction without changing the established diagnostics.
    pub(crate) const fn render() -> Self {
        Self {
            enumerator: "CoCreateInstance(MMDeviceEnumerator)",
            device: "GetDevice(pinned)",
            endpoint: "IMMEndpoint",
            data_flow: "GetDataFlow",
            event: "CreateEvent(audio)",
            activate: "Activate(IAudioClient)",
            client2: "IAudioClient2",
            client_properties: "SetClientProperties",
            mix_format: "GetMixFormat",
            session_guid: "CoCreateGuid(session)",
            set_event: "SetEventHandle",
        }
    }
}

/// Failures of the shared open sequence, before stack-specific mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenSharedError {
    /// A COM stage failed.
    Stage(StageError),
    /// `GetDataFlow` reported the opposite direction for the pinned endpoint.
    WrongDirection,
}

impl From<StageError> for OpenSharedError {
    fn from(failure: StageError) -> Self {
        Self::Stage(failure)
    }
}

/// The open artifacts each stack keeps for streaming.
pub(crate) struct OpenedSharedClient {
    pub client: IAudioClient,
    pub event: OwnedHandle,
}

/// Open one pinned shared-mode client for a direction (W4): enumerate,
/// GetDevice, flow check, CreateEvent, Activate, SetClientProperties,
/// GetMixFormat, initialize, SetEventHandle.
///
/// The caller must already hold the COM apartment for the stream's whole
/// lifetime. `prepare_format` validates the GetMixFormat allocation, records
/// any per-stack report fields, and returns the client format plus an extra
/// value the caller needs after the sequence; `initialize` performs the
/// stack's shared initialization between the mix read and SetEventHandle.
/// Both callbacks receive the caller's report so their updates land at the
/// same failure points as the inlined sequences they replace.
#[allow(clippy::type_complexity)]
pub(crate) fn open_shared_client<R, X, E>(
    stages: SharedOpenStages,
    endpoint_id: &str,
    capture: bool,
    report: &mut R,
    prepare_format: impl FnOnce(
        &mut R,
        TaskMemory<WAVEFORMATEX>,
    ) -> Result<(PreparedSharedFormat, X), E>,
    initialize: impl FnOnce(&mut R, &IAudioClient, &PreparedSharedFormat, &GUID) -> Result<(), E>,
) -> Result<(OpenedSharedClient, X), E>
where
    E: From<OpenSharedError>,
{
    let enumerator: IMMDeviceEnumerator = open_stage(stages.enumerator, unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    })?;
    let device = open_stage(stages.device, unsafe {
        enumerator.GetDevice(&HSTRING::from(endpoint_id))
    })?;
    let endpoint: IMMEndpoint = open_stage(stages.endpoint, device.cast())?;
    let flow = if capture { eCapture } else { eRender };
    if open_stage(stages.data_flow, unsafe { endpoint.GetDataFlow() })? != flow {
        return Err(OpenSharedError::WrongDirection.into());
    }
    let event = open_stage(stages.event, OwnedHandle::event())?;
    let client: IAudioClient = open_stage(stages.activate, unsafe {
        device.Activate(CLSCTX_ALL, None)
    })?;
    let client2: IAudioClient2 = open_stage(stages.client2, client.cast())?;
    open_stage(stages.client_properties, unsafe {
        client2.SetClientProperties(&client_properties())
    })?;
    let memory = TaskMemory(open_stage(stages.mix_format, unsafe {
        client.GetMixFormat()
    })?);
    let (format, extra) = prepare_format(report, memory)?;
    let session_guid = open_stage(stages.session_guid, unsafe { CoCreateGuid() })?;
    initialize(report, &client, &format, &session_guid)?;
    open_stage(stages.set_event, unsafe { client.SetEventHandle(event.0) })?;
    Ok((OpenedSharedClient { client, event }, extra))
}

/// Client properties both shared stacks configure before GetMixFormat.
fn client_properties() -> AudioClientProperties {
    AudioClientProperties {
        cbSize: size_of::<AudioClientProperties>() as u32,
        eCategory: AudioCategory_Other,
        ..Default::default()
    }
}

/// Run one labeled COM stage of the open sequence, converting into the
/// caller's stack error.
fn open_stage<E, T>(label: &'static str, result: windows::core::Result<T>) -> Result<T, E>
where
    E: From<OpenSharedError>,
{
    stage(label, result)
        .map_err(OpenSharedError::from)
        .map_err(E::from)
}

#[cfg(test)]
mod tests {
    use super::{
        EXTENSION_BYTES, EnginePeriodicityPolicy, LegacySharedInit, SUBFORMAT_FLOAT, SUBFORMAT_PCM,
    };
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

    fn owned(format: WAVEFORMATEXTENSIBLE) -> super::TaskMemory<WAVEFORMATEX> {
        let memory = super::TaskMemory(
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
    fn periodicity_retry_follows_the_stack_policy() {
        for policy in [
            EnginePeriodicityPolicy::ExactPeriod,
            EnginePeriodicityPolicy::EnginePeriodRetry,
        ] {
            assert_eq!(
                super::periodicity_locked_retry(policy, AUDCLNT_E_ENGINE_PERIODICITY_LOCKED),
                policy == EnginePeriodicityPolicy::EnginePeriodRetry
            );
            assert!(!super::periodicity_locked_retry(
                policy,
                AUDCLNT_E_DEVICE_INVALIDATED
            ));
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

    #[test]
    fn shared_open_stages_keep_direction_specific_report_labels() {
        let capture = super::SharedOpenStages::capture();
        assert_eq!(capture.enumerator, "CoCreateInstance(capture)");
        assert_eq!(capture.device, "GetDevice(capture pinned)");
        assert_eq!(capture.endpoint, "IMMEndpoint(capture)");
        assert_eq!(capture.event, "CreateEvent(capture audio)");
        assert_eq!(capture.activate, "Activate(capture)");
        assert_eq!(capture.set_event, "SetEventHandle(capture)");

        let render = super::SharedOpenStages::render();
        assert_eq!(render.enumerator, "CoCreateInstance(MMDeviceEnumerator)");
        assert_eq!(render.device, "GetDevice(pinned)");
        assert_eq!(render.endpoint, "IMMEndpoint");
        assert_eq!(render.event, "CreateEvent(audio)");
        assert_eq!(render.activate, "Activate(IAudioClient)");
        assert_eq!(render.set_event, "SetEventHandle");
    }
}
