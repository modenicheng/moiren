//! Failure-only metadata; never a fallback for ACTIVE client admission.
use super::{PropertyReadback, TopologyEvidence};
use serde::Serialize;

pub(crate) const MAX_ENDPOINTS: usize = 512;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Read<T> {
    Value {
        value: T,
    },
    Error {
        stage: &'static str,
        hresult: Option<i32>,
        message: String,
    },
    NotAttempted {
        stage: &'static str,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Identity {
    pub raw_instance: String,
    pub parent_instance: String,
    pub direction: String,
    pub exact_requested_identity: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Endpoint {
    pub index: u32,
    pub mmdevice_id: Read<String>,
    pub state: Read<u32>,
    pub identity: Read<Identity>,
    pub friendly_name: Read<PropertyReadback>,
    pub topology: Read<TopologyEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Failure {
    pub original_error: String,
    pub direction: String,
    pub requested_stable_id: String,
    pub requested_root: String,
    pub enumeration_mask: &'static str,
    pub elapsed_ms: u128,
    pub total_endpoints: Option<u32>,
    pub collection_error: Option<Read<()>>,
    pub truncated: bool,
    pub collection_completed: bool,
    pub endpoints: Vec<Endpoint>,
}

pub(crate) fn retain_failure<T>(
    operation: impl FnOnce() -> anyhow::Result<T>,
    pending: impl FnOnce(&anyhow::Error) -> Failure,
    mut publish: impl FnMut(Failure),
    collect: impl FnOnce(&anyhow::Error) -> Failure,
) -> anyhow::Result<T> {
    operation().inspect_err(|error| {
        // Persist the original failure before a diagnostic COM call can stall.
        publish(pending(error));
        publish(collect(error));
    })
}

pub(crate) fn budgeted<T>(
    stage: &'static str,
    within_budget: impl FnOnce() -> bool,
    collect: impl FnOnce() -> Read<T>,
) -> Read<T> {
    if within_budget() {
        collect()
    } else {
        Read::NotAttempted {
            stage,
            reason: "metadata work budget exhausted",
        }
    }
}

pub(crate) fn collect_endpoints<T>(
    count: u32,
    mut within_budget: impl FnMut() -> bool,
    mut collect: impl FnMut(u32) -> anyhow::Result<T>,
) -> (Vec<T>, bool, Option<anyhow::Error>) {
    let mut rows = Vec::new();
    let mut error = None;
    for index in 0..count.min(MAX_ENDPOINTS as u32) {
        if !within_budget() {
            break;
        }
        match collect(index) {
            Ok(row) => rows.push(row),
            Err(failure) => {
                error = Some(failure);
                break;
            }
        }
    }
    let truncated = rows.len() < count as usize;
    (rows, truncated, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(error: &anyhow::Error) -> Failure {
        Failure {
            original_error: format!("{error:#}"),
            direction: "Playback".into(),
            requested_stable_id: "stable".into(),
            requested_root: "root".into(),
            enumeration_mask: "ALL; diagnostic only",
            elapsed_ms: 0,
            total_endpoints: None,
            collection_error: Some(Read::Error {
                stage: "EnumAudioEndpoints",
                hresult: Some(-1),
                message: "metadata unavailable".into(),
            }),
            truncated: false,
            collection_completed: false,
            endpoints: Vec::new(),
        }
    }

    #[test]
    fn collection_failure_keeps_original_admission_error_and_explicit_metadata() {
        let mut evidence = None;
        let result = retain_failure::<()>(
            || Err(anyhow::anyhow!("original topology error").context("enumeration timed out")),
            failure,
            |value| evidence = Some(value),
            failure,
        );
        let original = "enumeration timed out: original topology error";
        assert_eq!(format!("{:#}", result.unwrap_err()), original);
        let evidence = evidence.unwrap();
        assert_eq!(evidence.original_error, original);
        assert!(evidence.collection_error.is_some());
        assert!(evidence.endpoints.is_empty());
    }

    #[test]
    fn successful_active_admission_does_not_collect_all_state_metadata() {
        let mut evidence = None;
        assert_eq!(
            retain_failure(
                || Ok(7),
                failure,
                |value| evidence = Some(value),
                |_| panic!("no diagnostic on success")
            )
            .unwrap(),
            7
        );
        assert!(evidence.is_none());
    }

    #[test]
    fn original_failure_is_published_even_when_collector_never_returns() {
        let published = std::cell::RefCell::new(Vec::new());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = retain_failure::<()>(
                || Err(anyhow::anyhow!("original")),
                failure,
                |value| published.borrow_mut().push(value),
                |_| {
                    assert_eq!(published.borrow()[0].original_error, "original");
                    panic!("simulated nonreturning collector");
                },
            );
        }));
        assert!(result.is_err());
        let values = published.into_inner();
        assert_eq!(values.len(), 1);
        assert!(!values[0].collection_completed);
    }

    #[test]
    fn mid_row_budget_expiry_never_schedules_the_next_metadata_stage() {
        assert!(matches!(
            budgeted("identity", || true, || Read::Value { value: 3 }),
            Read::Value { value: 3 }
        ));
        let value = budgeted::<u32>("GetState", || false, || panic!("no further COM read"));
        assert!(matches!(
            value,
            Read::NotAttempted {
                stage: "GetState",
                ..
            }
        ));
        let json = serde_json::to_value(value).unwrap();
        assert_eq!(json["status"], "not_attempted");
    }

    #[test]
    fn count_time_and_read_failures_explicitly_mark_incomplete_snapshot() {
        let (rows, truncated, error) = collect_endpoints(600, || true, Ok);
        assert_eq!(rows.len(), MAX_ENDPOINTS);
        assert!(truncated);
        assert!(error.is_none());

        let mut budget = 2;
        let (rows, truncated, error) = collect_endpoints(
            3,
            || {
                budget -= 1;
                budget >= 0
            },
            Ok,
        );
        assert_eq!(rows, [0, 1]);
        assert!(truncated);
        assert!(error.is_none());

        let (rows, truncated, error) = collect_endpoints(
            3,
            || true,
            |index| {
                if index == 1 {
                    anyhow::bail!("Item failed");
                }
                Ok(index)
            },
        );
        assert_eq!(rows, [0]);
        assert!(truncated);
        assert_eq!(error.unwrap().to_string(), "Item failed");

        let (rows, truncated, error) = collect_endpoints(2, || true, Ok);
        assert_eq!(rows, [0, 1]);
        assert!(!truncated);
        assert!(error.is_none());
    }
}
