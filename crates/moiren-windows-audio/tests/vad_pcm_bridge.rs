//! Zero-allocation gates for the VAD user-mode PCM bridge and the process
//! loopback capture consumption path (audit 2026-11-20, observation 7).
//!
//! No driver or device is involved: the bridges are fed synthetic packets and
//! the loopback capture path is exercised through the same `CaptureIngress` /
//! `ClockSource` pair that `process_loopback` streams drive on Windows. The
//! WASAPI COM calls around them are outside portable scope; every allocation
//! the Engine-facing side performs is covered here.
use moiren_windows_audio::{
    clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge},
    vad::{Channels, PcmMicrophoneBridge, PcmPlaybackBridge, SampleFormat},
};
mod common;

/// One stereo f32 frame pair for a Pcm16 mono packet (2 frames, 4 bytes).
const MONO_PACKET: [u8; 4] = [0x00, 0x40, 0x00, 0xc0];
/// One stereo Pcm16 packet of 2 frames: 2 frames x 2 channels x 2 bytes.
const STEREO_PACKET: [u8; 8] = [0x10, 0x20, 0xf0, 0xe0, 0x30, 0x40, 0xd0, 0xc0];

#[test]
fn playback_decode_and_engine_carrier_publish_allocate_nothing() {
    let (engine_writer, mut carrier_reader) =
        moiren_engine::boundary::audio_bridge(2, 8, 4096).unwrap();
    let mut bridge =
        PcmPlaybackBridge::new(Channels::Mono, SampleFormat::Pcm16, 2, 4096, engine_writer)
            .unwrap();
    let packet = MONO_PACKET;
    let mut engine_frame = [0.0f32; 4];
    // Repeated decode/publish pairs cross the SPSC ring write cursor several
    // times; the carrier consumer drains on the Engine render side each pass.
    common::assert_allocates_nothing(|| {
        for _ in 0..16 {
            assert!(bridge.decode_packet(&packet).unwrap());
            assert_eq!(
                carrier_reader
                    .read_interleaved(&mut engine_frame)
                    .unwrap()
                    .transferred_frames,
                2
            );
        }
    });
}

#[test]
fn microphone_encode_and_carrier_drain_allocate_nothing() {
    let (mut engine_writer, carrier_reader) =
        moiren_engine::boundary::audio_bridge(2, 8, 4096).unwrap();
    let mut bridge = PcmMicrophoneBridge::new(
        Channels::Stereo,
        SampleFormat::Pcm16,
        2,
        4096,
        carrier_reader,
    )
    .unwrap();
    let graph_frame = [0.25f32, -0.5, 0.5, -0.25];
    // The graph producer writes the prepared carrier; the worker-side encode
    // drains exactly one packet per pass across ring wraparound.
    common::assert_allocates_nothing(|| {
        for _ in 0..16 {
            engine_writer.write_interleaved(&graph_frame).unwrap();
            let packet = bridge.encode_packet().unwrap();
            assert_eq!(packet.len(), STEREO_PACKET.len());
        }
    });
}

#[test]
fn loopback_playback_to_engine_consumption_chain_allocates_nothing() {
    // Both halves of the VAD playback handoff in one guarded pass: the worker
    // decodes a native packet into the carrier and the Engine render side
    // consumes it, exactly the RT pair that a loopback stream alternates.
    let (engine_writer, mut carrier_reader) =
        moiren_engine::boundary::audio_bridge(2, 8, 4096).unwrap();
    let mut bridge = PcmPlaybackBridge::new(
        Channels::Stereo,
        SampleFormat::Pcm16,
        2,
        4096,
        engine_writer,
    )
    .unwrap();
    let packet = STEREO_PACKET;
    let mut engine_frame = [0.0f32; 4];
    common::assert_allocates_nothing(|| {
        for _ in 0..8 {
            assert!(bridge.decode_packet(&packet).unwrap());
            carrier_reader.read_interleaved(&mut engine_frame).unwrap();
            assert!(bridge.decode_packet(&packet).unwrap());
            carrier_reader.read_interleaved(&mut engine_frame).unwrap();
        }
    });
}

#[test]
fn process_loopback_capture_consumption_allocates_nothing() {
    // The native-rate stereo capture shape that process loopback streams use
    // (2 ch, 48 kHz f32, event-delivered packets). Packets are pushed into the
    // same prepared CaptureIngress and drained by the same ClockSource read
    // that the Engine render thread performs; the WASAPI GetBuffer/Release
    // bracket around `push_packet` is platform-side and carries no allocation.
    let config = ClockBridgeConfig {
        capacity_frames: 4096,
        target_fill_frames: 480,
        trim_on_prime: true,
        ..ClockBridgeConfig::default()
    };
    let (mut ingress, mut source, _observer) = capture_bridge(config).unwrap();
    let frames = 480usize;
    let payload = vec![0u8; frames * 2 * 4];
    let packet = CapturePacket {
        frames,
        flags: 0,
        device_position_frames: 0,
        qpc_100ns: 1_000,
    };
    // Prime past the fill target outside the guarded region so every guarded
    // read is a steady-state streaming pass, not first-fill servo work.
    let mut position = 0u64;
    for _ in 0..6 {
        ingress
            .push_packet(
                &payload,
                CapturePacket {
                    device_position_frames: position,
                    qpc_100ns: packet.qpc_100ns + position * 10,
                    ..packet
                },
            )
            .unwrap();
        position += frames as u64;
    }
    let mut out = [0.0f32; 512];
    source.read_interleaved(&mut out).unwrap();
    common::assert_allocates_nothing(|| {
        for index in 0..16u64 {
            ingress
                .push_packet(
                    &payload,
                    CapturePacket {
                        device_position_frames: position,
                        qpc_100ns: packet.qpc_100ns + position * 10,
                        ..packet
                    },
                )
                .unwrap();
            position += frames as u64;
            let report = source.read_interleaved(&mut out).unwrap();
            assert_eq!(
                report.transferred_frames,
                out.len() / 2,
                "steady-state pass {index}"
            );
        }
    });
}
