//! Offline evidence analysis and an explicitly unqualified live Process Loopback harness.
#[path = "e2e_latency_probe/cli.rs"]
mod cli;
#[path = "e2e_latency_probe/live.rs"]
mod live;
#[path = "e2e_latency_probe/measurement.rs"]
mod measurement;
#[cfg(any(windows, test))]
#[path = "e2e_latency_probe/measurement_sink.rs"]
mod measurement_sink;
#[path = "e2e_latency_probe/pcm.rs"]
mod pcm;

#[cfg(test)]
#[path = "e2e_latency_probe/tests.rs"]
mod tests;

fn main() {
    match cli::run(std::env::args().skip(1).collect()) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
    }
}
