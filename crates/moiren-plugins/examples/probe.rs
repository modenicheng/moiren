//! Exercise real, third-party plugins from the command line.
//!
//! Scans the given directories (default: the crate's standard directories),
//! then for every discovered descriptor: prints its metadata and parameter
//! table, instantiates it at 48 kHz with 256-frame blocks, renders a short
//! 440 Hz sine through it, reports per-channel output statistics, and
//! round-trips a state save/load. Scan failures are reported per file.
//!
//! Usage:
//!
//! ```text
//! cargo run -p moiren-plugins --example probe -- [directory...]
//! ```
//!
//! This crate has no crash isolation, so a misbehaving plugin can crash or
//! hang the probe; that is part of what the probe reveals.

use std::{io::Write as _, path::PathBuf, time::Instant};

use moiren_plugins::{
    PluginDescriptor, PluginInstance, instantiate,
    scan::{default_directories, scan_directories},
};

const SAMPLE_RATE: f64 = 48_000.0;
const MAX_BLOCK_FRAMES: u32 = 256;
const BLOCKS: usize = 20;
const TEST_FREQUENCY_HZ: f32 = 440.0;
const TEST_AMPLITUDE: f32 = 0.25;
const MAX_LISTED_PARAMETERS: usize = 16;

fn main() {
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let directories = if arguments.is_empty() {
        default_directories()
    } else {
        arguments
    };
    println!("Scanning {} director(y/ies):", directories.len());
    for directory in &directories {
        println!("  {}", directory.display());
    }
    let outcome = scan_directories(&directories);
    for (path, reason) in &outcome.failures {
        println!("[scan failure] {}: {reason}", path.display());
    }
    println!(
        "\n{} usable descriptor(s), {} scan failure(s)",
        outcome.descriptors.len(),
        outcome.failures.len()
    );
    for (index, descriptor) in outcome.descriptors.iter().enumerate() {
        println!();
        probe(index, descriptor);
    }
}

fn probe(index: usize, descriptor: &PluginDescriptor) {
    println!(
        "=== [{index}] {} {} ===",
        descriptor.format.label(),
        descriptor.name
    );
    println!("  id: {}", descriptor.id);
    println!(
        "  vendor: {}, version: {}",
        descriptor.vendor, descriptor.version
    );
    println!(
        "  scanned buses: {} in / {} out; synth: {}; editor: {}",
        descriptor.input_channels,
        descriptor.output_channels,
        descriptor.is_synth,
        descriptor.has_editor
    );
    println!("  path: {}", descriptor.path.display());

    // Flush before the plugin runs: if it hangs or crashes, this line is the
    // only clue about which plugin was at fault.
    print!("  instantiating at {SAMPLE_RATE} Hz, {MAX_BLOCK_FRAMES}-frame blocks... ");
    std::io::stdout().flush().expect("stdout flush");
    let mut instance = match instantiate(descriptor, SAMPLE_RATE, MAX_BLOCK_FRAMES) {
        Ok(instance) => instance,
        Err(error) => {
            println!("FAILED: {error}");
            return;
        }
    };
    println!("ok");
    let live = instance.descriptor();
    println!(
        "  live buses: {} in / {} out; latency: {} frames",
        live.input_channels,
        live.output_channels,
        instance.latency_frames()
    );

    let parameters = instance.parameters();
    println!("  parameters: {}", parameters.len());
    for parameter in parameters.iter().take(MAX_LISTED_PARAMETERS) {
        println!(
            "    #{} {} default {:.4} range [{:.4}, {:.4}]{}",
            parameter.id,
            parameter.name,
            parameter.default,
            parameter.min,
            parameter.max,
            if parameter.stepped { " stepped" } else { "" },
        );
    }
    if parameters.len() > MAX_LISTED_PARAMETERS {
        println!("    ... {} more", parameters.len() - MAX_LISTED_PARAMETERS);
    }

    render_sine(&mut *instance);
    round_trip_state(&mut *instance);
}

/// Renders sine blocks through the instance and reports output statistics.
fn render_sine(instance: &mut dyn PluginInstance) {
    let live = instance.descriptor();
    let input_channels = live.input_channels as usize;
    let output_channels = live.output_channels as usize;
    if output_channels == 0 {
        println!("  audio: skipped (no output channels)");
        return;
    }
    let frames = MAX_BLOCK_FRAMES as usize;
    let mut inputs = vec![vec![0.0f32; frames]; input_channels];
    let mut outputs = vec![vec![0.0f32; frames]; output_channels];
    let mut peaks = vec![0.0f32; output_channels];
    let mut sums = vec![0.0f64; output_channels];
    let mut non_finite = false;
    let mut first_block_time = None;
    for block in 0..BLOCKS {
        for samples in inputs.iter_mut() {
            for (frame, sample) in samples.iter_mut().enumerate() {
                let seconds = ((block * frames + frame) as f32) / SAMPLE_RATE as f32;
                *sample =
                    TEST_AMPLITUDE * (std::f32::consts::TAU * TEST_FREQUENCY_HZ * seconds).sin();
            }
        }
        for samples in &mut outputs {
            samples.fill(0.0);
        }
        let borrowed_inputs: Vec<&[f32]> = inputs.iter().map(|s| s.as_slice()).collect();
        let mut borrowed_outputs: Vec<&mut [f32]> =
            outputs.iter_mut().map(|s| s.as_mut_slice()).collect();
        let started = Instant::now();
        if let Err(error) = instance.process(&borrowed_inputs, &mut borrowed_outputs, &[]) {
            println!("  audio: process FAILED at block {block}: {error}");
            return;
        }
        if block == 0 {
            first_block_time = Some(started.elapsed());
        }
        for (channel, samples) in outputs.iter().enumerate() {
            for sample in samples {
                if !sample.is_finite() {
                    non_finite = true;
                    continue;
                }
                peaks[channel] = peaks[channel].max(sample.abs());
                sums[channel] += f64::from(*sample) * f64::from(*sample);
            }
        }
    }
    if let Some(elapsed) = first_block_time {
        println!("  audio: {BLOCKS} blocks rendered (first block {elapsed:?})");
    }
    let total_frames = (BLOCKS * frames) as f64;
    for (channel, sum) in sums.iter().enumerate() {
        println!(
            "  audio: out ch{} peak {:+.4} rms {:+.4}",
            channel,
            peaks[channel],
            (sum / total_frames).sqrt()
        );
    }
    if non_finite {
        println!("  audio: WARNING non-finite output samples");
    }
}

fn round_trip_state(instance: &mut dyn PluginInstance) {
    match instance.save_state() {
        Ok(Some(state)) => {
            println!("  state: {} bytes saved", state.len());
            match instance.load_state(&state) {
                Ok(()) => println!("  state: reload ok"),
                Err(error) => println!("  state: reload FAILED: {error}"),
            }
        }
        Ok(None) => println!("  state: plugin stores none"),
        Err(error) => println!("  state: save FAILED: {error}"),
    }
}
