//! Explicit little-endian ABI v2. No native struct layout crosses the wire.

use moiren_engine::node::VirtualEndpointId;
use std::{fmt, num::NonZeroU64};

#[cfg(any(windows, test))]
mod create_reply;
#[cfg(windows)]
pub(crate) use create_reply::{CreateReply, decode_create_reply};
#[cfg(test)]
mod abi_conformance;

pub const ABI_VERSION: u32 = 2;
pub const MAX_ENDPOINTS: u32 = 64;
pub const MAX_STREAMS_PER_PIN: u32 = 1;
pub const SAMPLE_RATE: u32 = 48_000;
pub const REQUEST_SLOTS: usize = 4;
pub const MAX_PACKET_FRAMES: u32 = 4_800;
pub const MAX_NAME_UNITS: usize = 64;
pub const MAX_PCM_BYTES: usize = MAX_PACKET_FRAMES as usize * 2 * 4;
pub const CAPABILITIES_SIZE: usize = 64;
pub const SNAPSHOT_SIZE: usize = 128;
pub const CREATE_SIZE: usize = 168;
pub const ENDPOINT_REQUEST_SIZE: usize = 32;
pub const GATE_REQUEST_SIZE: usize = 48;
pub const PCM_HEADER_SIZE: usize = 80;
pub const CAP_RAW_PDO: u32 = 1;
pub const CAP_POLICY_STAGED: u32 = 2;
pub const CAP_AUDIO: u32 = 4;
pub const CAP_STREAMS: u32 = 8;
pub const CAP_PCM: u32 = 16;
pub const CAP_CREATE_DEFERRED: u32 = 32;
pub const FORMAT_PCM16: u32 = 1;
pub const FORMAT_FLOAT32: u32 = 2;
const AUDIO_FLAGS: u32 = CAP_RAW_PDO | CAP_POLICY_STAGED | CAP_AUDIO | CAP_STREAMS;
const KNOWN_FLAGS: u32 = AUDIO_FLAGS | CAP_PCM | CAP_CREATE_DEFERRED;
const FORMATS: u32 = FORMAT_PCM16 | FORMAT_FLOAT32;
const CHANNEL_MASK: u32 = 6;
const INTENT_MASK: u32 = 6;

pub const IOCTL_CAPABILITIES: u32 = 0x0022_e000;
pub const IOCTL_CREATE: u32 = 0x0022_e004;
pub const IOCTL_QUERY: u32 = 0x0022_e008;
pub const IOCTL_REMOVE: u32 = 0x0022_e00c;
pub const IOCTL_SET_GATE: u32 = 0x0022_e010;
pub const IOCTL_READ_PCM: u32 = 0x0022_e016;
pub const IOCTL_WRITE_PCM: u32 = 0x0022_e019;

/// Control-interface identity in the kernel header's canonical word order
/// (Data1, Data2, Data3, then Data4 big-endian). Discovery builds the Windows
/// `GUID` from this single value; `abi_conformance` pins it to the header.
#[cfg(any(windows, test))]
pub const CONTROL_INTERFACE_GUID_U128: u128 = 0x4c7ef487_61a1_4a21_8d63_9d45115a7a5b;

/// Persistent identity bytes, independent of runtime tokens and MMDevice IDs.
/// Bytes 0..8 are the ABI Low word in LE; bytes 8..16 are High in LE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StableEndpointId([u8; 16]);

impl StableEndpointId {
    pub const fn new(low: u64, high: u64) -> Self {
        let low = low.to_le_bytes();
        let high = high.to_le_bytes();
        Self([
            low[0], low[1], low[2], low[3], low[4], low[5], low[6], low[7], high[0], high[1],
            high[2], high[3], high[4], high[5], high[6], high[7],
        ])
    }

    pub const fn from_le_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Maps the engine's stable bytes directly to the ABI's Low/High LE words.
    pub const fn from_virtual_endpoint_id(id: VirtualEndpointId) -> Self {
        Self(id.to_bytes())
    }

    /// Returns the same persistent identity, without involving runtime tokens
    /// or the Windows device-instance string representation.
    pub const fn to_virtual_endpoint_id(self) -> VirtualEndpointId {
        VirtualEndpointId::from_bytes(self.0)
    }

