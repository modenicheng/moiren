//! Control-side ABI v2 lifecycle and bounded PCM IO for the Moiren VAD driver.
//!
//! A raw PDO is not a WASAPI audio endpoint. This module does not provide an
//! engine backend, installation, or default-device policy. Discovery, lifecycle,
//! PCM IO, polling and cancellation drains belong on control/IO owner threads.
//! The PCM helper never performs IO on Engine render.

#[cfg(any(windows, test))]
mod endpoint_name;
mod format;
mod pcm_bridge;
mod pcm_slots;
mod protocol;

#[cfg(any(windows, test))]
mod interface_path;

#[cfg(windows)]
mod client;
#[cfg(windows)]
mod control_io;
#[cfg(windows)]
mod discovery;
#[cfg(windows)]
mod pcm;
#[cfg(windows)]
mod provider;

#[cfg(windows)]
pub use client::{ClientError, ControlClient};
#[cfg(windows)]
pub use discovery::{CONTROL_INTERFACE_GUID, DeviceInterface, discover_present_interfaces};
#[cfg(windows)]
pub use endpoint_name::{VadEndpointNameObservation, VadEndpointNameTarget};
pub use format::{PcmFormatError, decode_interleaved, encode_interleaved};
#[cfg(windows)]
pub use pcm::{PcmCompletion, PcmOwner, ResidualPcm};
pub use pcm_bridge::{PcmBoundaryError, PcmMicrophoneBridge, PcmPlaybackBridge};
pub use pcm_slots::PcmTicket;
#[cfg(windows)]
pub use provider::WindowsVadProvider;

pub use protocol::{
    ABI_VERSION, CAP_AUDIO, CAP_CREATE_DEFERRED, CAP_PCM, CAP_POLICY_STAGED, CAP_RAW_PDO,
    CAP_STREAMS, CAPABILITIES_SIZE, CREATE_SIZE, Capabilities, Channels, ENDPOINT_REQUEST_SIZE,
    EndpointKey, EndpointSnapshot, FORMAT_FLOAT32, FORMAT_PCM16, GATE_REQUEST_SIZE,
    IOCTL_CAPABILITIES, IOCTL_CREATE, IOCTL_QUERY, IOCTL_READ_PCM, IOCTL_REMOVE, IOCTL_SET_GATE,
    IOCTL_WRITE_PCM, Intent, MAX_ENDPOINTS, MAX_NAME_UNITS, MAX_PACKET_FRAMES, MAX_PCM_BYTES,
    MAX_STREAMS_PER_PIN, PCM_HEADER_SIZE, PcmPacketStamp, PcmShape, PdoState, ProtocolError,
    REQUEST_SLOTS, RuntimePdoToken, SAMPLE_RATE, SNAPSHOT_SIZE, SampleFormat, StableEndpointId,
    StreamState, capabilities_request, create_request, endpoint_request, gate_request,
};

#[cfg(test)]
mod tests;
