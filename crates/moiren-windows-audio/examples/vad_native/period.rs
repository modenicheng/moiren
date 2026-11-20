//! Probe request evidence, never an assertion of the negotiated native cadence.
use anyhow::ensure;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct PeriodEvidence {
    pub qualification: &'static str,
    pub minimum_period: bool,
    pub default_hns: Option<i64>,
    pub minimum_hns: Option<i64>,
    pub initial_request_hns: Option<i64>,
    pub final_request_hns: Option<i64>,
    pub alignment_retry: bool,
}

impl PeriodEvidence {
    pub fn new(minimum_period: bool) -> Self {
        Self {
            qualification: "unqualified-request-only",
            minimum_period,
            default_hns: None,
            minimum_hns: None,
            initial_request_hns: None,
            final_request_hns: None,
            alignment_retry: false,
        }
    }

    pub fn select(&mut self, default_hns: i64, minimum_hns: i64) -> anyhow::Result<i64> {
        self.default_hns = Some(default_hns);
        self.minimum_hns = Some(minimum_hns);
        let selected = if self.minimum_period {
            minimum_hns
        } else {
            default_hns
        };
        ensure!(
            (1..=1_000_000).contains(&selected),
            "native selected period out of bounds: {selected}"
        );
        let request = if self.minimum_period {
            selected
        } else {
            selected.max(100_000)
        };
        self.initial_request_hns = Some(request);
        self.final_request_hns = Some(request);
        Ok(request)
    }

    pub fn record_alignment_retry(&mut self, request_hns: i64) {
        self.alignment_retry = true;
        self.final_request_hns = Some(request_hns);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_preserves_baseline_but_explicitly_requests_queried_minimum() {
        let mut baseline = PeriodEvidence::new(false);
        assert_eq!(baseline.select(100_000, 10_000).unwrap(), 100_000);
        assert_eq!(baseline.select(50_000, 10_000).unwrap(), 100_000);
        assert_eq!(baseline.select(200_000, 10_000).unwrap(), 200_000);
        let mut minimum = PeriodEvidence::new(true);
        assert_eq!(minimum.select(100_000, 10_000).unwrap(), 10_000);
        assert_eq!(minimum.initial_request_hns, Some(10_000));
        assert_eq!(minimum.final_request_hns, Some(10_000));
        assert!(!minimum.alignment_retry);
    }

    #[test]
    fn invalid_minimum_fails_without_fallback_and_retains_the_raw_query() {
        for invalid in [i64::MIN, -1, 0, 1_000_001, i64::MAX] {
            let mut evidence = PeriodEvidence::new(true);
            assert!(evidence.select(100_000, invalid).is_err(), "{invalid}");
            assert_eq!(evidence.default_hns, Some(100_000));
            assert_eq!(evidence.minimum_hns, Some(invalid));
            assert_eq!(evidence.initial_request_hns, None);
            assert_eq!(evidence.final_request_hns, None);
        }
    }

    #[test]
    fn minimum_selection_has_no_automatic_one_ms_assumption() {
        for minimum in [1, 10_001, 50_000, 100_000, 1_000_000] {
            let mut evidence = PeriodEvidence::new(true);
            assert_eq!(evidence.select(100_000, minimum).unwrap(), minimum);
        }
    }

    #[test]
    fn minimum_selection_validates_minimum_not_the_unused_default() {
        for default in [i64::MIN, -1, 0, 1_000_001, i64::MAX] {
            let mut evidence = PeriodEvidence::new(true);
            assert_eq!(evidence.select(default, 10_000).unwrap(), 10_000);
            assert_eq!(evidence.default_hns, Some(default));
        }
    }

    #[test]
    fn baseline_validation_does_not_gain_a_new_minimum_requirement() {
        assert_eq!(
            PeriodEvidence::new(false).select(100_000, 0).unwrap(),
            100_000
        );
        for invalid in [-1, 0, 1_000_001] {
            assert!(PeriodEvidence::new(false).select(invalid, 10_000).is_err());
        }
    }

    #[test]
    fn alignment_retry_retains_initial_request_and_records_final_attempt() {
        let mut evidence = PeriodEvidence::new(true);
        evidence.select(100_000, 10_000).unwrap();
        // Same conversion used by the existing single fresh-client retry.
        let aligned = (512_u64 * 10_000_000).div_ceil(48_000) as i64;
        evidence.record_alignment_retry(aligned);
        assert_eq!(evidence.initial_request_hns, Some(10_000));
        assert_eq!(evidence.final_request_hns, Some(106_667));
        assert!(evidence.alignment_retry);
        // This evidence exists before Activate/Initialize, even if either fails.
        let json = serde_json::to_value(evidence).unwrap();
        assert_eq!(json["default_hns"], 100_000);
        assert_eq!(json["minimum_hns"], 10_000);
        assert_eq!(json["initial_request_hns"], 10_000);
        assert_eq!(json["final_request_hns"], 106_667);
        assert_eq!(json["alignment_retry"], true);
        assert_eq!(json["qualification"], "unqualified-request-only");
        assert!(json.get("actual_period_hns").is_none());
    }
}
