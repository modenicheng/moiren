//! Native packet decoding and generation publication; no driver pointer escapes.
use super::{
    ClockBridgeConfig, ClockBridgeError, Frame, telemetry::Counters, timing::PacketTiming,
};
use crate::latency::{
    QpcClock,
    packets::{PacketHistoryWriter, PacketPublicationOrigin, PacketPublicationSample},
};
use crate::stats::{DISCONTINUITY, SILENT, TIMESTAMP_ERROR};
use rtrb::Producer;
use std::sync::{Arc, atomic::Ordering::Relaxed};

#[derive(Debug, Clone, Copy)]
pub struct CapturePacket {
    pub frames: usize,
    pub flags: u32,
    pub device_position_frames: u64,
    pub qpc_100ns: u64,
}
pub struct CaptureIngress {
    packet_events: PacketHistoryWriter,
    publication_clock: Option<QpcClock>,
    publication_clock_high_water_100ns: Option<u64>,
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    delivery_events: crate::latency::events::EventHistoryWriter,
    tx: Producer<Frame>,
    config: ClockBridgeConfig,
    generation: u64,
    next_position: Option<u64>,
    timing: PacketTiming,
    counters: Arc<Counters>,
}
impl CaptureIngress {
    pub(super) fn new(
        tx: Producer<Frame>,
        config: ClockBridgeConfig,
        counters: Arc<Counters>,
        delivery_events: crate::latency::events::EventHistoryWriter,
        packet_events: PacketHistoryWriter,
        publication_clock: Option<QpcClock>,
    ) -> Self {
        Self {
            packet_events,
            publication_clock,
            publication_clock_high_water_100ns: None,
            delivery_events,
            tx,
            config,
            generation: 1,
            next_position: None,
            timing: PacketTiming::default(),
            counters,
        }
    }
    pub fn input_channels(&self) -> usize {
        self.config.input_channels
    }

    /// Native owner only, after successful packet release or callback drain.
    /// The mutable ingress is the sole delivery publisher; graph writes never
    /// manufacture native packet arrival observations.
    #[cfg(any(windows, test))]
    pub(crate) fn publish_native_delivery(
        &mut self,
        sample: crate::latency::InputDeliverySnapshot,
    ) {
        self.counters.native_delivery.publish(sample);
    }

    /// One completed native drain, not one packet or per-sample timestamp.
    #[cfg(any(windows, test))]
    pub(crate) fn record_native_delivery_event(&mut self, frames: u64, qpc_100ns: Option<u64>) {
        self.delivery_events.record(frames, qpc_100ns);
    }

    /// SILENT packets carry no readable payload (contract documented on
    /// `capture_packet::borrow_capture_packet`). Overflow accepts a whole-frame
    /// prefix and drops the new tail; the next packet starts a fresh generation.
    pub fn push_packet(
        &mut self,
        bytes: &[u8],
        packet: CapturePacket,
    ) -> Result<usize, ClockBridgeError> {
        let clock = self.publication_clock;
        self.push_packet_with_publication_clock(bytes, packet, || {
            clock.as_ref().and_then(QpcClock::now)
        })
    }