    pub const fn to_le_bytes(self) -> [u8; 16] {
        self.0
    }
    pub fn low(self) -> u64 {
        read_u64(&self.0, 0)
    }
    pub fn high(self) -> u64 {
        read_u64(&self.0, 8)
    }
}

impl fmt::Display for StableEndpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Device-instance text uses the kernel's High-then-Low convention;
        // the stored bytes themselves remain the Engine's opaque byte order.
        write!(f, "{:016x}{:016x}", self.high(), self.low())?;
        Ok(())
    }
}

/// Opaque runtime PDO token. Never persist or use outside its owning session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimePdoToken(NonZeroU64);

impl RuntimePdoToken {
    pub fn new(value: u64) -> Result<Self, ProtocolError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(invalid("PDO token", value))
    }
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for RuntimePdoToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#018x}", self.get())
    }
}

/// All endpoint request identity fields travel together, including generations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EndpointKey {
    token: RuntimePdoToken,
    session_epoch: u64,
    endpoint_generation: u64,
}

impl EndpointKey {
    pub fn new(
        token: RuntimePdoToken,
        session_epoch: u64,
        endpoint_generation: u64,
    ) -> Result<Self, ProtocolError> {
        nonzero("session epoch", session_epoch)?;
        nonzero("endpoint generation", endpoint_generation)?;
        Ok(Self {
            token,
            session_epoch,
            endpoint_generation,
        })
    }
    pub const fn pdo_token(self) -> RuntimePdoToken {
        self.token
    }
    pub const fn session_epoch(self) -> u64 {
        self.session_epoch
    }
    pub const fn endpoint_generation(self) -> u64 {
        self.endpoint_generation
    }
}

macro_rules! wire_enum {
    ($name:ident, $field:literal, {$($variant:ident = $value:literal),+ $(,)?}) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u32)]
        pub enum $name { $($variant = $value),+ }
        impl TryFrom<u32> for $name {
            type Error = ProtocolError;
            fn try_from(value: u32) -> Result<Self, Self::Error> {
                match value { $($value => Ok(Self::$variant),)+
                    _ => Err(invalid($field, value.into())) }
            }
        }
    };
}
wire_enum!(Intent, "intent", {Playback = 1, Microphone = 2});
wire_enum!(Channels, "channels", {Mono = 1, Stereo = 2});
wire_enum!(PdoState, "state", {Present = 1, Removing = 2, Removed = 3, Creating = 4});
wire_enum!(StreamState, "stream state", {Closed = 0, Paused = 1, Running = 2});
wire_enum!(SampleFormat, "sample format", {Pcm16 = 1, Float32 = 2});

impl SampleFormat {
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::Pcm16 => 2,
            Self::Float32 => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    BufferSize { expected: usize, actual: usize },
    HeaderSize(u32),
    Version(u32),
    InvalidField { field: &'static str, value: u64 },
    Inconsistent(&'static str),
    AudioUnavailable,
    PcmUnavailable,
    NotReady,
    InvalidName,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferSize { expected, actual } => {
                write!(f, "expected exactly {expected} bytes, received {actual}")
            }
            Self::HeaderSize(value) => write!(f, "invalid response Size {value}"),
            Self::Version(value) => write!(f, "unsupported VAD ABI version {value}"),
            Self::InvalidField { field, value } => write!(f, "invalid {field}: {value:#x}"),
            Self::Inconsistent(reason) => write!(f, "inconsistent VAD response: {reason}"),
            Self::AudioUnavailable => f.write_str("VAD audio capabilities are incomplete"),
            Self::PcmUnavailable => f.write_str("VAD PCM transport capability is unavailable"),
            Self::NotReady => {
                f.write_str("VAD endpoint has no ready, running native packet stream")
            }
            Self::InvalidName => {
                f.write_str("VAD name must contain 1..=63 UTF-16 units and no NUL")
            }
        }
    }
}
impl std::error::Error for ProtocolError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    flags: u32,
    max_endpoints: u32,
    sample_formats: u32,
    channel_mask: u32,
    intent_mask: u32,
    session_epoch: u64,
}

