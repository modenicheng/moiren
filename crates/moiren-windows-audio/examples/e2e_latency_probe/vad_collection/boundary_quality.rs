//! Classified playback evidence belongs to one collector owner, never a fixed allowance.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;

#[derive(Default, Serialize)]
pub(super) struct Tracker {
    identity: Option<Identity>,
    first_observed: Option<Sample>,
    startup: Option<Sample>,
    latest: Option<Sample>,
    latest_raw: Option<Raw>,
    steady_delta: Option<Sample>,
    unknown_observations: u64,
    pending_initial_observations: u64,
    classification_available: bool,
    final_after_join: bool,
}

#[derive(PartialEq, Eq, Serialize)]
struct Identity {
    owner_generation: u64,
    node: String,
    capacity_frames: u64,
    target_frames: Option<u64>,
}

#[derive(Clone, Copy, Serialize)]
struct Raw {
    soft_events: u64,
    xruns: u64,
}

#[derive(Clone, Copy, Serialize)]
struct Sample {
    soft_events: u64,
    xruns: u64,
    initial_prime_muted_frames: u64,
    initial_prime_complete: bool,
    recovery_prime_muted_frames: u64,
    short_read_callbacks: u64,
}

impl Sample {
    fn checked_delta(self, before: Self) -> Result<Self> {
        let delta = |now: u64, previous: u64| {
            now.checked_sub(previous)
                .context("playback boundary counter regressed")
        };
        Ok(Self {
            soft_events: delta(self.soft_events, before.soft_events)?,
            xruns: delta(self.xruns, before.xruns)?,
            initial_prime_muted_frames: delta(
                self.initial_prime_muted_frames,
                before.initial_prime_muted_frames,
            )?,
            initial_prime_complete: self.initial_prime_complete,
            recovery_prime_muted_frames: delta(
                self.recovery_prime_muted_frames,
                before.recovery_prime_muted_frames,
            )?,
            short_read_callbacks: delta(self.short_read_callbacks, before.short_read_callbacks)?,
        })
    }
}

fn number(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .filter(|count| *count != u64::MAX)
        .with_context(|| format!("playback boundary {key} unavailable, malformed or saturated"))
}

