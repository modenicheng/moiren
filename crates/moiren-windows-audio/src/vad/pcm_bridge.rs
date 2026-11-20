//! Fixed-size PCM conversion stays on transport workers, never Engine render.

use super::{Channels, PcmFormatError, SampleFormat, decode_interleaved, encode_interleaved};
use moiren_engine::{
    boundary::{AudioReader, AudioWriter},
    host::BackendError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmBoundaryError {
    ZeroPacketFrames,
    PacketSizeOverflow,
    BufferBudgetExceeded,
    InvalidCarrier,
    WrongPacketSize { expected: usize, actual: usize },
    Format(PcmFormatError),
}

impl std::fmt::Display for PcmBoundaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VAD PCM boundary: {self:?}")
    }
}

impl std::error::Error for PcmBoundaryError {}

impl From<PcmFormatError> for PcmBoundaryError {
    fn from(value: PcmFormatError) -> Self {
        Self::Format(value)
    }
}

impl From<PcmBoundaryError> for BackendError {
    fn from(value: PcmBoundaryError) -> Self {
        Self::Unsupported(value.to_string())
    }
}

fn packet_buffers(
    channels: Channels,
    format: SampleFormat,
    packet_frames: usize,
    capacity_frames: usize,
    byte_budget: usize,
) -> Result<(Vec<f32>, Vec<u8>), PcmBoundaryError> {
    if packet_frames == 0 {
        return Err(PcmBoundaryError::ZeroPacketFrames);
    }
    let carrier_samples = packet_frames
        .checked_mul(2)
        .ok_or(PcmBoundaryError::PacketSizeOverflow)?;
    let carrier_bytes = capacity_frames
        .checked_mul(2 * std::mem::size_of::<f32>())
        .ok_or(PcmBoundaryError::PacketSizeOverflow)?;
    let pcm_bytes = packet_frames
        .checked_mul(channels as usize)
        .and_then(|samples| samples.checked_mul(format.bytes_per_sample()))
        .ok_or(PcmBoundaryError::PacketSizeOverflow)?;
    let scratch_bytes = carrier_samples
        .checked_mul(std::mem::size_of::<f32>())
        .and_then(|bytes| bytes.checked_add(pcm_bytes))
        .ok_or(PcmBoundaryError::PacketSizeOverflow)?;
    if carrier_bytes
        .checked_add(scratch_bytes)
        .is_none_or(|bytes| bytes > byte_budget)
        || carrier_bytes > isize::MAX as usize
        || scratch_bytes > isize::MAX as usize
    {
        return Err(PcmBoundaryError::BufferBudgetExceeded);
    }
    Ok((vec![0.0; carrier_samples], vec![0; pcm_bytes]))
}

/// Receives the native application's playback packets and publishes decoded
/// samples to the Engine's prepared carrier. Called only by the PCM worker.
pub struct PcmPlaybackBridge {
    channels: Channels,
    format: SampleFormat,
    carrier: AudioWriter<f32>,
    engine_packet: Vec<f32>,
    pcm_packet: Vec<u8>,
}

impl PcmPlaybackBridge {
    pub fn new(
        channels: Channels,
        format: SampleFormat,
        packet_frames: usize,
        byte_budget: usize,
        carrier: AudioWriter<f32>,
    ) -> Result<Self, PcmBoundaryError> {
        if carrier.channel_count() != 2 || carrier.capacity_frames() < packet_frames {
            return Err(PcmBoundaryError::InvalidCarrier);
        }
        let (engine_packet, pcm_packet) = packet_buffers(
            channels,
            format,
            packet_frames,
            carrier.capacity_frames(),
            byte_budget,
        )?;
        Ok(Self {
            channels,
            format,
            carrier,
            engine_packet,
            pcm_packet,
        })
    }

    pub fn packet_bytes(&self) -> usize {
        self.pcm_packet.len()
    }

    pub fn packet_frames(&self) -> u32 {
        u32::try_from(self.engine_packet.len() / 2).unwrap_or(u32::MAX)
    }