impl Capabilities {
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        validate_header(bytes, CAPABILITIES_SIZE)?;
        zero_reserved(bytes, &[52, 56, 60])?;
        let value = Self {
            flags: read_u32(bytes, 8),
            max_endpoints: read_u32(bytes, 12),
            sample_formats: read_u32(bytes, 24),
            channel_mask: read_u32(bytes, 28),
            intent_mask: read_u32(bytes, 32),
            session_epoch: read_u64(bytes, 40),
        };
        if value.flags & !KNOWN_FLAGS != 0 || value.flags & CAP_RAW_PDO == 0 {
            return Err(invalid("capability flags", value.flags.into()));
        }
        if !(1..=MAX_ENDPOINTS).contains(&value.max_endpoints) {
            return Err(invalid("max endpoints", value.max_endpoints.into()));
        }
        for (offset, expected, name) in [
            (16, MAX_STREAMS_PER_PIN, "max streams per pin"),
            (20, SAMPLE_RATE, "sample rate"),
            (36, REQUEST_SLOTS as u32, "request slots"),
            (48, MAX_PACKET_FRAMES, "max packet frames"),
        ] {
            if read_u32(bytes, offset) != expected {
                return Err(invalid(name, read_u32(bytes, offset).into()));
            }
        }
        nonzero("session epoch", value.session_epoch)?;
        if value.sample_formats & !FORMATS != 0
            || value.channel_mask == 0
            || value.channel_mask & !CHANNEL_MASK != 0
            || value.intent_mask == 0
            || value.intent_mask & !INTENT_MASK != 0
        {
            return Err(ProtocolError::Inconsistent(
                "unknown formats or intent/channel masks",
            ));
        }
        let audio = value.flags & CAP_AUDIO != 0;
        if audio != (value.flags & CAP_STREAMS != 0) {
            return Err(ProtocolError::Inconsistent(
                "audio and streams must be advertised together",
            ));
        }
        if audio {
            value.require_audio()?;
        } else if value.sample_formats != 0 {
            return Err(ProtocolError::Inconsistent(
                "formats advertised without audio",
            ));
        }
        if value.flags & CAP_PCM != 0 {
            value.require_pcm()?;
        }
        Ok(value)
    }

    pub fn require_audio(self) -> Result<(), ProtocolError> {
        if self.flags & AUDIO_FLAGS != AUDIO_FLAGS
            || self.sample_formats != FORMATS
            || self.channel_mask != CHANNEL_MASK
            || self.intent_mask != INTENT_MASK
        {
            return Err(ProtocolError::AudioUnavailable);
        }
        Ok(())
    }
    pub fn require_pcm(self) -> Result<(), ProtocolError> {
        self.require_audio()?;
        if self.flags & CAP_PCM == 0 {
            return Err(ProtocolError::PcmUnavailable);
        }
        Ok(())
    }
    pub fn validate_create(self, intent: Intent, channels: Channels) -> Result<(), ProtocolError> {
        if self.intent_mask & (1 << intent as u32) == 0
            || self.channel_mask & (1 << channels as u32) == 0
        {
            return Err(ProtocolError::Inconsistent(
                "unsupported intent or channels",
            ));
        }
        Ok(())
    }
    pub const fn flags(self) -> u32 {
        self.flags
    }
    pub const fn max_endpoints(self) -> u32 {
        self.max_endpoints
    }
    pub const fn max_streams_per_pin(self) -> u32 {
        MAX_STREAMS_PER_PIN
    }
    pub const fn target_sample_rate(self) -> u32 {
        SAMPLE_RATE
    }
    pub const fn sample_formats(self) -> u32 {
        self.sample_formats
    }
    pub const fn channel_mask(self) -> u32 {
        self.channel_mask
    }
    pub const fn intent_mask(self) -> u32 {
        self.intent_mask
    }
    pub const fn session_epoch(self) -> u64 {
        self.session_epoch
    }
    pub const fn request_slots(self) -> usize {
        REQUEST_SLOTS
    }
    pub const fn max_packet_frames(self) -> u32 {
        MAX_PACKET_FRAMES
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointSnapshot {
    key: EndpointKey,
    stable_id: StableEndpointId,
    state: PdoState,
    intent: Intent,
    channels: Channels,
    gate_open: bool,
    policy_prepared: bool,
    audio_ready: bool,
    stream_generation: u64,
    gate_epoch: u64,
    sample_format: Option<SampleFormat>,
    packet_frames: u32,
    last_status: u32,
    stream_state: StreamState,
    dropped_packets: u64,
    silence_packets: u64,
}

impl EndpointSnapshot {
    pub fn decode(bytes: &[u8], capabilities: Capabilities) -> Result<Self, ProtocolError> {
        validate_header(bytes, SNAPSHOT_SIZE)?;
        zero_reserved(bytes, &[120, 124])?;
        let sample_format = match read_u32(bytes, 88) {
            0 => None,
            value => Some(SampleFormat::try_from(value)?),
        };
        let value = Self {
            key: EndpointKey::new(
                RuntimePdoToken::new(read_u64(bytes, 8))?,
                read_u64(bytes, 40),
                read_u64(bytes, 48),
            )?,
            stable_id: StableEndpointId::from_le_bytes(bytes[72..88].try_into().unwrap()),
            state: PdoState::try_from(read_u32(bytes, 16))?,
            intent: Intent::try_from(read_u32(bytes, 20))?,
            channels: Channels::try_from(read_u32(bytes, 24))?,
            gate_open: read_bool(bytes, 28, "gate open")?,
            policy_prepared: read_bool(bytes, 32, "policy prepared")?,
            audio_ready: read_bool(bytes, 36, "audio ready")?,
            stream_generation: read_u64(bytes, 56),
            gate_epoch: read_u64(bytes, 64),
            sample_format,
            packet_frames: read_u32(bytes, 92),
            last_status: read_u32(bytes, 96),
            stream_state: StreamState::try_from(read_u32(bytes, 100))?,
            dropped_packets: read_u64(bytes, 104),
            silence_packets: read_u64(bytes, 112),
        };
        if value.key.session_epoch != capabilities.session_epoch {
            return Err(ProtocolError::Inconsistent(
                "snapshot session epoch changed",
            ));
        }
        // The driver uses the runtime PDO id for both request identity fields;
        // rejecting a split identity prevents a malformed snapshot from being
        // turned into a request that the kernel will never accept.
        if value.key.pdo_token().get() != value.key.endpoint_generation() {
            return Err(ProtocolError::Inconsistent(
                "endpoint token and generation differ",
            ));
        }
        capabilities.validate_create(value.intent, value.channels)?;
        if value.policy_prepared && capabilities.flags & CAP_POLICY_STAGED == 0 {
            return Err(ProtocolError::Inconsistent(
                "prepared policy without capability",
            ));
        }
        if value.audio_ready {
            capabilities.require_audio()?;
            if !value.policy_prepared || value.state != PdoState::Present {
                return Err(ProtocolError::Inconsistent(
                    "ready audio requires present prepared policy",
                ));
            }
        }
        if value.packet_frames > MAX_PACKET_FRAMES
            || (value.packet_frames != 0 && value.sample_format.is_none())
            || (value.stream_state != StreamState::Closed && value.stream_generation == 0)
        {
            return Err(ProtocolError::Inconsistent("invalid native stream shape"));
        }
        if value.stream_generation != 0 || value.sample_format.is_some() {
            capabilities.require_audio()?;
        }
        if value.gate_open {
            capabilities.require_pcm()?;
            if !value.audio_ready || value.gate_epoch == 0 {
                return Err(ProtocolError::Inconsistent(
                    "open gate requires ready audio and epoch",
                ));
            }
        }
        Ok(value)
    }

    pub fn require_key(self, key: EndpointKey) -> Result<Self, ProtocolError> {
        if self.key != key {
            return Err(ProtocolError::Inconsistent(
                "response endpoint/session generation changed",
            ));
        }
        Ok(self)
    }
    pub fn require_created(
        self,
        id: StableEndpointId,
        intent: Intent,
        channels: Channels,
    ) -> Result<Self, ProtocolError> {
        if self.stable_id != id
            || self.intent != intent
            || self.channels != channels
            || !matches!(self.state, PdoState::Creating | PdoState::Present)
            || self.gate_open
        {
            return Err(ProtocolError::Inconsistent(
                "CREATE identity, shape, state or gate mismatch",
            ));
        }
        Ok(self)
    }
    pub fn require_progress_after(self, previous: Self) -> Result<Self, ProtocolError> {
        self.require_key(previous.key)?;
        if self.stable_id != previous.stable_id
            || self.intent != previous.intent
            || self.channels != previous.channels
            || self.stream_generation < previous.stream_generation
            || self.gate_epoch < previous.gate_epoch
            || !matches!(
                (previous.state, self.state),
                (PdoState::Creating, _)
                    | (
                        PdoState::Present,
                        PdoState::Present | PdoState::Removing | PdoState::Removed
                    )
                    | (PdoState::Removing, PdoState::Removing | PdoState::Removed)
                    | (PdoState::Removed, PdoState::Removed)
            )
        {
            return Err(ProtocolError::Inconsistent(
                "endpoint identity or lifecycle regressed",
            ));
        }
        Ok(self)
    }
    pub fn removal_complete_after(self, previous: Self) -> Result<bool, ProtocolError> {
        self.require_progress_after(previous)?;
        if !matches!(self.state, PdoState::Removing | PdoState::Removed) {
            return Err(ProtocolError::Inconsistent("removal was not accepted"));
        }
        Ok(self.state == PdoState::Removed)
    }
    pub fn pcm_shape(self, capabilities: Capabilities) -> Result<PcmShape, ProtocolError> {
        capabilities.require_pcm()?;
        if self.key.session_epoch != capabilities.session_epoch
            || self.state != PdoState::Present
            || !self.audio_ready
            || !self.policy_prepared
            || !self.gate_open
            || !matches!(
                self.stream_state,
                StreamState::Paused | StreamState::Running
            )
            || self.stream_generation == 0
            || self.gate_epoch == 0
            || self.packet_frames == 0
        {
            return Err(ProtocolError::NotReady);
        }
        Ok(PcmShape {
            snapshot: self,
            format: self.sample_format.ok_or(ProtocolError::NotReady)?,
        })
    }
    pub const fn key(self) -> EndpointKey {
        self.key
    }
    pub const fn pdo_token(self) -> RuntimePdoToken {
        self.key.token
    }
    pub const fn stable_id(self) -> StableEndpointId {
        self.stable_id
    }
    pub const fn state(self) -> PdoState {
        self.state
    }
    pub const fn intent(self) -> Intent {
        self.intent
    }
    pub const fn channels(self) -> Channels {
        self.channels
    }
    pub const fn gate_open(self) -> bool {
        self.gate_open
    }
    pub const fn policy_prepared(self) -> bool {
        self.policy_prepared
    }
    pub const fn audio_ready(self) -> bool {
        self.audio_ready
    }
    pub const fn session_epoch(self) -> u64 {
        self.key.session_epoch
    }
    pub const fn endpoint_generation(self) -> u64 {
        self.key.endpoint_generation
    }
    pub const fn stream_generation(self) -> u64 {
        self.stream_generation
    }
    pub const fn gate_epoch(self) -> u64 {
        self.gate_epoch
    }
    pub const fn sample_format(self) -> Option<SampleFormat> {
        self.sample_format
    }
    pub const fn packet_frames(self) -> u32 {
        self.packet_frames
    }
    pub const fn last_status(self) -> u32 {
        self.last_status
    }
    pub const fn stream_state(self) -> StreamState {
        self.stream_state
    }
    pub const fn dropped_packets(self) -> u64 {
        self.dropped_packets
    }
    pub const fn silence_packets(self) -> u64 {
        self.silence_packets
    }
}

/// One exact native packet shape plus endpoint, stream and gate epochs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmShape {
    snapshot: EndpointSnapshot,
    format: SampleFormat,
}

