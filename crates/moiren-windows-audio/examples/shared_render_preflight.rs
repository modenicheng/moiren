use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(windows), allow(dead_code))]
enum RenderStatus {
    Stopped,
    #[cfg(test)]
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    endpoint_id: String,
    policy: PolicySelection,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(not(windows), allow(dead_code))]
struct CancellationEvidence {
    initialized: Option<bool>,
    stop_error: Option<String>,
    join_error: Option<String>,
    status: Option<RenderStatus>,
    stream_started: Option<bool>,
    stop_succeeded: Option<bool>,
    report_failure: Option<String>,
    cleanup_complete: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn cancellation_evidence(
    stop_error: Option<String>,
    join_error: Option<String>,
    report: Option<(RenderStatus, bool, bool, bool, Option<String>)>,
) -> CancellationEvidence {
    let (initialized, status, stream_started, stop_succeeded, report_failure) = report
        .map(
            |(status, initialized, stream_started, stop_succeeded, report_failure)| {
                (
                    Some(initialized),
                    Some(status),
                    Some(stream_started),
                    Some(stop_succeeded),
                    report_failure,
                )
            },
        )
        .unwrap_or((None, None, None, None, None));
    let cleanup_complete =
        stop_error.is_none() && join_error.is_none() && stream_started == Some(false);
    CancellationEvidence {
        initialized,
        stop_error,
        join_error,
        status,
        stream_started,
        stop_succeeded,
        report_failure,
        cleanup_complete,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PolicySelection {
    Process,
    Routing,
    Both,
}

enum Command {
    Help,
    #[cfg_attr(not(windows), allow(dead_code))]
    Run(Options),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut endpoint_id = None;
    let mut policy = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => return Ok(Command::Help),
            "--endpoint" => {
                if endpoint_id.is_some() {
                    return Err("--endpoint may be specified only once".into());
                }
                let value = args
                    .next()
                    .ok_or_else(|| "--endpoint requires an ID".to_owned())?;
                if value.trim().is_empty() || value.contains('\0') {
                    return Err("--endpoint requires a nonempty ID".into());
                }
                endpoint_id = Some(value);
            }
            "--policy" => {
                if policy.is_some() {
                    return Err("--policy may be specified only once".into());
                }
                policy = Some(match args.next().as_deref() {
                    Some("process") => PolicySelection::Process,
                    Some("routing") => PolicySelection::Routing,
                    Some("both") => PolicySelection::Both,
                    _ => return Err("--policy requires process, routing, or both".into()),
                });
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    let endpoint_id = endpoint_id.ok_or_else(|| "--endpoint is required".to_owned())?;
    let policy = policy.ok_or_else(|| "--policy is required".to_owned())?;
    Ok(Command::Run(Options {
        endpoint_id,
        policy,
    }))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn policies(
    selection: PolicySelection,
) -> Vec<(
    &'static str,
    moiren_windows_audio::latency::shared_period::SharedPeriodPolicy,
)> {
    use moiren_windows_audio::latency::shared_period::SharedPeriodPolicy;

    // Both paths now use Professional's device-default Shared period. Do not
    // silently retain the removed renderer's implicit five-millisecond request.
    let process = (
        "process",
        SharedPeriodPolicy {
            desired_period_ms: None,
        },
    );
    let routing = (
        "routing",
        SharedPeriodPolicy {
            desired_period_ms: None,
        },
    );
    match selection {
        PolicySelection::Process => vec![process],
        PolicySelection::Routing => vec![routing],
        PolicySelection::Both => vec![process, routing],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|value| (*value).to_owned()))
    }

    #[test]
    fn endpoint_and_period_policy_must_be_selected_explicitly() {
        assert!(matches!(
            parse(&["--endpoint", "virtual-output", "--policy", "process"]),
            Ok(Command::Run(Options { endpoint_id, policy: PolicySelection::Process }))
                if endpoint_id == "virtual-output"
        ));
        assert!(parse(&["virtual-output"]).is_err());
        assert!(parse(&[]).is_err());
        assert!(parse(&["--endpoint", "virtual-output"]).is_err());
    }

    #[test]
    fn arguments_reject_duplicates_empty_values_and_unknown_options() {
        assert!(parse(&["--endpoint", "one", "--endpoint", "two", "--policy", "both"]).is_err());
        assert!(parse(&["--endpoint", " ", "--policy", "both"]).is_err());
        assert!(
            parse(&[
                "--endpoint",
                "one",
                "--policy",
                "both",
                "--policy",
                "process"
            ])
            .is_err()
        );
        assert!(parse(&["--endpoint", "one", "--policy", "invalid"]).is_err());
        assert!(parse(&["--endpoint", "one", "--policy", "both", "--unexpected"]).is_err());
    }

    #[test]
    fn help_does_not_select_a_default_endpoint() {
        assert!(matches!(parse(&["--help"]), Ok(Command::Help)));
    }

    #[test]
    fn process_and_routing_use_the_professional_default_shared_period() {
        assert_eq!(
            policies(PolicySelection::Process)[0].1.desired_period_ms,
            None
        );
        assert_eq!(
            policies(PolicySelection::Routing)[0].1.desired_period_ms,
            None
        );
        let both = policies(PolicySelection::Both);
        assert_eq!(both[0].0, "process");
        assert_eq!(both[1].0, "routing");
        assert_eq!(both[0].1, policies(PolicySelection::Process)[0].1);
        assert_eq!(both[1].1, policies(PolicySelection::Routing)[0].1);
    }

    #[test]
    fn cleanup_requires_successful_stop_join_and_a_clean_pre_activation_report() {
        let report = Some((
            RenderStatus::Failed,
            true,
            false,
            false,
            Some("render preparation was cancelled".into()),
        ));
        let clean = cancellation_evidence(None, None, report.clone());
        assert!(clean.cleanup_complete);
        assert_eq!(clean.status, Some(RenderStatus::Failed));
        assert_eq!(clean.initialized, Some(true));
        assert_eq!(clean.stream_started, Some(false));
        assert_eq!(clean.stop_succeeded, Some(false));
        assert_eq!(
            clean.report_failure.as_deref(),
            Some("render preparation was cancelled")
        );
        assert!(
            !cancellation_evidence(Some("stop failed".into()), None, report.clone())
                .cleanup_complete
        );
        assert!(!cancellation_evidence(None, Some("join failed".into()), report).cleanup_complete);
        assert!(!cancellation_evidence(None, None, None).cleanup_complete);
        let missing_report = cancellation_evidence(None, None, None);
        assert_eq!(missing_report.initialized, None);
        assert!(!missing_report.cleanup_complete);
        let started = Some((RenderStatus::Stopped, true, true, true, None));
        assert!(!cancellation_evidence(None, None, started).cleanup_complete);
        let failed_before_initialize = Some((
            RenderStatus::Failed,
            false,
            false,
            false,
            Some("Initialize failed".into()),
        ));
        let not_initialized = cancellation_evidence(None, None, failed_before_initialize);
        assert!(not_initialized.cleanup_complete);
        assert_eq!(not_initialized.initialized, Some(false));
        let initialized_failure = Some((
            RenderStatus::Failed,
            true,
            false,
            false,
            Some("native initialization failure".into()),
        ));
        assert!(cancellation_evidence(None, None, initialized_failure).cleanup_complete);
    }
}

#[cfg(windows)]
fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    use moiren_engine::{
        audio_config::SampleFormat,
        node::{DeviceMode, DeviceOptions},
    };
    use moiren_windows_audio::{
        native::NativeStream,
        wasapi::{WasapiOptions, WasapiStream},
    };

    let mut results = Vec::new();
    for (name, shared_period) in policies(options.policy) {
        let render_options = WasapiOptions {
            endpoint_id: options.endpoint_id.clone(),
            sample_rate: 48_000,
            device: DeviceOptions {
                mode: DeviceMode::Shared,
                sample_format: SampleFormat::Float32,
                ..DeviceOptions::default()
            },
            input: false,
        };
        let result = match WasapiStream::open(render_options) {
            Ok(mut stream) => {
                let timing = stream.prepared_timing();
                let period = stream.shared_period();
                // Opening, stopping and releasing happen on this one COM owner.
                // Start is never called, and there is no worker to join.
                let stop_error = stream.stop().err().map(|error| error.to_string());
                let release_error = stream.release().err().map(|error| error.to_string());
                let stop_succeeded = stop_error.is_none();
                let cleanup = cancellation_evidence(
                    stop_error,
                    release_error,
                    Some((RenderStatus::Stopped, true, false, stop_succeeded, None)),
                );
                let cleanup_complete = cleanup.cleanup_complete;
                serde_json::json!({
                    "policy": name,
                    "initialized": cleanup.initialized,
                    "activated": false,
                    "requested_shared_period": shared_period,
                    "info": {"timing": timing, "shared_period": period},
                    "report": null,
                    "cleanup": cleanup,
                    "cleanup_complete": cleanup_complete,
                })
            }
            Err(failure) => {
                // A failed open has no public partial-initialization report.
                // Preserve unknown initialization/cleanup rather than success.
                let cleanup = cancellation_evidence(None, None, None);
                let cleanup_complete = cleanup.cleanup_complete;
                serde_json::json!({
                    "policy": name,
                    "initialized": cleanup.initialized,
                    "activated": false,
                    "requested_shared_period": shared_period,
                    "error": failure.to_string(),
                    "error_debug": format!("{failure:?}"),
                    "report": null,
                    "cleanup": cleanup,
                    "cleanup_complete": cleanup_complete,
                })
            }
        };
        results.push(result);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "endpoint_id": options.endpoint_id,
            "results": results,
            "physical_end_to_end_latency_ms": null,
            "meaning": "pre-activation WASAPI render initialization diagnostic only; no PCM or latency measurement"
        }))?
    );
    Ok(())
}

fn main() {
    let result = parse_args(std::env::args().skip(1));
    #[cfg(windows)]
    let result = result.and_then(|command| match command {
        Command::Help => {
            println!("shared_render_preflight --endpoint <explicit render endpoint ID> --policy process|routing|both");
            Ok(())
        }
        Command::Run(options) => run(options).map_err(|error| error.to_string()),
    });
    #[cfg(not(windows))]
    let result = result.and_then(|command| match command {
        Command::Help => {
            println!("shared_render_preflight --endpoint <explicit render endpoint ID> --policy process|routing|both (Windows only)");
            Ok(())
        }
        Command::Run(_) => Err("native render preflight requires Windows".into()),
    });
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
