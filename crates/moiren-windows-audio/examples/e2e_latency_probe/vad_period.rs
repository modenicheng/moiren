//! E2E native period request evidence, independent of actual negotiated cadence.

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
    pub(super) fn new() -> Self {
        Self {
            qualification: "unqualified-request-only",
            minimum_period: true,
            default_hns: None,
            minimum_hns: None,
            initial_request_hns: None,
            final_request_hns: None,
            alignment_retry: false,
        }
    }

    pub(super) fn record_alignment_retry(&mut self, request_hns: i64) {
        self.final_request_hns = Some(request_hns);
        self.alignment_retry = true;
    }
}

pub(super) fn select_period(
    default_hns: i64,
    minimum_hns: i64,
    evidence: Option<&mut PeriodEvidence>,
) -> anyhow::Result<i64> {
    if let Some(evidence) = evidence {
        // Retain both raw queries even when the selected minimum is invalid.
        evidence.default_hns = Some(default_hns);
        evidence.minimum_hns = Some(minimum_hns);
        ensure!(
            (1_000..=1_000_000).contains(&minimum_hns),
            "native minimum period out of bounds: {minimum_hns}"
        );
        evidence.initial_request_hns = Some(minimum_hns);
        evidence.final_request_hns = Some(minimum_hns);
        Ok(minimum_hns)
    } else {
        ensure!(
            (1_000..=1_000_000).contains(&default_hns),
            "native period out of bounds: {default_hns}"
        );
        Ok(default_hns)
    }
}

#[cfg(test)]
mod tests {
    use super::{PeriodEvidence, select_period};

    #[test]
    fn default_selection_preserves_legacy_range_without_adding_a_floor() {
        for default_hns in [
            1_000, 1_001, 9_999, 10_000, 50_000, 100_000, 999_999, 1_000_000,
        ] {
            assert_eq!(
                select_period(default_hns, 10_000, None).unwrap(),
                default_hns
            );
        }
    }

    #[test]
    fn default_selection_ignores_invalid_unused_minimum() {
        for minimum_hns in [i64::MIN, -1, 0, 999, 1_000_001, i64::MAX] {
            assert_eq!(select_period(5_000, minimum_hns, None).unwrap(), 5_000);
        }
    }

    #[test]
    fn default_selection_rejects_invalid_default_without_using_valid_minimum() {
        for default_hns in [i64::MIN, -1, 0, 999, 1_000_001, i64::MAX] {
            assert!(
                select_period(default_hns, 10_000, None).is_err(),
                "invalid default {default_hns}"
            );
        }
    }

    #[test]
    fn explicit_minimum_selection_preserves_range_and_records_request() {
        for minimum_hns in [1_000, 1_001, 10_001, 50_000, 100_000, 999_999, 1_000_000] {
            let mut evidence = PeriodEvidence::new();
            assert_eq!(
                select_period(100_000, minimum_hns, Some(&mut evidence)).unwrap(),
                minimum_hns
            );
            assert_eq!(evidence.default_hns, Some(100_000));
            assert_eq!(evidence.minimum_hns, Some(minimum_hns));
            assert_eq!(evidence.initial_request_hns, Some(minimum_hns));
            assert_eq!(evidence.final_request_hns, Some(minimum_hns));
            assert!(!evidence.alignment_retry);
        }
    }

    #[test]
    fn explicit_minimum_selection_ignores_invalid_unused_default() {
        for default_hns in [i64::MIN, -1, 0, 999, 1_000_001, i64::MAX] {
            let mut evidence = PeriodEvidence::new();
            assert_eq!(
                select_period(default_hns, 10_000, Some(&mut evidence)).unwrap(),
                10_000
            );
            assert_eq!(evidence.default_hns, Some(default_hns));
            assert_eq!(evidence.minimum_hns, Some(10_000));
            assert_eq!(evidence.initial_request_hns, Some(10_000));
            assert_eq!(evidence.final_request_hns, Some(10_000));
        }
    }

    #[test]
    fn invalid_selected_minimum_retains_raw_queries_without_fallback() {
        for minimum_hns in [i64::MIN, -1, 0, 999, 1_000_001, i64::MAX] {
            let mut evidence = PeriodEvidence::new();
            assert!(
                select_period(100_000, minimum_hns, Some(&mut evidence)).is_err(),
                "invalid minimum {minimum_hns}"
            );
            let json = serde_json::to_value(evidence).unwrap();
            assert_eq!(json["default_hns"], 100_000);
            assert_eq!(json["minimum_hns"], minimum_hns);
            assert!(json["initial_request_hns"].is_null());
            assert!(json["final_request_hns"].is_null());
            assert_eq!(json["alignment_retry"], false);
            assert_eq!(json["minimum_period"], true);
            assert_eq!(json["qualification"], "unqualified-request-only");
        }
    }

    #[test]
    fn new_evidence_is_copyable_and_serializes_only_unqualified_request_fields() {
        fn require_traits<T: Clone + Copy + std::fmt::Debug + serde::Serialize>() {}
        require_traits::<PeriodEvidence>();

        assert_eq!(
            serde_json::to_value(PeriodEvidence::new()).unwrap(),
            serde_json::json!({
                "qualification": "unqualified-request-only",
                "minimum_period": true,
                "default_hns": null,
                "minimum_hns": null,
                "initial_request_hns": null,
                "final_request_hns": null,
                "alignment_retry": false,
            })
        );
    }

    #[test]
    fn alignment_retry_serializes_final_attempt_without_claiming_actual_cadence() {
        let mut evidence = PeriodEvidence::new();
        assert_eq!(
            select_period(100_000, 10_000, Some(&mut evidence)).unwrap(),
            10_000
        );
        // The request receipt exists before a fresh client is activated or initialized.
        let aligned_hns = (512_u64 * 10_000_000).div_ceil(48_000) as i64;
        evidence.record_alignment_retry(aligned_hns);
        let json = serde_json::to_value(evidence).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "qualification": "unqualified-request-only",
                "minimum_period": true,
                "default_hns": 100_000,
                "minimum_hns": 10_000,
                "initial_request_hns": 10_000,
                "final_request_hns": 106_667,
                "alignment_retry": true,
            })
        );
        assert!(json.get("actual_period_hns").is_none());
        assert!(json.get("native_packet_frames").is_none());
    }
}
