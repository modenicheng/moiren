//! Continuous native-host scenario with live edits and independent source exits.
mod evidence;
mod generator;
use anyhow::{Result, bail, ensure};
use generator::OwnedChild;
use moiren_app::host::{
    HostConfig, HostEvent,
    windows::{CaptureSelection, HostSession, SessionOptions, SessionStatus, SourceStatus},
};
use moiren_core::protocol::ReplyCode;
use moiren_engine::processor::CompressorSettings;
use moiren_windows_audio::{process_loopback::inspect_process, render::RenderStatus};
use serde_json::json;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

fn process(child: &OwnedChild) -> Result<CaptureSelection> {
    Ok(CaptureSelection::Process {
        identity: inspect_process(child.pid())?,
    })
}

pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [mode, endpoint] = args.as_slice()
        && mode == "--owned-tone"
    {
        return generator::tone(endpoint);
    }
    let [
        out_key,
        output,
        input_key,
        input,
        generator_key,
        generator_output,
        seconds_key,
        seconds,
    ] = args.as_slice()
    else {
        bail!(
            "host_regression --output <48k-stereo-ID> --input <supported-capture-ID> --generator-output <isolated-48k-stereo-ID> --seconds <>=30>"
        );
    };
    ensure!(
        out_key == "--output"
            && input_key == "--input"
            && generator_key == "--generator-output"
            && seconds_key == "--seconds",
        "use the documented explicit selections"
    );
    let seconds = seconds.parse::<u64>()?;
    ensure!(
        seconds >= 30,
        "the source lifecycle scenario needs at least 30 seconds"
    );
    let mut child = OwnedChild::spawn(generator_output)?;
    // Deliberately no audio prewarm: cold child startup is part of the scenario.
    let physical_selection = CaptureSelection::Physical {
        endpoint_id: input.clone(),
    };
    let mut session = HostSession::start(SessionOptions {
        output_endpoint_id: output.clone(),
        sources: vec![physical_selection.clone(), process(&child)?],
        config: HostConfig::default(),
    })?;
    let graph = session.graph_snapshot()?;
    let sink = graph.sink;
    let physical = graph.sources[0].id;
    let mut application = graph.sources[1].id;
    let mut accepted = BTreeSet::new();
    let mut applied = BTreeSet::new();
    let mut terminal = BTreeSet::new();
    let mut racing = BTreeSet::new();
    let mut plans = BTreeSet::new();
    let mut checkpoints = Vec::new();
    let mut events = Vec::new();
    let started = Instant::now();
    let mut action = 0;
    let mut next_sample = Duration::ZERO;
    let mut last_frame = 0;
    let mut exited = false;
    let mut audible = false;
    while started.elapsed() < Duration::from_secs(seconds) {
        for event in session.poll() {
            match &event {
                HostEvent::Parameter(reply) => {
                    terminal.insert(reply.request_id);
                    if reply.code == ReplyCode::Applied {
                        applied.insert(reply.request_id);
                    }
                }
                HostEvent::PlanApplied { revision, .. } => {
                    plans.insert(*revision);
                }
                _ => {}
            }
            events.extend(evidence::events(vec![event]));
        }
        let snapshot = session.snapshot();
        ensure!(
            snapshot.status == SessionStatus::Running,
            "host failed: {:?}",
            snapshot.failure
        );
        ensure!(
            snapshot.runtime.timeline >= last_frame,
            "render timeline went backwards"
        );
        ensure!(
            session.graph_snapshot()?.sink == sink,
            "live output identity changed during publication"
        );
        last_frame = snapshot.runtime.timeline;
        audible |= snapshot.runtime.peak_amplitude > 0.0001;
        exited |= snapshot.sources.iter().any(|source| {
            source.id == application.0 && source.status == SourceStatus::TargetExited
        });
        let elapsed = started.elapsed();
        let due = [1, 3, 5, 6, 7, 10, 13, 15, 18, 21, 24, 26];
        if action < due.len() && elapsed >= Duration::from_secs(due[action]) {
            match action {
                0 => {
                    for reply in [
                        session.set_gain(physical, 0.02, 128)?,
                        session.set_gain(application, 0.03, 128)?,
                        session.set_pan(physical, -0.25, 128)?,
                        session.set_pan(application, 0.25, 128)?,
                    ] {
                        ensure!(
                            reply.code == ReplyCode::Accepted,
                            "parameter was not accepted: {:?}",
                            reply.code
                        );
                        accepted.insert(reply.request_id);
                    }
                }
                1 => {
                    session.set_compressor(application, Some(CompressorSettings::default()))?;
                    session.publish()?;
                }
                2 => session.enable_source(application, false)?,
                3 => session.enable_source(application, true)?,
                4 => child.stop()?,
                5 => {
                    ensure!(
                        exited,
                        "target exit was not reported while other owners stayed running"
                    );
                    ensure!(
                        session.restart_source(application).is_err(),
                        "process exit must require a fresh pinned selection"
                    );
                    child = OwnedChild::spawn(generator_output)?;
                    session.replace_source(application, process(&child)?)?;
                    session.publish()?;
                }
                6 => {
                    session.stop_source(physical)?;
                    let snapshot = session.snapshot();
                    ensure!(
                        snapshot
                            .sources
                            .iter()
                            .any(|s| s.id == physical.0 && !s.available),
                        "stopped input still available"
                    );
                    ensure!(
                        snapshot
                            .sources
                            .iter()
                            .any(|s| s.id == application.0 && s.status == SourceStatus::Running),
                        "stopping physical input ended its peer"
                    );
                }
                7 => {
                    session.restart_source(physical)?;
                    session.publish()?;
                }
                8 => {
                    session.remove_source(application)?;
                    session.publish()?;
                }
                9 => {
                    application = session.add_source(process(&child)?)?;
                    session.publish()?;
                }
                10 => {
                    session.set_compressor(application, Some(CompressorSettings::default()))?;
                    session.publish()?;
                    let reply = session.set_gain(application, 0.03, 128)?;
                    ensure!(
                        reply.code == ReplyCode::Accepted,
                        "live parameter during publication rejected"
                    );
                    accepted.insert(reply.request_id);
                    racing.insert(reply.request_id);
                }
                11 => {
                    ensure!(
                        session
                            .add_source(CaptureSelection::Physical {
                                endpoint_id: String::new()
                            })
                            .is_err(),
                        "invalid source accepted"
                    );
                    ensure!(
                        session.snapshot().status == SessionStatus::Running,
                        "new source failure stopped output"
                    );
                    // A provisional request against a retiring revision can be
                    // rejected. Retry against the confirmed plan and require its
                    // Applied receipt, rather than treating Accepted as final.
                    let reply = session.set_gain(application, 0.03, 128)?;
                    ensure!(
                        reply.code == ReplyCode::Accepted,
                        "confirmed-plan parameter rejected"
                    );
                    accepted.insert(reply.request_id);
                }
                _ => unreachable!(),
            }
            checkpoints.push(evidence::snapshot(
                &format!("action_{action}"),
                elapsed.as_secs_f64(),
                &session.snapshot(),
            ));
            action += 1;
        }
        if elapsed >= next_sample {
            checkpoints.push(evidence::snapshot(
                "sample",
                elapsed.as_secs_f64(),
                &snapshot,
            ));
            next_sample += Duration::from_secs(1);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = session.stop();
    for event in &report.events {
        if event["type"] == "parameter"
            && let Some(id) = event["reply"]["request_id"].as_u64()
        {
            terminal.insert(id);
            if event["reply"]["code"] == "Applied" {
                applied.insert(id);
            }
        }
    }
    let result = json!({"schema_version":1,"requested_seconds":seconds,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "scenario":"cold physical capture plus owned process, live gain/pan, compressor swap, target exit/replacement, stop/restart/remove/add",
        "parameter_accepted":accepted,"parameter_applied":applied,"parameter_terminal":terminal,"parameter_racing_publication":racing,"plan_revisions_applied":plans,"target_exit_observed":exited,
        "checkpoints":checkpoints,"events":events,"final":evidence::report(&report)});
    serde_json::to_writer_pretty(std::io::stdout().lock(), &result)?;
    println!();
    ensure!(
        action == 12 && exited && audible,
        "scenario did not complete or no output signal was observed"
    );
    ensure!(
        accepted.is_subset(&terminal)
            && accepted.difference(&racing).all(|id| applied.contains(id)),
        "provisional parameters lack terminal receipts or normal parameters lack Applied receipts"
    );
    ensure!(
        plans.len() >= 6,
        "live graph publications were not confirmed"
    );
    let render = report
        .render
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing native report"))?;
    ensure!(
        render.status == RenderStatus::Stopped && render.stream_started && render.stop_succeeded,
        "native output did not stop cleanly"
    );
    ensure!(
        render.stats.processed_frames > seconds.saturating_mul(48_000) * 9 / 10,
        "continuous timeline did not advance for the requested duration"
    );
    Ok(())
}
