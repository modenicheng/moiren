//! Clock omission is distinct from native PCM loss and must survive final join.
use anyhow::{Result, ensure};
use moiren_windows_audio::virtual_owner::{DEBT_CAP_PERIODS, VirtualClockSnapshot};

pub(in crate::live) fn live(
    sample: Option<&VirtualClockSnapshot>,
    pure_virtual: bool,
) -> Result<Option<u64>> {
    if pure_virtual {
        // A contended cumulative read is skipped, not a zero-loss observation.
        // The stopped/joined sample is still mandatory and covers trailing loss.
        sample.map(assess).transpose()
    } else {
        ensure!(
            sample.is_none(),
            "physical master cannot have a virtual clock"
        );
        Ok(None)
    }
}

pub(in crate::live) fn assess(sample: &VirtualClockSnapshot) -> Result<u64> {
    ensure!(
        sample.schema_version == 1
            && (8_000..=384_000).contains(&sample.engine_sample_rate)
            && sample.engine_block_frames > 0
            && (1..=DEBT_CAP_PERIODS).contains(&sample.recovery_cap_periods),
        "virtual clock schema or prepared shape invalid"
    );
    let counters = sample.counters;
    ensure!(
        !counters.counters_saturated,
        "virtual clock counters saturated"
    );
    for count in [
        counters.rendered_periods,
        counters.rendered_frames,
        counters.late_wake_bursts,
        counters.clamped_periods,
        counters.clamped_frames,
        counters.timeline_resets,
        counters.max_backlog_periods,
    ] {
        ensure!(count != u64::MAX, "virtual clock counter exhausted");
    }
    let block = u64::from(sample.engine_block_frames);
    ensure!(
        counters.rendered_periods.checked_mul(block) == Some(counters.rendered_frames)
            && counters.clamped_periods.checked_mul(block) == Some(counters.clamped_frames)
            && (counters.rendered_periods == 0 || counters.max_backlog_periods > 0)
            && (counters.clamped_periods == 0
                || counters.max_backlog_periods > sample.recovery_cap_periods),
        "virtual clock period/frame accounting inconsistent"
    );
    // A reset changes transport epochs; correctness grace cannot excuse it.
    ensure!(
        counters.timeline_resets == 0,
        "virtual clock reset transport epochs"
    );
    Ok(counters.clamped_frames)
}

pub(super) fn parse_final(value: &serde_json::Value) -> Result<u64> {
    let sample: VirtualClockSnapshot = serde_json::from_value(value.clone())?;
    let loss = assess(&sample)?;
    ensure!(
        sample.counters.rendered_periods > 0,
        "virtual clock never rendered"
    );
    Ok(loss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_windows_audio::virtual_owner::VirtualClockCounters;

    fn sample() -> VirtualClockSnapshot {
        VirtualClockSnapshot {
            schema_version: 1,
            engine_sample_rate: 48_000,
            engine_block_frames: 32,
            recovery_cap_periods: 1,
            counters: VirtualClockCounters {
                rendered_periods: 2,
                rendered_frames: 64,
                max_backlog_periods: 1,
                ..Default::default()
            },
        }
    }

    #[test]
    fn unavailable_live_sample_stays_unknown_and_cannot_replace_final_evidence() {
        let coherent = sample();
        for current in [Some(&coherent), None, Some(&coherent)] {
            assert_eq!(live(current, true).unwrap(), current.map(|_| 0));
        }
        assert!(parse_final(&serde_json::Value::Null).is_err());
        assert_eq!(live(None, false).unwrap(), None);
        assert!(live(Some(&coherent), false).is_err());
    }

    #[test]
    fn present_invalid_samples_are_never_hidden_as_unavailable_or_clean() {
        for variant in 0..3 {
            let mut bad = sample();
            match variant {
                0 => bad.counters.counters_saturated = true,
                1 => bad.counters.timeline_resets = 1,
                _ => bad.counters.rendered_frames = 1,
            }
            assert!(live(Some(&bad), true).is_err());
        }
        let mut lost = sample();
        lost.counters.clamped_frames = 288;
        lost.counters.clamped_periods = 9;
        lost.counters.max_backlog_periods = 10;
        assert_eq!(live(Some(&lost), true).unwrap(), Some(288));
    }
}