impl Tracker {
    pub(super) fn observe(
        &mut self,
        native: &Value,
        final_after_join: bool,
    ) -> Result<Vec<String>> {
        ensure!(
            !self.final_after_join,
            "playback boundary already finalized"
        );
        ensure!(
            native.get("running").and_then(Value::as_bool) == Some(!final_after_join),
            "playback boundary owner lifecycle unavailable or changed"
        );
        let owner_generation = number(native, "owner_generation")?;
        ensure!(owner_generation > 0, "playback boundary owner unavailable");
        let streams = native
            .get("streams")
            .and_then(Value::as_array)
            .context("playback boundary stream identities unavailable")?;
        let playback: Vec<_> = streams
            .iter()
            .filter(|stream| stream.get("kind").and_then(Value::as_str) == Some("virtual_playback"))
            .collect();
        ensure!(
            playback.len() == 1,
            "playback boundary stream missing or duplicated"
        );
        let node = playback[0]
            .get("node")
            .and_then(Value::as_str)
            .filter(|node| !node.trim().is_empty())
            .context("playback boundary node unavailable")?;
        let probes = native
            .get("boundary_probes")
            .and_then(Value::as_array)
            .context("playback boundary probes unavailable")?;
        let matching: Vec<_> = probes
            .iter()
            .filter(|probe| probe.get("node").and_then(Value::as_str) == Some(node))
            .collect();
        ensure!(
            matching.len() == 1,
            "playback boundary probe missing or duplicated"
        );
        let probe = matching[0];
        ensure!(
            probe.get("kind") == playback[0].get("kind"),
            "playback boundary kind changed"
        );
        let capacity_frames = number(probe, "capacity_frames")?;
        ensure!(
            capacity_frames > 0 && capacity_frames <= u64::from(u32::MAX),
            "playback boundary capacity invalid"
        );
        let target_frames = match probe.get("target_frames") {
            Some(Value::Null) => None,
            Some(value) => {
                let target = value
                    .as_u64()
                    .context("playback boundary target malformed")?;
                ensure!(
                    target > 0 && target <= capacity_frames,
                    "playback boundary target invalid"
                );
                Some(target)
            }
            None => anyhow::bail!("playback boundary target unavailable"),
        };
        let identity = Identity {
            owner_generation,
            node: node.to_owned(),
            capacity_frames,
            target_frames,
        };
        ensure!(
            self.identity
                .as_ref()
                .is_none_or(|before| *before == identity),
            "playback boundary owner/node/prepared shape changed"
        );
        let raw = Raw {
            soft_events: number(probe, "soft_events")?,
            xruns: number(probe, "xruns")?,
        };
        if let Some(before) = self.latest_raw {
            ensure!(
                raw.soft_events >= before.soft_events && raw.xruns >= before.xruns,
                "playback boundary raw counter regressed"
            );
        }
        let classified = probe
            .get("playback_events")
            .context("playback classification missing")?;
        if classified.is_null() {
            // One-attempt versioned reads can be contended even after startup.
            // Retain the last coherent sample as historical evidence, not as a
            // claim about this poll. Joined final evidence must be available.
            ensure!(
                !final_after_join,
                "playback classification unavailable after owner join"
            );
            self.unknown_observations = self
                .unknown_observations
                .checked_add(1)
                .context("playback unknown observation counter overflow")?;
            self.identity = Some(identity);
            self.latest_raw = Some(raw);
            self.classification_available = false;
            return Ok(if raw.xruns == 0 {
                Vec::new()
            } else {
                vec!["playback boundary xruns".into()]
            });
        }
        ensure!(
            classified.get("saturated").and_then(Value::as_bool) == Some(false),
            "playback classification saturated or saturation unknown"
        );
        let sample = Sample {
            soft_events: raw.soft_events,
            xruns: raw.xruns,
            initial_prime_muted_frames: number(classified, "initial_prime_muted_frames")?,
            initial_prime_complete: classified
                .get("initial_prime_complete")
                .and_then(Value::as_bool)
                .context("playback initial prime completion unavailable or malformed")?,
            recovery_prime_muted_frames: number(classified, "recovery_prime_muted_frames")?,
            short_read_callbacks: number(classified, "short_read_callbacks")?,
        };
        let mixed_total = sample
            .initial_prime_muted_frames
            .checked_add(sample.recovery_prime_muted_frames)
            .and_then(|frames| frames.checked_add(sample.short_read_callbacks))
            .context("playback boundary mixed-unit legacy total overflow")?;
        if let Some(before) = self.latest {
            sample.checked_delta(before)?;
            ensure!(
                !before.initial_prime_complete || sample.initial_prime_complete,
                "playback initial prime completion regressed"
            );
        }
        if final_after_join {
            ensure!(
                self.startup.is_some() && sample.initial_prime_complete,
                "joined playback requires completed live startup evidence"
            );
            // Only drift-adapted playback adds muted output *frames* and short
            // read *callbacks* to the legacy mixed-unit soft_events counter.
            // Live publication is independent, so equality is valid only here.
            if target_frames.is_some() {
                ensure!(
                    sample.soft_events == mixed_total,
                    "joined playback legacy/classified total mismatch"
                );
            } else {
                ensure!(
                    sample.initial_prime_muted_frames == 0
                        && sample.recovery_prime_muted_frames == 0
                        && sample.soft_events == 0,
                    "unadapted playback cannot claim prime frames or legacy soft events"
                );
            }
        }
        let startup = self
            .startup
            .or_else(|| sample.initial_prime_complete.then_some(sample));
        let steady_delta = startup
            .map(|before| sample.checked_delta(before))
            .transpose()?;
        let mut faults = Vec::new();
        if steady_delta.is_some_and(|delta| delta.initial_prime_muted_frames != 0) {
            faults.push("playback boundary initial-prime growth after startup baseline".into());
        }
        if sample.recovery_prime_muted_frames != 0 {
            faults.push("playback boundary recovery-prime muted frames".into());
        }
        if sample.short_read_callbacks != 0 {
            faults.push("playback boundary short-read callbacks".into());
        }
        if sample.xruns != 0 {
            faults.push("playback boundary xruns".into());
        }
        if !sample.initial_prime_complete {
            self.pending_initial_observations = self
                .pending_initial_observations
                .checked_add(1)
                .context("playback pending initial observation counter overflow")?;
        }
        self.identity = Some(identity);
        self.first_observed.get_or_insert(sample);
        self.startup = startup;
        self.latest = Some(sample);
        self.latest_raw = Some(raw);
        self.steady_delta = steady_delta;
        self.classification_available = true;
        self.final_after_join = final_after_join;
        Ok(faults)
    }
}

#[cfg(test)]
#[path = "boundary_quality/tests.rs"]
mod tests;
