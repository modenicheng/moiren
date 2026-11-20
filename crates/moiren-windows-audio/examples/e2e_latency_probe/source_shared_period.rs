//! Exact owned-source experiment request; not queue residence or latency.
use anyhow::{Result, ensure};
use moiren_windows_audio::latency::shared_period::{
    SharedFormatInfo, SharedInitializationApi, SharedInitializationFailure,
    SharedInitializationStage, SharedPeriodCaps, SharedPeriodReport,
};

#[cfg(windows)]
#[path = "source_shared_period/native.rs"]
pub(super) mod native;

pub(super) enum ReadbackError {
    Native(i32),
    Descriptor { frames: u32, error: anyhow::Error },
}

pub(super) fn prepare_exact(
    requested: u32,
    format: SharedFormatInfo,
    report: &mut SharedPeriodReport,
    capabilities: impl FnOnce() -> std::result::Result<SharedPeriodCaps, i32>,
    initialize: impl FnOnce(u32) -> std::result::Result<(), i32>,
    current: impl FnOnce() -> std::result::Result<(u32, SharedFormatInfo), ReadbackError>,
) -> Result<()> {
    *report = SharedPeriodReport {
        mix_format: Some(format),
        client_format: Some(format),
        requested_period_frames: Some(requested),
        desired_period_ms: Some(f64::from(requested) * 1000.0 / f64::from(format.sample_rate)),
        ..Default::default()
    };
    ensure!(
        (64..=4096).contains(&requested),
        "source Shared period outside 64..4096"
    );
    let caps = capabilities().map_err(|hresult| {
        report.client3_failure = Some(SharedInitializationFailure {
            stage: SharedInitializationStage::Capabilities,
            hresult,
        });
        anyhow::anyhow!(
            "source Shared capabilities: HRESULT 0x{:08X}",
            hresult as u32
        )
    })?;
    report.caps = Some(caps);
    ensure!(
        caps.fundamental_frames > 0
            && caps.min_frames > 0
            && caps.min_frames <= caps.max_frames
            && requested >= caps.min_frames
            && requested <= caps.max_frames
            && requested.is_multiple_of(caps.fundamental_frames),
        "exact source Shared period {requested} outside native capabilities {caps:?}"
    );
    initialize(requested).map_err(|hresult| {
        report.client3_failure = Some(SharedInitializationFailure {
            stage: SharedInitializationStage::Initialization,
            hresult,
        });
        anyhow::anyhow!("source Shared initialize: HRESULT 0x{:08X}", hresult as u32)
    })?;
    report.initialization_api = Some(SharedInitializationApi::Client3);
    let (frames, actual_format) = current().map_err(|failure| match failure {
        ReadbackError::Native(hresult) => {
            report.actual_period_hresult = Some(hresult);
            anyhow::anyhow!(
                "source Shared period readback: HRESULT 0x{:08X}",
                hresult as u32
            )
        }
        ReadbackError::Descriptor { frames, error } => {
            // The COM call succeeded: do not relabel local descriptor rejection
            // as a native HRESULT or invent a usable client-rate period.
            report.engine_period_frames = Some(frames);
            error.context("source Shared readback descriptor rejected after successful COM call")
        }
    })?;
    report.engine_period_frames = Some(frames);
    report.engine_sample_rate = Some(actual_format.sample_rate);
    if frames > 0 && actual_format.sample_rate > 0 {
        report.actual_period_frames = u32::try_from(
            (u64::from(frames) * u64::from(format.sample_rate))
                .div_ceil(u64::from(actual_format.sample_rate)),
        )
        .ok();
    }
    ensure!(
        actual_format == format,
        "source Shared engine format {actual_format:?} differs from exact source mix {format:?}"
    );
    ensure!(
        report.actual_period_frames == Some(requested) && frames > 0,
        "source Shared period readback differs: requested {requested}, actual {:?}",
        report.actual_period_frames
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn format() -> SharedFormatInfo {
        SharedFormatInfo {
            sample_rate: 48_000,
            channels: 2,
            container_bits: 32,
            valid_bits: Some(32),
            channel_mask: Some(3),
            floating_point: true,
        }
    }

    fn caps() -> SharedPeriodCaps {
        SharedPeriodCaps {
            default_frames: 480,
            fundamental_frames: 48,
            min_frames: 96,
            max_frames: 480,
        }
    }

    #[test]
    fn source_shared_period_exact_request_never_rounds_or_accepts_unknown_actual() {
        let init = Cell::new(0);
        let mut report = SharedPeriodReport::default();
        let result = prepare_exact(
            144,
            format(),
            &mut report,
            || Ok(caps()),
            |n| {
                assert_eq!(n, 144);
                init.set(init.get() + 1);
                Ok(())
            },
            || Ok((144, format())),
        );
        result.unwrap();
        assert_eq!(init.get(), 1);
        assert_eq!(report.requested_period_frames, Some(144));
        assert_eq!(report.actual_period_frames, Some(144));
        assert_eq!(
            report.initialization_api,
            Some(SharedInitializationApi::Client3)
        );
        assert_eq!(report.fallback_reason, None);

        let mut report = SharedPeriodReport::default();
        assert!(
            prepare_exact(
                145,
                format(),
                &mut report,
                || Ok(caps()),
                |_| panic!("must not round"),
                || panic!("uninitialized")
            )
            .is_err()
        );
        assert_eq!(report.requested_period_frames, Some(145));
        assert_eq!(report.caps, Some(caps()));
        assert_eq!(report.initialization_api, None);
        assert_eq!(report.actual_period_frames, None);

        for actual in [0, 96, 480] {
            let mut report = SharedPeriodReport::default();
            assert!(
                prepare_exact(
                    144,
                    format(),
                    &mut report,
                    || Ok(caps()),
                    |_| Ok(()),
                    || Ok((actual, format()))
                )
                .is_err()
            );
            assert_eq!(report.engine_period_frames, Some(actual));
            assert_eq!(report.actual_period_frames, (actual != 0).then_some(actual));
        }
    }

    #[test]
    fn source_shared_period_failures_retain_stage_without_retry_or_fabricated_actual() {
        for stage in 0..3 {
            let mut report = SharedPeriodReport::default();
            let mut init_calls = 0;
            let mut reads = 0;
            assert!(
                prepare_exact(
                    144,
                    format(),
                    &mut report,
                    || if stage == 0 { Err(-11) } else { Ok(caps()) },
                    |_| {
                        init_calls += 1;
                        if stage == 1 { Err(-22) } else { Ok(()) }
                    },
                    || {
                        reads += 1;
                        Err(ReadbackError::Native(-33))
                    },
                )
                .is_err()
            );
            assert_eq!(init_calls, usize::from(stage > 0));
            assert_eq!(reads, usize::from(stage > 1));
            assert_eq!(report.requested_period_frames, Some(144));
            assert_eq!(report.actual_period_frames, None);
            assert_eq!(report.fallback_reason, None);
            if stage == 2 {
                assert_eq!(report.actual_period_hresult, Some(-33));
                assert_eq!(
                    report.initialization_api,
                    Some(SharedInitializationApi::Client3)
                );
            } else {
                assert_eq!(
                    report.client3_failure.unwrap().hresult,
                    if stage == 0 { -11 } else { -22 }
                );
            }
        }
    }

    #[test]
    fn source_shared_period_local_descriptor_rejection_is_not_a_com_hresult() {
        let mut report = SharedPeriodReport::default();
        let error = prepare_exact(
            144,
            format(),
            &mut report,
            || Ok(caps()),
            |_| Ok(()),
            || {
                Err(ReadbackError::Descriptor {
                    frames: 144,
                    error: anyhow::anyhow!("unsupported subtype"),
                })
            },
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("unsupported subtype"));
        assert_eq!(report.engine_period_frames, Some(144));
        assert_eq!(report.actual_period_frames, None);
        assert_eq!(report.actual_period_hresult, None);
        assert_eq!(report.client3_failure, None);
    }

    #[test]
    fn source_shared_period_refuses_unmatched_engine_format() {
        for changed in 0..3 {
            let mut actual = format();
            match changed {
                0 => actual.sample_rate = 96_000,
                1 => actual.channels = 1,
                _ => actual.floating_point = false,
            }
            let mut report = SharedPeriodReport::default();
            assert!(
                prepare_exact(
                    144,
                    format(),
                    &mut report,
                    || Ok(caps()),
                    |_| Ok(()),
                    || Ok((144, actual))
                )
                .is_err()
            );
            assert_eq!(report.engine_sample_rate, Some(actual.sample_rate));
            assert_eq!(report.requested_period_frames, Some(144));
        }
    }
}
