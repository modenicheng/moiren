//! Control-side Process admission against the complete prepared render wake.
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::{
    latency::{NativeStreamTiming, PreparedRenderQueue},
    process_loopback::ProcessBridgeOptions,
};
use serde::Serialize;

/// Preparation evidence only; requested segment preflight is not admission.
/// Render geometry never supplies Process delivery timing or sample age.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ProcessAdmissionEvidence {
    #[serde(rename = "requested_segment_preflight_not_admitted")]
    pub(super) requested: ProcessBridgeOptions,
    pub(super) observed_native_timing: NativeStreamTiming,
    pub(super) observed_render_queue: PreparedRenderQueue,
    #[serde(rename = "admitted_complete_native_wake_fixed_48khz")]
    pub(super) admitted: Option<ProcessBridgeOptions>,
    pub(super) failure: Option<ProcessPreparationFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ProcessPreparationFailure {
    pub(super) stage: ProcessPreparationStage,
    pub(super) message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProcessPreparationStage {
    Admission,
    Capture,
}

pub(super) fn admit_process_bridge(
    options: ProcessBridgeOptions,
    timing: NativeStreamTiming,
    queue: PreparedRenderQueue,
) -> Result<ProcessBridgeOptions> {
    ensure!(
        timing.sample_rate == 48_000
            && timing.capacity_frames > 0
            && timing
                .period_frames
                .is_none_or(|period| { period > 0 && period <= timing.capacity_frames }),
        "Process probe requires valid actual native render timing at 48 kHz"
    );
    ensure!(
        queue.capacity_frames == timing.capacity_frames
            && queue.target_frames > 0
            && queue.target_frames <= timing.capacity_frames,
        "Process probe render queue must match actual native capacity and have a positive target within it"
    );
    // Empty-padding startup can demand the entire target in one wake. Engine
    // segmentation cannot reduce that reserve or establish Process delivery.
    options
        .for_consumer(48_000, queue.target_frames)
        .context("admitting Process input against the complete native render wake")
}

#[cfg(test)]
pub(super) fn prepare_capture_for_render<T>(
    options: ProcessBridgeOptions,
    timing: NativeStreamTiming,
    queue: PreparedRenderQueue,
    prepare: impl FnOnce(ProcessBridgeOptions) -> Result<T>,
) -> Result<(ProcessBridgeOptions, T)> {
    prepare_capture_for_render_recording(options, timing, queue, &mut None, prepare)
}

pub(super) fn prepare_capture_for_render_recording<T>(
    options: ProcessBridgeOptions,
    timing: NativeStreamTiming,
    queue: PreparedRenderQueue,
    evidence: &mut Option<ProcessAdmissionEvidence>,
    prepare: impl FnOnce(ProcessBridgeOptions) -> Result<T>,
) -> Result<(ProcessBridgeOptions, T)> {
    let record = evidence.insert(ProcessAdmissionEvidence {
        requested: options,
        observed_native_timing: timing,
        observed_render_queue: queue,
        admitted: None,
        failure: None,
    });
    let admitted = match admit_process_bridge(options, timing, queue) {
        Ok(admitted) => admitted,
        Err(error) => {
            record.failure = Some(ProcessPreparationFailure {
                stage: ProcessPreparationStage::Admission,
                message: format!("{error:#}"),
            });
            return Err(error);
        }
    };
    // Store admission before capture preparation can fail or enter native code.
    record.admitted = Some(admitted);
    match prepare(admitted) {
        Ok(capture) => Ok((admitted, capture)),
        Err(error) => {
            record.failure = Some(ProcessPreparationFailure {
                stage: ProcessPreparationStage::Capture,
                message: format!("{error:#}"),
            });
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_windows_audio::latency::{
        RenderQueuePolicy, prepare_render_queue,
        transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    };

    fn timing() -> NativeStreamTiming {
        NativeStreamTiming {
            sample_rate: 48_000,
            period_frames: Some(48),
            capacity_frames: 512,
        }
    }

    fn options(target_frames: u32) -> ProcessBridgeOptions {
        ProcessBridgeOptions {
            input: TransportInputPolicy::Target {
                capacity_frames: 2048,
                target_frames,
                jitter_frames: 0,
            },
            delivery: Some(InputDeliveryTiming {
                sample_rate: 48_000,
                max_packet_frames: 32,
                max_gap_frames: 32,
                source: DeliveryTimingSource::Declared,
            }),
            max_demand_frames: 64,
            ..Default::default()
        }
    }

    #[test]
    fn segment64_admission_does_not_authorize_native512_wake() {
        let options = options(128);
        assert!(
            options.prepare().is_ok(),
            "old segment-only preflight passes"
        );
        let queue = prepare_render_queue(timing(), RenderQueuePolicy::FullCapacity).unwrap();
        assert!(admit_process_bridge(options, timing(), queue).is_err());
    }

    #[test]
    fn full_capacity_and_target_use_the_actual_prepared_demand_not_capacity_for_both() {
        let native = timing();
        let full = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        let target = prepare_render_queue(
            native,
            RenderQueuePolicy::Target {
                frames: 64,
                wake_margin_frames: 8,
                callback_budget_frames: 8,
            },
        )
        .unwrap();
        assert_eq!(target.target_frames, 64);
        assert_eq!(target.capacity_frames, 512);
        let accepted = admit_process_bridge(options(128), native, target).unwrap();
        assert_eq!(accepted.max_demand_frames, 64);
        assert!(admit_process_bridge(options(128), native, full).is_err());
    }

    #[test]
    fn accepted_whole_wake_has_exact_48k_bound_and_input_minimum() {
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        // 512 complete demand + 2 interpolation frames + 32 declared gap.
        let requested = options(546);
        let admitted = admit_process_bridge(requested, native, queue).unwrap();
        assert_eq!(admitted.max_demand_frames, 512);
        assert_eq!(admitted.engine_sample_rate, 48_000);
        assert_eq!(admitted.input, requested.input);
        assert_eq!(admitted.delivery, requested.delivery);
        let prepared = admitted.prepare().unwrap();
        assert_eq!(prepared.prime_frames, 546);
        assert_eq!(prepared.modeled_minimum_frames, Some(546));
        assert!(admit_process_bridge(options(545), native, queue).is_err());
    }

    #[test]
    fn complete_wake_admission_is_independent_of_engine_segment_and_old_rate() {
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        let expected = admit_process_bridge(options(1024), native, queue).unwrap();
        for segment in [1, 32, 64, 128, 256, 512] {
            let segmented = ProcessBridgeOptions {
                engine_sample_rate: 96_000,
                max_demand_frames: segment,
                ..options(1024)
            };
            assert_eq!(
                admit_process_bridge(segmented, native, queue).unwrap(),
                expected
            );
        }
        assert_eq!(expected.max_demand_frames, 512);
        assert_eq!(expected.engine_sample_rate, 48_000);
    }

    #[test]
    fn invalid_native_geometry_or_mismatched_queue_is_refused() {
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        for invalid in [
            NativeStreamTiming {
                sample_rate: 0,
                ..native
            },
            NativeStreamTiming {
                sample_rate: 44_100,
                ..native
            },
            NativeStreamTiming {
                sample_rate: 96_000,
                ..native
            },
            NativeStreamTiming {
                capacity_frames: 0,
                ..native
            },
            NativeStreamTiming {
                period_frames: Some(0),
                ..native
            },
            NativeStreamTiming {
                period_frames: Some(513),
                ..native
            },
        ] {
            assert!(
                admit_process_bridge(options(1024), invalid, queue).is_err(),
                "{invalid:?}"
            );
        }
        for invalid in [
            PreparedRenderQueue {
                capacity_frames: 0,
                ..queue
            },
            PreparedRenderQueue {
                capacity_frames: 1024,
                ..queue
            },
            PreparedRenderQueue {
                target_frames: 0,
                ..queue
            },
            PreparedRenderQueue {
                target_frames: 513,
                ..queue
            },
        ] {
            assert!(
                admit_process_bridge(options(1024), native, invalid).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn unknown_native_period_is_not_invented_or_required_for_full_capacity() {
        let native = NativeStreamTiming {
            period_frames: None,
            ..timing()
        };
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        assert_eq!(queue.deadline_budget_frames, None);
        assert_eq!(
            admit_process_bridge(options(1024), native, queue)
                .unwrap()
                .max_demand_frames,
            512
        );
    }

    #[test]
    fn optional_ingress_policy_and_delivery_provenance_are_preserved_exactly() {
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        for limit in [None, Some(1), Some(123_456), Some(u64::MAX)] {
            for source in [
                DeliveryTimingSource::Declared,
                DeliveryTimingSource::Empirical,
            ] {
                let mut requested = options(1024);
                requested.max_ingress_elapsed_100ns = limit;
                requested.delivery.as_mut().unwrap().source = source;
                let admitted = admit_process_bridge(requested, native, queue).unwrap();
                assert_eq!(
                    admitted,
                    ProcessBridgeOptions {
                        max_demand_frames: 512,
                        ..requested
                    }
                );
            }
            let compatible = ProcessBridgeOptions {
                max_ingress_elapsed_100ns: limit,
                ..Default::default()
            };
            let admitted = admit_process_bridge(compatible, native, queue).unwrap();
            assert_eq!(
                admitted,
                ProcessBridgeOptions {
                    max_demand_frames: 512,
                    ..compatible
                }
            );
            assert!(
                admitted.delivery.is_none(),
                "render timing is not Process delivery"
            );
        }
    }

    #[test]
    fn unsupported_source_age_and_invalid_ingress_or_delivery_still_refuse() {
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        for requested in [
            ProcessBridgeOptions {
                age_limit_100ns: Some(1),
                ..options(1024)
            },
            ProcessBridgeOptions {
                age_limit_100ns: Some(1),
                ..Default::default()
            },
            ProcessBridgeOptions {
                max_ingress_elapsed_100ns: Some(0),
                ..options(1024)
            },
            ProcessBridgeOptions {
                delivery: None,
                ..options(1024)
            },
        ] {
            assert!(
                admit_process_bridge(requested, native, queue).is_err(),
                "{requested:?}"
            );
        }
        let mut bad_delivery = options(1024);
        bad_delivery.delivery.as_mut().unwrap().sample_rate = 96_000;
        assert!(admit_process_bridge(bad_delivery, native, queue).is_err());
    }

    #[test]
    fn capture_preparation_never_runs_after_whole_wake_or_geometry_refusal() {
        use std::cell::Cell;
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        for (requested, actual) in [
            (options(128), native),
            (
                options(1024),
                NativeStreamTiming {
                    sample_rate: 96_000,
                    ..native
                },
            ),
        ] {
            let called = Cell::new(false);
            let result = prepare_capture_for_render(requested, actual, queue, |_| {
                called.set(true);
                Ok(())
            });
            assert!(result.is_err());
            assert!(
                !called.get(),
                "capture must remain unprepared after refusal"
            );
        }
    }

    #[test]
    fn capture_preparation_receives_exact_admitted_bound_and_returns_capture() {
        use std::cell::Cell;
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        let requested = ProcessBridgeOptions {
            max_ingress_elapsed_100ns: Some(123_456),
            ..options(1024)
        };
        let expected = ProcessBridgeOptions {
            max_demand_frames: 512,
            ..requested
        };
        let called = Cell::new(0);
        let (admitted, capture) = prepare_capture_for_render(requested, native, queue, |options| {
            called.set(called.get() + 1);
            assert_eq!(options, expected);
            Ok(String::from("prepared-capture"))
        })
        .unwrap();
        assert_eq!(called.get(), 1);
        assert_eq!(admitted, expected);
        assert_eq!(capture, "prepared-capture");
    }

    #[test]
    fn capture_preparation_failure_keeps_its_original_typed_error() {
        #[derive(Debug)]
        struct CaptureFailure;
        impl std::fmt::Display for CaptureFailure {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("capture preparation refused")
            }
        }
        impl std::error::Error for CaptureFailure {}

        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        let error = prepare_capture_for_render::<()>(options(1024), native, queue, |_| {
            Err(CaptureFailure.into())
        })
        .unwrap_err();
        assert!(error.is::<CaptureFailure>());
        assert_eq!(error.to_string(), "capture preparation refused");
    }

    #[test]
    fn recording_refusal_serializes_raw_request_and_geometry_without_capture() {
        use std::cell::Cell;
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        for (requested, actual) in [
            (options(128), native),
            (
                options(1024),
                NativeStreamTiming {
                    sample_rate: 96_000,
                    ..native
                },
            ),
            (
                ProcessBridgeOptions {
                    age_limit_100ns: Some(1),
                    ..options(1024)
                },
                native,
            ),
        ] {
            let called = Cell::new(false);
            let mut evidence = None;
            let result = prepare_capture_for_render_recording(
                requested,
                actual,
                queue,
                &mut evidence,
                |_| {
                    called.set(true);
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert!(!called.get());
            let evidence = evidence.expect("rejected admission must remain reportable");
            assert_eq!(evidence.requested, requested);
            assert_eq!(evidence.observed_native_timing, actual);
            assert_eq!(evidence.observed_render_queue, queue);
            assert!(evidence.admitted.is_none());
            let value = serde_json::to_value(&evidence).unwrap();
            assert_eq!(
                value["requested_segment_preflight_not_admitted"],
                serde_json::to_value(requested).unwrap()
            );
            assert_eq!(
                value["observed_native_timing"],
                serde_json::to_value(actual).unwrap()
            );
            assert_eq!(
                value["observed_render_queue"],
                serde_json::to_value(queue).unwrap()
            );
            assert!(value["admitted_complete_native_wake_fixed_48khz"].is_null());
            assert_eq!(value["failure"]["stage"], "admission");
            assert!(!value["failure"]["message"].as_str().unwrap().is_empty());
            assert!(!value.to_string().contains("qualified"));
        }
    }

    #[test]
    fn recording_success_serializes_exact_admission_and_replaces_previous_failure() {
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        let mut evidence = None;
        assert!(
            prepare_capture_for_render_recording(
                options(128),
                native,
                queue,
                &mut evidence,
                |_| Ok(())
            )
            .is_err()
        );
        let requested = ProcessBridgeOptions {
            max_ingress_elapsed_100ns: Some((1_u64 << 53) + 1),
            ..options(1024)
        };
        let (admitted, capture) = prepare_capture_for_render_recording(
            requested,
            native,
            queue,
            &mut evidence,
            |options| {
                assert_eq!(options.max_demand_frames, 512);
                Ok(17)
            },
        )
        .unwrap();
        assert_eq!(capture, 17);
        let evidence = evidence.expect("accepted admission must remain reportable");
        assert_eq!(evidence.admitted, Some(admitted));
        assert!(evidence.failure.is_none());
        let value = serde_json::to_value(&evidence).unwrap();
        assert_eq!(
            value["requested_segment_preflight_not_admitted"]["max_demand_frames"],
            64
        );
        assert_eq!(
            value["admitted_complete_native_wake_fixed_48khz"]["max_demand_frames"],
            512
        );
        assert_eq!(
            value["admitted_complete_native_wake_fixed_48khz"]["max_ingress_elapsed_100ns"],
            (1_u64 << 53) + 1
        );
        assert!(value["failure"].is_null());
        assert_eq!(admitted.delivery, requested.delivery);
        assert_eq!(admitted.age_limit_100ns, requested.age_limit_100ns);
    }

    #[test]
    fn recording_capture_error_is_distinct_and_preserves_typed_error_and_admission() {
        #[derive(Debug)]
        struct CaptureFailure;
        impl std::fmt::Display for CaptureFailure {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("injected capture preparation failure")
            }
        }
        impl std::error::Error for CaptureFailure {}
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        let mut evidence = None;
        let error = prepare_capture_for_render_recording::<()>(
            options(1024),
            native,
            queue,
            &mut evidence,
            |_| Err(CaptureFailure.into()),
        )
        .unwrap_err();
        assert!(error.is::<CaptureFailure>());
        let evidence = evidence.expect("capture failure must not erase prior admission");
        assert_eq!(evidence.admitted.unwrap().max_demand_frames, 512);
        assert_eq!(
            evidence.failure.as_ref().unwrap().stage,
            ProcessPreparationStage::Capture
        );
        let value = serde_json::to_value(&evidence).unwrap();
        assert_eq!(value["failure"]["stage"], "capture");
        assert_eq!(
            value["failure"]["message"],
            "injected capture preparation failure"
        );
        assert_eq!(
            value["admitted_complete_native_wake_fixed_48khz"]["max_demand_frames"],
            512
        );
    }

    #[test]
    fn recording_admission_is_published_before_entering_capture_closure() {
        let native = timing();
        let queue = prepare_render_queue(native, RenderQueuePolicy::FullCapacity).unwrap();
        let mut evidence = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = prepare_capture_for_render_recording::<()>(
                options(1024),
                native,
                queue,
                &mut evidence,
                |_| panic!("capture entry sentinel"),
            );
        }));
        assert!(result.is_err());
        let evidence = evidence.expect("admission must be stored before invoking capture");
        assert_eq!(evidence.admitted.unwrap().max_demand_frames, 512);
        assert!(
            evidence.failure.is_none(),
            "unwinding is not a returned capture failure"
        );
    }
}
