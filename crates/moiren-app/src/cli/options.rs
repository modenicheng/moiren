use anyhow::{Context, Result, bail, ensure};

use crate::service::{
    AudioSettings, DeliveryProvenance, ExpertInputBuffer, ProcessingPrecision,
    TransportLatencySettings,
};
use moiren_engine::node::DeviceMode;

#[derive(Debug, Clone)]
pub(super) struct MonitorOptions {
    pub pid: Option<u32>,
    pub output: Option<String>,
    pub seconds: u16,
    pub startup_timeout_ms: u64,
    pub audio: AudioSettings,
    target: Option<u32>,
    capacity: Option<u32>,
    jitter: Option<u32>,
    packet: Option<u32>,
    gap: Option<u32>,
    age: Option<u64>,
}

impl Default for MonitorOptions {
    fn default() -> Self {
        Self {
            pid: None,
            output: None,
            seconds: 30,
            startup_timeout_ms: 15000,
            audio: AudioSettings::default(),
            target: None,
            capacity: None,
            jitter: None,
            packet: None,
            gap: None,
            age: None,
        }
    }
}

impl MonitorOptions {
    pub fn set(&mut self, flag: &str, value: &str) -> Result<()> {
        let number = || {
            value
                .parse::<u32>()
                .with_context(|| format!("invalid {flag} value"))
        };
        match flag {
            "--process" => self.pid = Some(number()?),
            "--output" => self.output = Some(value.into()),
            "--seconds" => self.seconds = value.parse().context("invalid --seconds value")?,
            "--startup-timeout-ms" => {
                self.startup_timeout_ms = value.parse().context("invalid startup timeout")?
            }
            "--engine-block-frames" => self.audio.processing.block_frames = number()?,
            "--output-buffer-frames" => self.audio.output.buffer_frames = Some(number()?),
            "--output-mode" => {
                self.audio.output.mode = match value {
                    "shared" => DeviceMode::Shared,
                    "exclusive" => DeviceMode::Exclusive,
                    _ => bail!("--output-mode must be shared or exclusive"),
                }
            }
            "--precision" => {
                self.audio.processing.precision = match value {
                    "f32" => ProcessingPrecision::F32,
                    "f64" => ProcessingPrecision::F64,
                    _ => bail!("--precision must be f32 or f64"),
                }
            }
            "--target-frames" => self.target = Some(number()?),
            "--capacity-frames" => self.capacity = Some(number()?),
            "--jitter-frames" => self.jitter = Some(number()?),
            "--max-packet-frames" => self.packet = Some(number()?),
            "--max-gap-frames" => self.gap = Some(number()?),
            "--age-limit-100ns" => {
                self.age = Some(value.parse().context("invalid sample-age limit")?)
            }
            _ => bail!("unknown monitor option: {flag}; use monitor --help"),
        }
        Ok(())
    }

    pub fn transport(&self) -> Result<TransportLatencySettings> {
        let fields = [
            self.target,
            self.capacity,
            self.jitter,
            self.packet,
            self.gap,
        ];
        let supplied = fields.iter().filter(|field| field.is_some()).count();
        ensure!(
            supplied == 0 || supplied == fields.len(),
            "explicit Process budget requires target, capacity, jitter, max-packet and max-gap together"
        );
        let mut settings = TransportLatencySettings::default();
        if supplied != 0 {
            settings.process = Some(ExpertInputBuffer {
                target_frames: self.target.expect("complete tuple"),
                capacity_frames: self.capacity.expect("complete tuple"),
                jitter_frames: self.jitter.expect("complete tuple"),
                max_packet_frames: self.packet.expect("complete tuple"),
                max_gap_frames: self.gap.expect("complete tuple"),
                age_limit_100ns: self.age,
                provenance: DeliveryProvenance::Declared,
            });
        }
        // Compatible must not silently discard unsupported age intent either.
        if supplied == 0 && self.age.is_some() {
            bail!(
                "Process sample-age enforcement unsupported: packet timestamps do not establish sample age"
            );
        }
        settings.validate().map_err(anyhow::Error::msg)?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.pid
                .is_some_and(|pid| pid != 0 && pid != std::process::id()),
            "--process requires a nonzero PID other than Moiren itself"
        );
        ensure!(
            self.output
                .as_ref()
                .is_some_and(|output| !output.trim().is_empty()
                    && !output.contains('\0')
                    && output.len() <= 32768),
            "--output requires an explicit endpoint identity"
        );
        ensure!(
            (1..=600).contains(&self.seconds),
            "--seconds must be 1..600"
        );
        ensure!(
            (1..=30000).contains(&self.startup_timeout_ms),
            "--startup-timeout-ms must be 1..30000"
        );
        self.audio.validate()?;
        ensure!(
            self.age.is_none(),
            "Process sample-age enforcement unsupported: packet timestamps do not establish sample age"
        );
        let transport = self.transport()?;
        #[cfg(windows)]
        transport.process_options().prepare()?;
        #[cfg(not(windows))]
        let _ = transport;
        Ok(())
    }
}
