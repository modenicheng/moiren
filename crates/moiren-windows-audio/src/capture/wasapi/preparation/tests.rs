use super::*;
use crate::{SessionDuration, capture::CaptureSource};

#[test]
fn configured_process_preparation_changes_real_priming_and_reports_the_attached_queue() {
    use crate::{
        clock_bridge::CapturePacket,
        latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
        process_loopback::ProcessBridgeOptions,
    };
    let options = ProcessBridgeOptions {
        input: TransportInputPolicy::Target {
            capacity_frames: 8192,
            target_frames: 1024,
            jitter_frames: 96,
        },
        delivery: Some(InputDeliveryTiming {
            sample_rate: 48_000,
            max_packet_frames: 480,
            max_gap_frames: 480,
            source: DeliveryTimingSource::Declared,
        }),
        ..Default::default()
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let preparation = CapturePreparation::Configured {
        sender,
        config: options.clock_config().unwrap(),
    };
    let mut report = CaptureReport::new(
        CaptureSource::ProcessLoopback,
        SessionDuration::UntilStopped,
    );
    let mut ingress = preparation
        .prepare(
            NativeStreamTiming {
                sample_rate: 48_000,
                period_frames: None,
                capacity_frames: 480,
            },
            2,
            &mut report,
            None,
        )
        .unwrap();
    let startup = receiver.recv().unwrap().unwrap();
    let mut source = startup.source;
    assert_eq!(source.prepared_input(), Some(options.prepare().unwrap()));
    assert_eq!(report.input_queue, source.prepared_input());
    assert_eq!(source.prepared_input().unwrap().capacity_frames, 8192);
    // Three 480-frame publications cross 1024 but do not reach the old 2048
    // threshold. Passing this requires the actual reader to use the new target.
    let pcm: Vec<u8> = (0..480)
        .flat_map(|_| [0.25f32, -0.5].into_iter().flat_map(f32::to_le_bytes))
        .collect();
    for index in 0..3 {
        ingress
            .push_packet(
                &pcm,
                CapturePacket {
                    frames: 480,
                    flags: 0,
                    device_position_frames: 0,
                    qpc_100ns: 1 + index * 100_000,
                },
            )
            .unwrap();
    }
    let mut output = [0.0; 2];
    assert_eq!(
        source
            .read_interleaved(&mut output)
            .unwrap()
            .transferred_frames,
        1
    );
    assert_eq!(output, [0.25, -0.5]);
    assert_eq!(startup.observer.snapshot().discontinuities, 0);
    assert_eq!(report.shared_period.actual_period_frames, None);
}

#[test]
fn compatible_process_bridge_retains_legacy_fifo_without_running_backlog_recovery() {
    let options = crate::process_loopback::ProcessBridgeOptions::default();
    let config = options.clock_config().unwrap();
    assert_eq!(config.capacity_frames, 8192);
    assert_eq!(config.target_fill_frames, 2048);
    assert_eq!(config.prepared_input, None);
    assert!(!config.detect_position_gaps);
    let (_, source, _) = crate::clock_bridge::capture_bridge(config).unwrap();
    assert_eq!(source.prepared_input(), None);
}

#[test]
fn configured_position_clock_requires_physical_capture_and_matching_units() {
    use crate::{
        clock_bridge::CapturePacket, latency::shared_period::SharedFormatInfo, stats::SILENT,
    };
    for source_kind in [CaptureSource::Physical, CaptureSource::ProcessLoopback] {
        for mix_rate in [48_000, 96_000] {
            let (sender, receiver) = mpsc::sync_channel(1);
            let preparation = CapturePreparation::Configured {
                sender,
                config: ClockBridgeConfig {
                    capacity_frames: 64,
                    target_fill_frames: 2,
                    trim_on_prime: false,
                    max_correction_ppm: 0.0,
                    detect_position_gaps: true,
                    ..Default::default()
                },
            };
            let mut report = CaptureReport::new(source_kind, SessionDuration::UntilStopped);
            report.shared_period.mix_format = Some(SharedFormatInfo {
                sample_rate: mix_rate,
                channels: 2,
                container_bits: 32,
                valid_bits: None,
                channel_mask: None,
                floating_point: true,
            });
            let mut ingress = preparation
                .prepare(
                    NativeStreamTiming {
                        sample_rate: 48_000,
                        period_frames: None,
                        capacity_frames: 64,
                    },
                    2,
                    &mut report,
                    None,
                )
                .unwrap();
            let startup = receiver.recv().unwrap().unwrap();
            for index in 0..2u64 {
                ingress
                    .push_packet(
                        &[],
                        CapturePacket {
                            frames: 4,
                            flags: SILENT,
                            device_position_frames: index * 4,
                            qpc_100ns: 1_000_000 + (index * 4 * 10_000_000).div_ceil(48_000),
                        },
                    )
                    .unwrap();
            }
            let mut source = startup.source;
            // The first packet only anchors correlation. Advance to the
            // second packet whose continuous position/QPC can be qualified.
            assert_eq!(
                source
                    .read_interleaved(&mut [0.0; 8])
                    .unwrap()
                    .transferred_frames,
                4
            );
            assert_eq!(
                source.oldest_frame_stamp().is_some(),
                source_kind == CaptureSource::Physical && mix_rate == 48_000
            );
        }
    }
}
