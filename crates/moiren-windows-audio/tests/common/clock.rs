//! Clock-bridge packet construction and consumption scaffolding shared by the
//! `clock_*` integration tests.
#![allow(dead_code)] // shared across test binaries; each compiles the subset it uses

use moiren_windows_audio::{
    clock_bridge::{CaptureIngress, CapturePacket, ClockSource},
    stats::SILENT,
};

/// QPC origin (100 ns units) the packet stamps are derived from.
pub const ORIGIN: u64 = 1_000_000;

/// Silent packet stamped as if captured at `rate` starting from [`ORIGIN`].
pub fn packet(frames: usize, position: u64, rate: u32) -> CapturePacket {
    CapturePacket {
        frames,
        flags: SILENT,
        device_position_frames: position,
        qpc_100ns: ORIGIN + (position * 10_000_000).div_ceil(u64::from(rate)),
    }
}

/// Pushes an empty silent packet and returns the accepted frame count.
pub fn push(ingress: &mut CaptureIngress, frames: usize, position: u64, rate: u32) -> usize {
    ingress
        .push_packet(&[], packet(frames, position, rate))
        .unwrap()
}

/// Consumes `frames` one frame at a time, asserting each read transfers
/// exactly one frame without xruns.
pub fn consume(source: &mut ClockSource, frames: usize) {
    for _ in 0..frames {
        let report = source.read_interleaved(&mut [0.0; 2]).unwrap();
        assert_eq!(report.transferred_frames, 1);
        assert_eq!(report.xruns, 0);
    }
}
