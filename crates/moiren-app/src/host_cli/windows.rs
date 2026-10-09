use super::*;
use crate::host::{
    HostConfig, SourceId,
    windows::{CaptureSelection, HostSession, SessionOptions, SessionStatus},
};
use anyhow::{Context, bail};
use moiren_windows_audio::process_loopback::inspect_process;
use std::{
    io::{BufRead, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

pub fn run_host(command: HostCommand) -> anyhow::Result<()> {
    match command {
        HostCommand::Help => {
            println!("{HOST_HELP}");
            Ok(())
        }
        HostCommand::List => emit(
            &json!({"type":"catalog","inputs":HostSession::capture_devices()?,"outputs":HostSession::output_devices()?,"processes":HostSession::processes()?}),
        ),
        HostCommand::Run {
            output,
            inputs,
            seconds,
        } => {
            let mut session = match start_host(output, inputs) {
                Ok(session) => session,
                Err(error) => {
                    emit(&json!({"type":"startup_failed","error":error.to_string()}))?;
                    return Err(error);
                }
            };
            let deadline = seconds
                .map(|s| {
                    Instant::now()
                        .checked_add(Duration::from_secs(s))
                        .context("duration exceeds the platform clock range")
                })
                .transpose()?;
            emit(&json!({"type":"started","snapshot":session.snapshot()}))?;
            let (tx, rx) = mpsc::sync_channel::<Result<String, String>>(32);
            // A blocked terminal reader cannot stall polling receipts/failures.
            // It owns no session resources and ends when EOF or receiver drop
            // releases a pending send. It need not be joined while stdin waits.
            std::thread::Builder::new()
                .name("moiren-host-stdin".into())
                .spawn(move || {
                    for line in std::io::stdin().lock().lines() {
                        if tx.send(line.map_err(|e| e.to_string())).is_err() {
                            break;
                        }
                    }
                })?;
            let mut eof = false;
            let mut next_status = Instant::now();
            let mut sequence = 0u64;
            let result = (|| -> anyhow::Result<()> {
                loop {
                    for event in session.poll() {
                        emit(&event_json(event))?;
                    }
                    let snapshot = session.snapshot();
                    if snapshot.status == SessionStatus::Failed {
                        break;
                    }
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        break;
                    }
                    if Instant::now() >= next_status {
                        emit(&json!({"type":"status","snapshot":snapshot}))?;
                        next_status = Instant::now() + Duration::from_secs(1);
                    }
                    let input = if eof {
                        std::thread::sleep(Duration::from_millis(10));
                        None
                    } else {
                        match rx.recv_timeout(Duration::from_millis(10)) {
                            Ok(line) => Some(line),
                            Err(mpsc::RecvTimeoutError::Timeout) => None,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                eof = true;
                                None
                            }
                        }
                    };
                    if eof && deadline.is_none() {
                        break;
                    }
                    if let Some(line) = input {
                        sequence += 1;
                        let reply = line
                            .map_err(HostCliError)
                            .and_then(|line| parse_control_command(&line));
                        match reply {
                            Ok(ControlCommand::Stop) => {
                                emit(
                                    &json!({"type":"ack","sequence":sequence,"ok":true,"stopping":true}),
                                )?;
                                break;
                            }
                            Ok(command) => match apply_command(&mut session, command) {
                                Ok(value) => emit(
                                    &json!({"type":"ack","sequence":sequence,"ok":true,"result":value}),
                                )?,
                                Err(error) => emit(
                                    &json!({"type":"ack","sequence":sequence,"ok":false,"error":error.to_string()}),
                                )?,
                            },
                            Err(error) => emit(
                                &json!({"type":"ack","sequence":sequence,"ok":false,"error":error.to_string()}),
                            )?,
                        }
                    }
                }
                Ok(())
            })();
            // Even stdout/parser errors join owners and reclaim terminal replies
            // before returning. CLI failures never abandon streaming workers.
            let report = session.stop();
            for event in &report.events {
                emit(event)?;
            }
            emit(&json!({"type":"final","report":report}))?;
            result?;
            if report.snapshot.status == SessionStatus::Failed {
                bail!("audio host failed: {:?}", report.snapshot.failure);
            }
            Ok(())
        }
    }
}
/// Selection preflight and native startup share one failure projection. Resolve
/// every process identity before starting captures, so a later invalid PID
/// cannot leave an earlier selection partially running.
fn start_host(output: String, inputs: Vec<InitialInput>) -> anyhow::Result<HostSession> {
    let selections = inputs
        .into_iter()
        .map(|input| match input {
            InitialInput::Physical(endpoint_id) => Ok(CaptureSelection::Physical { endpoint_id }),
            InitialInput::Process(pid) => Ok(CaptureSelection::Process {
                identity: inspect_process(pid)?,
            }),
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    HostSession::start(SessionOptions {
        output_endpoint_id: output,
        sources: selections,
        config: HostConfig::default(),
    })
    .map_err(Into::into)
}
fn emit(value: &Value) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}
fn selection(input: InputSelection) -> anyhow::Result<CaptureSelection> {
    Ok(match input {
        InputSelection::Physical { endpoint_id } => CaptureSelection::Physical { endpoint_id },
        InputSelection::Process {
            pid,
            creation_time_100ns,
        } => {
            let identity = inspect_process(pid)?;
            if identity.creation_time_100ns != creation_time_100ns {
                bail!("stale process identity: PID creation time changed");
            }
            CaptureSelection::Process { identity }
        }
    })
}
fn apply_command(session: &mut HostSession, command: ControlCommand) -> anyhow::Result<Value> {
    Ok(match command {
        ControlCommand::Status => json!(session.snapshot()),
        ControlCommand::Graph => graph_json(&session.graph_snapshot()?),
        ControlCommand::Devices => {
            json!({"inputs":HostSession::capture_devices()?,"outputs":HostSession::output_devices()?})
        }
        ControlCommand::Processes => json!(HostSession::processes()?),
        ControlCommand::Gain {
            source,
            value,
            ramp_frames,
        } => reply_json(session.set_gain(SourceId(source), value, ramp_frames)?),
        ControlCommand::Pan {
            source,
            value,
            ramp_frames,
        } => reply_json(session.set_pan(SourceId(source), value, ramp_frames)?),
        ControlCommand::Compressor {
            source,
            enabled,
            settings,
        } => {
            session.set_compressor(SourceId(source), enabled.then(|| settings.into()))?;
            json!({"staged":true})
        }
        ControlCommand::Add { selection: input } => {
            json!({"source":session.add_source(selection(input)?)?.0,"staged":true})
        }
        ControlCommand::StopSource { source } => {
            session.stop_source(SourceId(source))?;
            json!({"stopped":source})
        }
        ControlCommand::Remove { source } => {
            session.remove_source(SourceId(source))?;
            json!({"staged":true})
        }
        ControlCommand::Replace {
            source,
            selection: input,
        } => {
            session.replace_source(SourceId(source), selection(input)?)?;
            json!({"staged":true})
        }
        ControlCommand::Restart { source } => {
            session.restart_source(SourceId(source))?;
            json!({"staged":true})
        }
        ControlCommand::Enable { source, enabled } => {
            session.enable_source(SourceId(source), enabled)?;
            json!({"enabled":enabled})
        }
        ControlCommand::Publish => json!({"revision":session.publish()?}),
        ControlCommand::Cancel => {
            session.cancel_pending()?;
            json!({"cancel_requested":true})
        }
        ControlCommand::Stop => unreachable!("stop handled by the control loop"),
    })
}