    /// Return the carrier when a native stream is retired. The transport
    /// worker can then rebuild only its format-dependent scratch state after a
    /// stream reopen without moving the Engine-facing bridge endpoint.
    pub fn into_carrier(self) -> AudioWriter<f32> {
        self.carrier
    }

    /// Returns false when the bounded Engine carrier was full; it drops the
    /// newest packet instead of creating unbounded latency.
    pub fn decode_packet(&mut self, pcm: &[u8]) -> Result<bool, PcmBoundaryError> {
        if pcm.len() != self.pcm_packet.len() {
            return Err(PcmBoundaryError::WrongPacketSize {
                expected: self.pcm_packet.len(),
                actual: pcm.len(),
            });
        }
        decode_interleaved(pcm, &mut self.engine_packet, self.format, self.channels)?;
        Ok(self
            .carrier
            .write_interleaved_exact(&self.engine_packet)
            .expect("prepared stereo carrier accepts interleaved frames"))
    }
}

/// Encodes the Engine's bounded microphone carrier into one exact native PCM
/// packet. Empty carrier reads are silence-filled by `AudioReader`.
pub struct PcmMicrophoneBridge {
    channels: Channels,
    format: SampleFormat,
    carrier: AudioReader<f32>,
    engine_packet: Vec<f32>,
    pcm_packet: Vec<u8>,
}

impl PcmMicrophoneBridge {
    pub fn new(
        channels: Channels,
        format: SampleFormat,
        packet_frames: usize,
        byte_budget: usize,
        carrier: AudioReader<f32>,
    ) -> Result<Self, PcmBoundaryError> {
        if carrier.channel_count() != 2 || carrier.capacity_frames() < packet_frames {
            return Err(PcmBoundaryError::InvalidCarrier);
        }
        let (engine_packet, pcm_packet) = packet_buffers(
            channels,
            format,
            packet_frames,
            carrier.capacity_frames(),
            byte_budget,
        )?;
        Ok(Self {
            channels,
            format,
            carrier,
            engine_packet,
            pcm_packet,
        })
    }

    pub fn packet_bytes(&self) -> usize {
        self.pcm_packet.len()
    }

    pub fn packet_frames(&self) -> u32 {
        u32::try_from(self.engine_packet.len() / 2).unwrap_or(u32::MAX)
    }

    /// Engine-authored frames currently queued in the carrier. Worker-side
    /// only: the re-prime wait and diagnostics observe the level; the graph
    /// producer is the only writer.
    pub fn available_frames(&self) -> usize {
        self.carrier.available_frames()
    }

    /// Return the carrier when a native stream is retired so a later stream
    /// generation can rebuild its packet conversion state on the worker.
    pub fn into_carrier(self) -> AudioReader<f32> {
        self.carrier
    }

    /// Drains one fixed-size packet; any missing Engine frames are silence.
    pub fn encode_packet(&mut self) -> Result<&[u8], PcmBoundaryError> {
        Ok(self.encode_packet_with_metrics()?.0)
    }

    /// Encodes a packet and returns the silence count captured during the same
    /// mutable operation. Returning the scalar beside the borrow avoids a
    /// second borrow of this bridge while the packet is submitted.
    pub fn encode_packet_with_metrics(&mut self) -> Result<(&[u8], usize), PcmBoundaryError> {
        let report = self
            .carrier
            .read_interleaved(&mut self.engine_packet)
            .expect("prepared stereo carrier accepts interleaved frames");
        if report.transferred_frames < self.engine_packet.len() / 2 {
            log::debug!(
                target: "moiren::audio::vad",
                "event=microphone_carrier_short requested_frames={} transferred_frames={} qpc={:?} report={report:?}",
                self.engine_packet.len() / 2,
                report.transferred_frames,
                diagnostic_qpc(),
            );
        }
        let silence_frames =
            (self.engine_packet.len() / 2).saturating_sub(report.transferred_frames);
        encode_interleaved(
            &self.engine_packet,
            &mut self.pcm_packet,
            self.format,
            self.channels,
        )?;
        Ok((&self.pcm_packet, silence_frames))
    }
}

