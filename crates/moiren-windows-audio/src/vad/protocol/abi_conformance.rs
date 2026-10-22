//! The kernel header `driver/vad/include/moiren_vad.h` is the wire contract's
//! canonical owner. These tests parse it and fail whenever the Rust mirror
//! drifts, instead of letting a one-sided edit compile green everywhere and
//! mismatch IOCTLs or layouts on hardware.
use super::*;
use std::collections::HashMap;

const HEADER: &str = include_str!("../../../../../driver/vad/include/moiren_vad.h");

/// Collapse line continuations so multi-line defines parse as one record.
fn normalized() -> String {
    HEADER.replace("\r\n", "\n").replace("\\\n", " ")
}

/// Plain numeric `#define` values; expression-valued defines are skipped and
/// covered by dedicated tests below.
fn defines() -> HashMap<String, u64> {
    let mut map = HashMap::new();
    for line in normalized().lines() {
        let Some(rest) = line.trim().strip_prefix("#define ") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let (Some(name), Some(value), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let digits = value.strip_suffix('u').unwrap_or(value);
        let parsed = if let Some(hex) = digits.strip_prefix("0x") {
            u64::from_str_radix(hex, 16)
        } else {
            digits.parse::<u64>()
        };
        if let Ok(number) = parsed {
            map.insert(name.to_string(), number);
        }
    }
    map
}

/// `static_assert(sizeof(NAME) == N)` records the ABI's authoritative layout
/// sizes; the header compiler enforces them for C, this map for Rust.
fn struct_sizes() -> HashMap<String, u64> {
    let mut map = HashMap::new();
    for line in HEADER.lines() {
        let Some(rest) = line.trim().strip_prefix("static_assert(sizeof(") else {
            continue;
        };
        let Some(split) = rest.find(") == ") else {
            continue;
        };
        if let Ok(size) = rest[split + 5..].trim_end_matches(");").parse::<u64>() {
            map.insert(rest[..split].to_string(), size);
        }
    }
    map
}

fn header_define(header: &HashMap<String, u64>, name: &str) -> u64 {
    *header
        .get(name)
        .unwrap_or_else(|| panic!("header define {name} missing or not a plain literal"))
}

#[test]
fn scalar_constants_match_header() {
    let header = defines();
    let cases = [
        ("MOIREN_VAD_ABI_VERSION", ABI_VERSION as u64),
        ("MOIREN_VAD_MAX_ENDPOINTS", MAX_ENDPOINTS as u64),
        ("MOIREN_VAD_MAX_STREAMS_PER_PIN", MAX_STREAMS_PER_PIN as u64),
        ("MOIREN_VAD_SAMPLE_RATE", SAMPLE_RATE as u64),
        ("MOIREN_VAD_REQUEST_SLOTS", REQUEST_SLOTS as u64),
        ("MOIREN_VAD_MAX_PACKET_FRAMES", MAX_PACKET_FRAMES as u64),
        ("MOIREN_VAD_MAX_NAME_UNITS", MAX_NAME_UNITS as u64),
        ("MOIREN_VAD_CAP_RAW_PDO", CAP_RAW_PDO as u64),
        ("MOIREN_VAD_CAP_POLICY_STAGED", CAP_POLICY_STAGED as u64),
        ("MOIREN_VAD_CAP_AUDIO", CAP_AUDIO as u64),
        ("MOIREN_VAD_CAP_STREAMS", CAP_STREAMS as u64),
        ("MOIREN_VAD_CAP_PCM", CAP_PCM as u64),
        ("MOIREN_VAD_CAP_CREATE_DEFERRED", CAP_CREATE_DEFERRED as u64),
        ("MOIREN_VAD_FORMAT_PCM16", FORMAT_PCM16 as u64),
        ("MOIREN_VAD_FORMAT_FLOAT32", FORMAT_FLOAT32 as u64),
        ("MOIREN_VAD_INTENT_PLAYBACK", Intent::Playback as u64),
        ("MOIREN_VAD_INTENT_MICROPHONE", Intent::Microphone as u64),
        ("MOIREN_VAD_STATE_PRESENT", PdoState::Present as u64),
        ("MOIREN_VAD_STATE_REMOVING", PdoState::Removing as u64),
        ("MOIREN_VAD_STATE_REMOVED", PdoState::Removed as u64),
        ("MOIREN_VAD_STATE_CREATING", PdoState::Creating as u64),
        ("MOIREN_VAD_STREAM_CLOSED", StreamState::Closed as u64),
        ("MOIREN_VAD_STREAM_PAUSED", StreamState::Paused as u64),
        ("MOIREN_VAD_STREAM_RUNNING", StreamState::Running as u64),
    ];
    for (name, value) in cases {
        assert_eq!(
            header_define(&header, name),
            value,
            "{name} disagrees with protocol.rs"
        );
    }
}

#[test]
fn ioctl_codes_match_header() {
    let header = defines();
    let cases = [
        ("IOCTL_MOIREN_VAD_CAPABILITIES", IOCTL_CAPABILITIES),
        ("IOCTL_MOIREN_VAD_CREATE", IOCTL_CREATE),
        ("IOCTL_MOIREN_VAD_QUERY", IOCTL_QUERY),
        ("IOCTL_MOIREN_VAD_REMOVE", IOCTL_REMOVE),
        ("IOCTL_MOIREN_VAD_SET_GATE", IOCTL_SET_GATE),
        ("IOCTL_MOIREN_VAD_READ_PCM", IOCTL_READ_PCM),
        ("IOCTL_MOIREN_VAD_WRITE_PCM", IOCTL_WRITE_PCM),
    ];
    for (name, value) in cases {
        assert_eq!(
            header_define(&header, name),
            value as u64,
            "{name} disagrees with protocol.rs"
        );
    }
}

#[test]
fn wire_struct_sizes_match_header() {
    let header = struct_sizes();
    let cases = [
        ("MOIREN_VAD_CAPABILITIES", CAPABILITIES_SIZE),
        ("MOIREN_VAD_ENDPOINT_SNAPSHOT", SNAPSHOT_SIZE),
        ("MOIREN_VAD_CREATE_REQUEST", CREATE_SIZE),
        ("MOIREN_VAD_ENDPOINT_REQUEST", ENDPOINT_REQUEST_SIZE),
        ("MOIREN_VAD_GATE_REQUEST", GATE_REQUEST_SIZE),
        ("MOIREN_VAD_PCM_HEADER", PCM_HEADER_SIZE),
    ];
    for (name, value) in cases {
        let size = *header
            .get(name)
            .unwrap_or_else(|| panic!("header static_assert for {name} missing"));
        assert_eq!(size, value as u64, "{name} disagrees with protocol.rs");
    }
}

#[test]
fn max_pcm_bytes_derive_matches_header() {
    let header = defines();
    let frames = header_define(&header, "MOIREN_VAD_MAX_PACKET_FRAMES");
    assert_eq!(MAX_PCM_BYTES as u64, frames * 2 * 4);
}

#[test]
fn control_interface_guid_matches_header() {
    let text = normalized();
    let line = text
        .lines()
        .find(|line| line.contains("MOIREN_VAD_CONTROL_INTERFACE_GUID"))
        .expect("control interface GUID define present in header");
    let body = line[line.find('{').expect("GUID opening brace")..].replace("0x", "");
    let words: Vec<u128> = body
        .split(|c: char| !c.is_ascii_hexdigit())
        .filter(|token| !token.is_empty())
        .map(|token| u128::from_str_radix(token, 16).expect("hex word"))
        .collect();
    assert_eq!(words.len(), 11, "GUID data words: {words:?}");
    // Data1 (32) + Data2 (16) + Data3 (16) + Data4 (8x8) fill the u128 in
    // header word order; GUID::from_u128 reads the same layout.
    let mut value = (words[0] << 16) | words[1];
    value = (value << 16) | words[2];
    for word in &words[3..] {
        value = (value << 8) | word;
    }
    assert_eq!(value, CONTROL_INTERFACE_GUID_U128);
}
