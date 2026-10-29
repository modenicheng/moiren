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
    /// Process host time since packet publication began. This is neither a
    /// source sample-age limit nor an end-to-end latency bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_max_ingress_elapsed_100ns: Option<u64>,
    /// Virtual-clock catch-up only, independent of input residency and physical
    /// output wake demand. None preserves the native eight-period ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vad_recovery_cap_periods: Option<u64>,
}

impl TransportLatencySettings {
    pub fn validate(self) -> Result<(), String> {
        if self.process_max_ingress_elapsed_100ns == Some(0) {
            return Err("Process: ingress elapsed limit must be positive".into());
        }
        if self
            .vad_recovery_cap_periods
            .is_some_and(|periods| !(1..=8).contains(&periods))
        {
            return Err("VAD: recovery cap must be within 1..=8 periods".into());
        }
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
            max_ingress_elapsed_100ns: self.process_max_ingress_elapsed_100ns,
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
            recovery_cap_periods: self.vad_recovery_cap_periods,
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
    fn ingress_elapsed_native_options_preserve_policy_and_whole_consumer_demand() {
        for limit in [None, Some(1), Some(123_456), Some(u64::MAX)] {
            for process in [None, Some(ExpertInputBuffer::default())] {
                let settings = TransportLatencySettings {
                    process,
                    process_max_ingress_elapsed_100ns: limit,
                    vad_recovery_cap_periods: Some(3),
                    ..Default::default()
                };
                settings.validate().unwrap();
                let options = settings.process_options();
                assert_eq!(options.max_ingress_elapsed_100ns, limit);
                assert_eq!(options.age_limit_100ns, None);
                assert_eq!(options.max_demand_frames, 1);
                let admitted = options.for_consumer(96_000, 512).unwrap();
                assert_eq!(admitted.max_ingress_elapsed_100ns, limit);
                assert_eq!(admitted.engine_sample_rate, 96_000);
                assert_eq!(admitted.max_demand_frames, 512);
                assert_eq!(settings.virtual_options().recovery_cap_periods, Some(3));
            }
        }
        let invalid = TransportLatencySettings {
            process_max_ingress_elapsed_100ns: Some(0),
            ..Default::default()
        };
        assert!(invalid.validate().is_err());
        assert!(invalid.process_options().prepare().is_err());
    }

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
            ..Default::default()
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

    #[test]
    fn vad_recovery_cap_controls_whole_virtual_burst_not_physical_wake_demand() {
        let defaults = TransportLatencySettings::default().virtual_options();
        assert_eq!(defaults.recovery_cap_periods, None);
        assert_eq!(defaults.admitted_recovery_cap_periods().unwrap(), 8);
        for cap in 1..=8 {
            let settings = TransportLatencySettings {
                vad_recovery_cap_periods: Some(cap),
                ..Default::default()
            };
            settings.validate().unwrap();
            let options = settings.virtual_options();
            assert_eq!(options.recovery_cap_periods, Some(cap));
            let prepared = options.prepare_recovery(48_000, 64).unwrap();
            assert_eq!(prepared.engine_burst_frames, 64 * cap as u32);
            assert_eq!(prepared.recovery_cap_periods, cap);
            assert!(
                prepared.queue.is_none(),
                "Compatible residency is unchanged"
            );
        }
        let mut settings = TransportLatencySettings {
            vad: Some(ExpertInputBuffer {
                capacity_frames: 1024,
                target_frames: 128,
                jitter_frames: 0,
                max_packet_frames: 32,
                max_gap_frames: 32,
                ..Default::default()
            }),
            vad_recovery_cap_periods: Some(1),
            ..Default::default()
        };
        settings.validate().unwrap();
        let options = settings.virtual_options();
        let prepared = options.prepare_recovery(48_000, 32).unwrap();
        assert_eq!(prepared.engine_burst_frames, 32);
        assert_eq!(prepared.queue.unwrap().prime_frames, 128);
        assert!(options.prepare_budget(48_000, 32, 256).is_err());
        assert!(options.admit_native_packet(64).is_err());
        settings.vad_recovery_cap_periods = None;
        assert!(
            settings
                .virtual_options()
                .prepare_recovery(48_000, 32)
                .is_err()
        );
    }
}
