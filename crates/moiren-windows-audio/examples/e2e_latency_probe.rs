//! Offline evidence analysis and an explicitly unqualified live Process Loopback harness.
#[path = "e2e_latency_probe/cli.rs"]
mod cli;
#[path = "e2e_latency_probe/controlled_load.rs"]
mod controlled_load;
#[path = "e2e_latency_probe/indexed_pcm.rs"]
mod indexed_pcm;
#[path = "e2e_latency_probe/live.rs"]
mod live;
#[path = "e2e_latency_probe/measurement.rs"]
mod measurement;
#[cfg(any(windows, test))]
#[path = "e2e_latency_probe/measurement_sink.rs"]
mod measurement_sink;
#[path = "e2e_latency_probe/pcm.rs"]
mod pcm;
#[path = "e2e_latency_probe/probe_logging.rs"]
mod probe_logging;
#[path = "e2e_latency_probe/process_recording.rs"]
mod process_recording;

#[cfg(test)]
#[path = "e2e_latency_probe/tests.rs"]
mod tests;

fn main() {
    std::process::exit(run_cli(std::env::args().skip(1).collect()));
}

fn run_cli(arguments: Vec<String>) -> i32 {
    // Keep the handle through CLI teardown (including owner joins), then drop
    // it before main exits the process.
    let result = probe_logging::initialize().and_then(|_logger| cli::run(arguments));
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error:#}");
            2
        }
    }
}
