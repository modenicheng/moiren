//! Machine-local transport intent. Native owners validate it against their
//! complete negotiated demand; these bounds are never hardware latency.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryProvenance {
    #[default]
    Declared,
    Empirical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertInputBuffer {
    pub capacity_frames: u32,
    pub target_frames: u32,
    pub jitter_frames: u32,
    pub max_packet_frames: u32,
    pub max_gap_frames: u32,
    /// Process-only requested sample-age limit, not measured queue duration.
    /// Persisted as intent; native Process preparation explicitly refuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_limit_100ns: Option<u64>,
    #[serde(default)]
    pub provenance: DeliveryProvenance,
}

impl Default for ExpertInputBuffer {
    fn default() -> Self {
        Self {
            capacity_frames: 8192,
            target_frames: 2048,
            jitter_frames: 96,
            max_packet_frames: 480,
            max_gap_frames: 480,
            age_limit_100ns: None,
            provenance: DeliveryProvenance::Declared,
        }
    }
}

impl ExpertInputBuffer {
    /// Portable admission checks only shape and bounded storage. The platform
    /// retains authority over FIR reserve, packet shape and actual demand.
    pub fn validate(self) -> Result<(), String> {
        const MAX_FRAMES: u32 = 262_144;
        if !(4..=MAX_FRAMES).contains(&self.capacity_frames)
            || !(2..self.capacity_frames).contains(&self.target_frames)
            || self.jitter_frames >= self.capacity_frames
            || !(1..self.capacity_frames).contains(&self.max_packet_frames)
            || !(1..self.capacity_frames).contains(&self.max_gap_frames)
        {
            return Err("expert input buffer values exceed the supported frame range".into());
        }
        if self.target_frames >= self.capacity_frames - self.max_packet_frames
            || self.target_frames <= self.max_gap_frames + self.jitter_frames
        {
            return Err("expert input target must leave packet headroom and cover delivery gap plus jitter; native preparation may require more reserve".into());
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransportLatencySettings {
    /// None preserves the platform's Compatible policy. All explicit frame
    /// bounds refer to the fixed 48 kHz input transport, not Engine segments.
    pub process: Option<ExpertInputBuffer>,
    pub vad: Option<ExpertInputBuffer>,
}

impl TransportLatencySettings {
    pub fn validate(self) -> Result<(), String> {
        if self
            .vad
            .is_some_and(|input| input.age_limit_100ns.is_some())
        {
            return Err("VAD: age_limit_100ns is Process-only requested intent".into());
        }
        for (name, input) in [("Process", self.process), ("VAD", self.vad)] {
            if let Some(input) = input {
                input
                    .validate()
                    .map_err(|error| format!("{name}: {error}"))?;
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    pub(crate) fn process_options(
        self,
    ) -> moiren_windows_audio::process_loopback::ProcessBridgeOptions {
        moiren_windows_audio::process_loopback::ProcessBridgeOptions {
            input: policy(self.process),
            delivery: delivery(self.process),
            age_limit_100ns: self.process.and_then(|input| input.age_limit_100ns),
            engine_sample_rate: 48_000,
            // Static preflight must not invent a 256-frame consumer. The
            // output owner replaces this with its complete prepared demand.
            max_demand_frames: 1,
        }
    }

    #[cfg(windows)]
    pub(crate) fn virtual_options(
        self,
    ) -> moiren_windows_audio::virtual_owner::VirtualBufferOptions {
        moiren_windows_audio::virtual_owner::VirtualBufferOptions {
            input: policy(self.vad),
            delivery: delivery(self.vad),
            ..Default::default()
        }
    }
}

#[cfg(windows)]
fn policy(
    input: Option<ExpertInputBuffer>,
) -> moiren_windows_audio::latency::transport::TransportInputPolicy {
    use moiren_windows_audio::latency::transport::TransportInputPolicy;
    input.map_or(TransportInputPolicy::Compatible, |input| {
        TransportInputPolicy::Target {
            capacity_frames: input.capacity_frames,
            target_frames: input.target_frames,
            jitter_frames: input.jitter_frames,
        }
    })
}

#[cfg(windows)]
fn delivery(
    input: Option<ExpertInputBuffer>,
) -> Option<moiren_windows_audio::latency::transport::InputDeliveryTiming> {
    use moiren_windows_audio::latency::transport::{DeliveryTimingSource, InputDeliveryTiming};
    input.map(|input| InputDeliveryTiming {
        sample_rate: 48_000,
        max_packet_frames: input.max_packet_frames,
        max_gap_frames: input.max_gap_frames,
        source: match input.provenance {
            DeliveryProvenance::Declared => DeliveryTimingSource::Declared,
            DeliveryProvenance::Empirical => DeliveryTimingSource::Empirical,
        },
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn sub_256_expert_requests_use_one_frame_preflight_then_actual_native_demand() {
        let input = ExpertInputBuffer {
            capacity_frames: 1024,
            target_frames: 128,
            jitter_frames: 0,
            max_packet_frames: 64,
            max_gap_frames: 32,
            provenance: DeliveryProvenance::Empirical,
            age_limit_100ns: None,
        };
        let settings = TransportLatencySettings {
            process: Some(input),
            vad: Some(input),
        };
        settings.validate().unwrap();
        let process = settings.process_options();
        assert_eq!(process.max_demand_frames, 1);
        assert_eq!(process.prepare().unwrap().prime_frames, 128);
        assert!(process.for_consumer(48_000, 256).is_err());
        assert_eq!(
            process.delivery.unwrap().source,
            moiren_windows_audio::latency::transport::DeliveryTimingSource::Empirical
        );
        let vad = settings.virtual_options();
        vad.validate().unwrap();
        assert_eq!(vad.prepare(48_000, 1).unwrap().unwrap().prime_frames, 128);
        assert!(vad.prepare(48_000, 256).is_err());
        assert!(vad.admit_native_packet(128).is_err());
    }

    #[test]
    fn compatible_defaults_and_native_preparation_errors_remain_explicit() {
        let defaults = TransportLatencySettings::default();
        assert!(defaults.process_options().delivery.is_none());
        assert!(
            defaults
                .virtual_options()
                .prepare(48_000, 256)
                .unwrap()
                .is_none()
        );
        let settings = TransportLatencySettings {
            vad: Some(ExpertInputBuffer {
                capacity_frames: 512,
                target_frames: 40,
                jitter_frames: 0,
                max_packet_frames: 32,
                max_gap_frames: 32,
                ..Default::default()
            }),
            ..Default::default()
        };
        settings.validate().unwrap();
        assert!(
            settings.virtual_options().validate().is_err(),
            "FIR reserve belongs to native preparation"
        );
    }
}