impl PcmShape {
    pub const fn snapshot(self) -> EndpointSnapshot {
        self.snapshot
    }
    pub const fn format(self) -> SampleFormat {
        self.format
    }
    pub const fn frames(self) -> u32 {
        self.snapshot.packet_frames
    }
    pub const fn channels(self) -> Channels {
        self.snapshot.channels
    }
    pub const fn pcm_bytes(self) -> usize {
        self.frames() as usize * self.channels() as usize * self.format.bytes_per_sample()
    }
    pub const fn read_bytes(self) -> usize {
        PCM_HEADER_SIZE + self.pcm_bytes()
    }
    pub fn header(self, sequence: u64, qpc: u64) -> [u8; PCM_HEADER_SIZE] {
        let mut bytes = request_header();
        write_key(&mut bytes, self.snapshot.key);
        write_u64(&mut bytes, 32, self.snapshot.stream_generation);
        write_u64(&mut bytes, 40, self.snapshot.gate_epoch);
        write_u64(&mut bytes, 48, sequence);
        write_u64(&mut bytes, 56, qpc);
        write_u32(&mut bytes, 64, self.format as u32);
        write_u32(&mut bytes, 68, self.frames());
        write_u32(&mut bytes, 72, self.channels() as u32);
        bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmPacketStamp {
    pub sequence: u64,
    pub qpc: u64,
}

impl PcmPacketStamp {
    pub fn decode_read(bytes: &[u8], shape: PcmShape) -> Result<Self, ProtocolError> {
        validate_header(bytes, PCM_HEADER_SIZE)?;
        let expected = shape.header(0, 0);
        // Only Sequence and Qpc may be stamped by the driver.
        if bytes[..48] != expected[..48] || bytes[64..] != expected[64..] {
            return Err(ProtocolError::Inconsistent(
                "PCM response generations or shape changed",
            ));
        }
        let value = Self {
            sequence: read_u64(bytes, 48),
            qpc: read_u64(bytes, 56),
        };
        nonzero("PCM QPC", value.qpc)?;
        Ok(value)
    }
}

pub fn capabilities_request() -> [u8; 8] {
    request_header()
}

pub fn create_request(
    id: StableEndpointId,
    intent: Intent,
    channels: Channels,
    name: &str,
    allow_default: bool,
) -> Result<[u8; CREATE_SIZE], ProtocolError> {
    let mut bytes = request_header();
    write_u32(&mut bytes, 8, intent as u32);
    write_u32(&mut bytes, 12, channels as u32);
    bytes[16..32].copy_from_slice(&id.to_le_bytes());
    write_u32(&mut bytes, 32, u32::from(allow_default));
    let mut count = 0;
    for unit in name.encode_utf16() {
        if unit < 0x20 || unit == 0 || count >= MAX_NAME_UNITS - 1 {
            return Err(ProtocolError::InvalidName);
        }
        bytes[40 + count * 2..42 + count * 2].copy_from_slice(&unit.to_le_bytes());
        count += 1;
    }
    if count == 0 {
        return Err(ProtocolError::InvalidName);
    }
    Ok(bytes)
}
pub fn endpoint_request(key: EndpointKey) -> [u8; ENDPOINT_REQUEST_SIZE] {
    let mut bytes = request_header();
    write_key(&mut bytes, key);
    bytes
}
pub fn gate_request(snapshot: EndpointSnapshot, open: bool) -> [u8; GATE_REQUEST_SIZE] {
    let mut bytes = request_header();
    write_key(&mut bytes, snapshot.key);
    write_u32(&mut bytes, 32, u32::from(open));
    write_u64(&mut bytes, 40, snapshot.gate_epoch);
    bytes
}
fn write_key(bytes: &mut [u8], key: EndpointKey) {
    write_u64(bytes, 8, key.token.get());
    write_u64(bytes, 16, key.session_epoch);
    write_u64(bytes, 24, key.endpoint_generation);
}
fn request_header<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    write_u32(&mut bytes, 0, N as u32);
    write_u32(&mut bytes, 4, ABI_VERSION);
    bytes
}
fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn validate_header(bytes: &[u8], expected: usize) -> Result<(), ProtocolError> {
    if bytes.len() != expected {
        return Err(ProtocolError::BufferSize {
            expected,
            actual: bytes.len(),
        });
    }
    if read_u32(bytes, 0) != expected as u32 {
        return Err(ProtocolError::HeaderSize(read_u32(bytes, 0)));
    }
    if read_u32(bytes, 4) != ABI_VERSION {
        return Err(ProtocolError::Version(read_u32(bytes, 4)));
    }
    Ok(())
}
fn zero_reserved(bytes: &[u8], offsets: &[usize]) -> Result<(), ProtocolError> {
    for &offset in offsets {
        if read_u32(bytes, offset) != 0 {
            return Err(invalid("reserved", read_u32(bytes, offset).into()));
        }
    }
    Ok(())
}
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated length"),
    )
}
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated length"),
    )
}
fn read_bool(bytes: &[u8], offset: usize, name: &'static str) -> Result<bool, ProtocolError> {
    match read_u32(bytes, offset) {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(invalid(name, value.into())),
    }
}
fn nonzero(name: &'static str, value: u64) -> Result<(), ProtocolError> {
    if value == 0 {
        return Err(invalid(name, value));
    }
    Ok(())
}
const fn invalid(field: &'static str, value: u64) -> ProtocolError {
    ProtocolError::InvalidField { field, value }
}