fn diagnostic_qpc() -> Option<u64> {
    #[cfg(windows)]
    {
        let mut value = 0;
        // SAFETY: this worker-only diagnostic owns writable counter storage.
        // It runs after a short read, never from Engine render.
        unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut value) }.ok()?;
        (value > 0).then_some(value as u64)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::vad::{
        Channels, PcmBoundaryError, PcmMicrophoneBridge, PcmPlaybackBridge, SampleFormat,
    };
    use moiren_engine::boundary::audio_bridge;

    #[test]
    fn playback_packet_decodes_mono_without_duplicating_channels() {
        let (engine_writer, mut carrier_reader) = audio_bridge(2, 8, 4096).unwrap();
        let mut bridge =
            PcmPlaybackBridge::new(Channels::Mono, SampleFormat::Pcm16, 2, 4096, engine_writer)
                .unwrap();
        bridge.decode_packet(&[0x00, 0x40, 0x00, 0xc0]).unwrap();
        let mut carrier = [f32::NAN; 4];
        carrier_reader.read_interleaved(&mut carrier).unwrap();
        assert_eq!(carrier, [0.5, 0.0, -0.5, 0.0]);
    }

    #[test]
    fn playback_drops_a_packet_without_publishing_a_partial_tail() {
        let (engine_writer, mut carrier_reader) = audio_bridge(2, 3, 4096).unwrap();
        let mut bridge =
            PcmPlaybackBridge::new(Channels::Mono, SampleFormat::Pcm16, 2, 4096, engine_writer)
                .unwrap();

        assert!(bridge.decode_packet(&[0x00, 0x20, 0x00, 0x40]).unwrap());
        assert!(!bridge.decode_packet(&[0x00, 0xc0, 0x00, 0xe0]).unwrap());

        let mut carrier = [f32::NAN; 6];
        carrier_reader.read_interleaved(&mut carrier).unwrap();
        assert_eq!(carrier, [0.25, 0.0, 0.5, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn microphone_packet_encodes_silence_for_missing_engine_frames() {
        let (mut engine_writer, carrier_reader) = audio_bridge(2, 8, 4096).unwrap();
        engine_writer.write_interleaved(&[0.25, -0.5]).unwrap();
        let mut bridge = PcmMicrophoneBridge::new(
            Channels::Stereo,
            SampleFormat::Pcm16,
            2,
            4096,
            carrier_reader,
        )
        .unwrap();
        assert_eq!(
            bridge.encode_packet().unwrap(),
            &[0x00, 0x20, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn microphone_packet_metrics_report_only_the_missing_tail() {
        let (mut engine_writer, carrier_reader) = audio_bridge(2, 8, 4096).unwrap();
        engine_writer.write_interleaved(&[0.25, -0.5]).unwrap();
        let mut bridge = PcmMicrophoneBridge::new(
            Channels::Stereo,
            SampleFormat::Pcm16,
            2,
            4096,
            carrier_reader,
        )
        .unwrap();

        let (packet, silence_frames) = bridge.encode_packet_with_metrics().unwrap();
        assert_eq!(packet.len(), 8);
        assert_eq!(silence_frames, 1);
        assert_eq!(bridge.packet_frames(), 2);
    }

    #[test]
    fn playback_rejects_wrong_packet_lengths_and_excessive_staging() {
        let (engine_writer, _) = audio_bridge(2, 8, 4096).unwrap();
        let mut bridge = PcmPlaybackBridge::new(
            Channels::Stereo,
            SampleFormat::Float32,
            2,
            4096,
            engine_writer,
        )
        .unwrap();
        assert!(matches!(
            bridge.decode_packet(&[0; 4]),
            Err(PcmBoundaryError::WrongPacketSize { .. })
        ));
        let (engine_writer, _) = audio_bridge(2, 8, 4096).unwrap();
        assert!(matches!(
            PcmPlaybackBridge::new(
                Channels::Stereo,
                SampleFormat::Float32,
                2,
                80,
                engine_writer,
            ),
            Err(PcmBoundaryError::BufferBudgetExceeded)
        ));
    }
}
