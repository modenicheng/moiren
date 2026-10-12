use moiren_windows_audio::clock_bridge::{CapturePacket, ClockBridgeConfig, capture_bridge};
fn run_case(capture_period: usize, render_period: usize, target: usize, jitter: bool, drift: f64) {
    let (mut ingress, mut source, observer) = capture_bridge(ClockBridgeConfig {
        target_fill_frames: target,
        ..ClockBridgeConfig::default()
    })
    .unwrap();
    let mut payload = vec![0u8; 8192 * 8];
    for sample in payload.chunks_exact_mut(4) {
        sample.copy_from_slice(&0.25f32.to_le_bytes());
    }
    let mut output = vec![0.0f32; render_period * 2];
    let mut incoming = 0.0f64;
    let mut position = 0u64;
    let mut capture_events = 0usize;
    let mut short_callbacks = 0usize;
    let mut received_audio = false;
    for time in (16..=120 * 48_000).step_by(16) {
        incoming += 16.0 * (1.0 + drift / 1_000_000.0);
        if time % capture_period == 0 {
            capture_events += 1;
            if !jitter || capture_events % 7 != 0 {
                let frames = incoming.floor() as usize;
                incoming -= frames as f64;
                ingress
                    .push_packet(
                        &payload[..frames * 8],
                        CapturePacket {
                            frames,
                            flags: 0,
                            device_position_frames: position,
                            qpc_100ns: time as u64 * 10_000_000 / 48_000,
                        },
                    )
                    .unwrap();
                position += frames as u64;
            }
        }
        if time % render_period == 0 {
            let mut transferred = 0;
            for segment in output.chunks_mut(256 * 2) {
                transferred += source.read_interleaved(segment).unwrap().transferred_frames;
            }
            if received_audio && transferred < render_period {
                short_callbacks += 1;
            }
            received_audio |= transferred > 0;
        }
    }
    let snapshot = observer.snapshot();
    println!(
        "{capture_period},{render_period},{target},{jitter},{drift:.0},{short_callbacks},{},{},{},{},{:.1},{}",
        snapshot.underrun_frames,
        snapshot.dropped_frames,
        snapshot.min_fill_frames,
        snapshot.max_fill_frames,
        snapshot.correction_ppm,
        snapshot.priming_frames
    );
}
fn main() {
    println!(
        "capture_period,render_period,target,jitter,drift_ppm,short_callbacks,underrun_frames,dropped_frames,min_fill,max_fill,correction_ppm,priming_frames"
    );
    for target in [2048, 1024, 512, 480, 256] {
        run_case(480, 480, target, false, 0.0);
    }
    for target in [2048, 1024, 512] {
        run_case(480, 480, target, true, 0.0);
    }
    for target in [512, 256, 128] {
        run_case(128, 128, target, true, 0.0);
    }
    for target in [1024, 512, 256] {
        run_case(480, 128, target, false, 0.0);
    }
    for drift in [-1000.0, 1000.0] {
        run_case(480, 480, 1024, true, drift);
        run_case(128, 128, 256, true, drift);
    }
    for drift in [-1000.0, 1000.0] {
        run_case(128, 128, 512, true, drift);
        run_case(480, 480, 512, false, drift);
    }
}