    /// Bracket the actual ring publication with a prepared host clock. Called
    /// twice per valid nonempty packet, never per sample; sample/device stamps
    /// remain separate. Injectable for deterministic software verification.
    pub fn push_packet_with_publication_clock(
        &mut self,
        bytes: &[u8],
        packet: CapturePacket,
        mut clock: impl FnMut() -> Option<u64>,
    ) -> Result<usize, ClockBridgeError> {
        let silent = packet.flags & SILENT != 0;
        let width = self.config.input_channels * 4;
        let expected = packet
            .frames
            .checked_mul(width)
            .ok_or(ClockBridgeError::InvalidPacket)?;
        if !silent && bytes.len() != expected {
            return Err(ClockBridgeError::InvalidPacket);
        }
        if packet.frames == 0 {
            return Ok(0);
        }
        let invalid_time = packet.flags & TIMESTAMP_ERROR != 0 || packet.qpc_100ns == 0;
        let gap = self.config.detect_position_gaps
            && !invalid_time
            && self
                .next_position
                .is_some_and(|p| p != packet.device_position_frames);
        if gap || packet.flags & DISCONTINUITY != 0 {
            self.generation = self.generation.wrapping_add(1);
            self.counters.discontinuities.fetch_add(1, Relaxed);
        }
        self.next_position = if invalid_time {
            None
        } else {
            packet
                .device_position_frames
                .checked_add(packet.frames as u64)
        };
        if invalid_time {
            self.counters.timestamp_errors.fetch_add(1, Relaxed);
        } else {
            self.counters
                .last_device_position_frames
                .store(packet.device_position_frames, Relaxed);
            self.counters
                .last_qpc_100ns
                .store(packet.qpc_100ns, Relaxed);
        }
        self.counters
            .captured_frames
            .fetch_add(packet.frames as u64, Relaxed);
        if silent {
            self.counters
                .silent_frames
                .fetch_add(packet.frames as u64, Relaxed);
        }
        let before = clock();
        let ingress_started_100ns = before
            .filter(|&now| {
                self.publication_clock_high_water_100ns
                    .is_none_or(|previous| now >= previous)
            })
            .and_then(std::num::NonZeroU64::new);
        self.publication_clock_high_water_100ns = self
            .publication_clock_high_water_100ns
            .max(before.filter(|&now| now != 0));
        let accepted = if self.tx.is_abandoned() {
            0
        } else {
            packet.frames.min(self.tx.slots())
        };
        let packet_stamp = self.timing.accept(
            packet,
            accepted,
            self.generation,
            self.config.input_sample_rate,
            self.config.detect_position_gaps,
        );
        let mut nonfinite = 0u64;
        if accepted > 0 {
            let generation = self.generation;
            let channels = self.config.input_channels;
            let chunk = self
                .tx
                .write_chunk_uninit(accepted)
                .expect("reserved capture frames");
            chunk.fill_from_iter((0..accepted).map(|frame| {
                let mut samples = [0.0; 2];
                if !silent {
                    for (ch, sample) in samples.iter_mut().enumerate().take(channels) {
                        let offset = frame * width + ch * 4;
                        let value = f32::from_le_bytes(
                            bytes[offset..offset + 4]
                                .try_into()
                                .expect("validated f32 width"),
                        );
                        if value.is_finite() {
                            *sample = value;
                        } else {
                            nonfinite += 1;
                        }
                    }
                    if channels == 1 {
                        samples[1] = samples[0];
                    }
                }
                Frame {
                    samples,
                    generation,
                    packet_stamp,
                    packet_offset_frames: frame as u64,
                    ingress_started_100ns,
                }
            }));
        }
        let after = clock();
        // Completion cannot retroactively qualify frames already published,
        // but it fences the next packet against a regressed start observation.
        self.publication_clock_high_water_100ns = self
            .publication_clock_high_water_100ns
            .max(after.filter(|&now| now != 0));
        self.packet_events.record(PacketPublicationSample {
            offered_frames: packet.frames as u64,
            accepted_frames: accepted as u64,
            qpc_before_100ns: before,
            qpc_after_100ns: after,
            origin: PacketPublicationOrigin::NativeCapture,
            buffer_generation: self.generation,
            flags: packet.flags,
        });
        self.counters
            .nonfinite_samples
            .fetch_add(nonfinite, Relaxed);
        self.counters
            .written_frames
            .fetch_add(accepted as u64, Relaxed);
        self.counters
            .dropped_frames
            .fetch_add((packet.frames - accepted) as u64, Relaxed);
        if accepted < packet.frames {
            self.generation = self.generation.wrapping_add(1);
        }
        Ok(accepted)
    }
}
impl Drop for CaptureIngress {
    fn drop(&mut self) {
        self.counters.producer_finished.store(true, Relaxed);
    }
}

/// The same continuous clock adapter also accepts graph output. It carries
/// graph timeline continuity, rather than inventing hardware/QPC timestamps.
impl moiren_engine::boundary::RtAudioSink<f32> for CaptureIngress {
    fn channel_count(&self) -> usize {
        2
    }

    fn write(
        &mut self,
        context: &moiren_engine::processor::ProcessContext,
        input: moiren_engine::buffer::AudioBlock<'_, f32>,
    ) -> moiren_engine::boundary::BoundaryReport {
        use moiren_engine::boundary::BoundaryReport;
        if input.channel_count() != 2
            || self.config.input_channels != 2
            || self.config.input_sample_rate != crate::NATIVE_SAMPLE_RATE
        {
            return BoundaryReport {
                discontinuity: true,
                xruns: 1,
                ..BoundaryReport::default()
            };
        }
        let gap = self
            .next_position
            .is_some_and(|position| position != context.timeline_start);
        if gap {
            self.generation = self.generation.wrapping_add(1);
            self.counters.discontinuities.fetch_add(1, Relaxed);
        }
        self.next_position = context.timeline_start.checked_add(input.frames() as u64);
        let accepted = if self.tx.is_abandoned() {
            0
        } else {
            input.frames().min(self.tx.slots())
        };
        if accepted > 0 {
            let generation = self.generation;
            let chunk = self
                .tx
                .write_chunk_uninit(accepted)
                .expect("reserved output follower frames");
            chunk.fill_from_iter((0..accepted).map(|frame| Frame {
                samples: std::array::from_fn(|channel| {
                    let value = input.channel(channel)[frame];
                    if value.is_finite() {
                        value
                    } else {
                        self.counters.nonfinite_samples.fetch_add(1, Relaxed);
                        0.0
                    }
                }),
                generation,
                packet_stamp: None,
                packet_offset_frames: frame as u64,
                ingress_started_100ns: None,
            }));
        }
        if input.frames() > 0 {
            self.packet_events.record(PacketPublicationSample {
                offered_frames: input.frames() as u64,
                accepted_frames: accepted as u64,
                origin: PacketPublicationOrigin::GraphOutput,
                buffer_generation: self.generation,
                ..Default::default()
            });
        }
        self.counters
            .captured_frames
            .fetch_add(input.frames() as u64, Relaxed);
        self.counters
            .written_frames
            .fetch_add(accepted as u64, Relaxed);
        self.counters
            .dropped_frames
            .fetch_add((input.frames() - accepted) as u64, Relaxed);
        if accepted < input.frames() {
            self.generation = self.generation.wrapping_add(1);
        }
        BoundaryReport {
            transferred_frames: accepted,
            discontinuity: gap || accepted < input.frames(),
            xruns: u64::from(accepted < input.frames()),
        }
    }
}
