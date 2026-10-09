//! Portable, pure command parsing and JSON projections. Native owners are only
//! opened by the Windows runner after parsing succeeds.
use crate::host::{GraphSnapshot, HostEvent};
use moiren_core::{graph::LogicalGraph, protocol::ControlReply};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::run_host;

pub const HOST_HELP: &str = "Moiren continuous audio host\nUsage:\n  moiren-app host --list\n  moiren-app host --output <ID> [--input <ID>]... [--process <PID>]... [--seconds <positive integer>]\nEach source starts at gain 0.05. Without --seconds, stop or stdin EOF ends the host.\nWith --seconds, EOF keeps rendering until the deadline; stop still ends it early.\nStdin accepts one JSON command per line (op: status, graph, gain, pan, compressor, add, stop_source, remove, replace, restart, enable, publish, cancel, devices, processes, stop).\nGraph/source/compressor edits are staged; publish makes them audible. Accepted gain/pan replies are provisional; match terminal parameter events by request_id.\nProcess add/replace requires explicit PID and creation_time_100ns; list processes for pinned identities. Initial --process pins identity before capture startup.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputSelection {
    Physical { endpoint_id: String },
    Process { pid: u32, creation_time_100ns: u64 },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialInput {
    Physical(String),
    Process(u32),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCommand {
    Help,
    List,
    Run {
        output: String,
        inputs: Vec<InitialInput>,
        seconds: Option<u64>,
    },
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct HostCliError(pub String);

pub fn parse_host_args(
    args: impl IntoIterator<Item = String>,
) -> Result<HostCommand, HostCliError> {
    let args: Vec<_> = args.into_iter().collect();
    if args == ["--help"] || args == ["-h"] {
        return Ok(HostCommand::Help);
    }
    if args == ["--list"] {
        return Ok(HostCommand::List);
    }
    let mut args = args.into_iter();
    let mut output = None;
    let mut inputs = Vec::new();
    let mut seconds = None;
    while let Some(key) = args.next() {
        if !["--output", "--input", "--process", "--seconds"].contains(&key.as_str()) {
            return Err(HostCliError(format!("unknown host argument: {key}")));
        }
        let value = args
            .next()
            .ok_or_else(|| HostCliError(format!("{key} requires a value")))?;
        match key.as_str() {
            "--output" if output.is_none() => {
                validate_endpoint(&value)?;
                output = Some(value);
            }
            "--input" => {
                validate_endpoint(&value)?;
                inputs.push(InitialInput::Physical(value));
            }
            "--process" => {
                let pid = value
                    .parse::<u32>()
                    .ok()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| HostCliError("--process requires a nonzero PID".into()))?;
                inputs.push(InitialInput::Process(pid));
            }
            "--seconds" if seconds.is_none() => {
                seconds = Some(
                    value
                        .parse::<u64>()
                        .ok()
                        .filter(|v| *v > 0)
                        .ok_or_else(|| {
                            HostCliError("--seconds requires a positive integer".into())
                        })?,
                );
            }
            _ => return Err(HostCliError(format!("duplicate argument: {key}"))),
        }
    }
    Ok(HostCommand::Run {
        output: output.ok_or_else(|| HostCliError("--output is required".into()))?,
        inputs,
        seconds,
    })
}
fn validate_endpoint(value: &str) -> Result<(), HostCliError> {
    if value.trim().is_empty() || value.contains('\0') {
        Err(HostCliError(
            "endpoint ID must be nonempty without NUL".into(),
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlCommand {
    Status,
    Graph,
    Devices,
    Processes,
    Publish,
    Cancel,
    Stop,
    Gain {
        source: u64,
        value: f64,
        #[serde(default)]
        ramp_frames: u32,
    },
    Pan {
        source: u64,
        value: f64,
        #[serde(default)]
        ramp_frames: u32,
    },
    Compressor {
        source: u64,
        enabled: bool,
        #[serde(default)]
        settings: CompressorConfig,
    },
    Add {
        selection: InputSelection,
    },
    StopSource {
        source: u64,
    },
    Remove {
        source: u64,
    },
    Replace {
        source: u64,
        selection: InputSelection,
    },
    Restart {
        source: u64,
    },
    Enable {
        source: u64,
        enabled: bool,
    },
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompressorConfig {
    pub input_gain_db: f64,
    pub threshold_db: f64,
    pub ratio: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub hold_ms: f64,
    pub knee_db: f64,
    pub makeup_gain_db: f64,
    pub output_gain_db: f64,
    pub mix: f64,
}
impl Default for CompressorConfig {
    fn default() -> Self {
        let s = moiren_engine::processor::CompressorSettings::default();
        Self {
            input_gain_db: s.input_gain_db,
            threshold_db: s.threshold_db,
            ratio: s.ratio,
            attack_ms: s.attack_ms,
            release_ms: s.release_ms,
            hold_ms: s.hold_ms,
            knee_db: s.knee_db,
            makeup_gain_db: s.makeup_gain_db,
            output_gain_db: s.output_gain_db,
            mix: s.mix,
        }
    }
}
impl From<CompressorConfig> for moiren_engine::processor::CompressorSettings {
    fn from(s: CompressorConfig) -> Self {
        Self {
            input_gain_db: s.input_gain_db,
            threshold_db: s.threshold_db,
            ratio: s.ratio,
            attack_ms: s.attack_ms,
            release_ms: s.release_ms,
            hold_ms: s.hold_ms,
            knee_db: s.knee_db,
            makeup_gain_db: s.makeup_gain_db,
            output_gain_db: s.output_gain_db,
            mix: s.mix,
        }
    }
}
pub fn parse_control_command(line: &str) -> Result<ControlCommand, HostCliError> {
    let value: Value = serde_json::from_str(line).map_err(|e| HostCliError(e.to_string()))?;
    let field_count = value.as_object().map_or(0, |object| object.len());
    let command: ControlCommand =
        serde_json::from_value(value).map_err(|e| HostCliError(e.to_string()))?;
    match &command {
        ControlCommand::Status
        | ControlCommand::Graph
        | ControlCommand::Devices
        | ControlCommand::Processes
        | ControlCommand::Publish
        | ControlCommand::Cancel
        | ControlCommand::Stop
            if field_count != 1 =>
        {
            return Err(HostCliError("command has unexpected fields".into()));
        }
        ControlCommand::Add { selection } | ControlCommand::Replace { selection, .. } => {
            match selection {
                InputSelection::Physical { endpoint_id } => validate_endpoint(endpoint_id)?,
                InputSelection::Process {
                    pid,
                    creation_time_100ns,
                } if *pid == 0 || *creation_time_100ns == 0 => {
                    return Err(HostCliError(
                        "process selection requires nonzero PID and creation time".into(),
                    ));
                }
                _ => {}
            }
        }
        ControlCommand::Gain { value, .. } if !(0.0..=16.0).contains(value) => {
            return Err(HostCliError("gain must be in [0, 16]".into()));
        }
        ControlCommand::Pan { value, .. } if !(-1.0..=1.0).contains(value) => {
            return Err(HostCliError("pan must be in [-1, 1]".into()));
        }
        _ => {}
    }
    Ok(command)
}
pub fn reply_json(reply: ControlReply) -> Value {
    json!({"request_id":reply.request_id,"plan_revision":reply.plan_revision,"timeline_epoch":reply.timeline_epoch,
        "code":format!("{:?}",reply.code),"effective_frame":reply.effective_frame})
}
pub fn event_json(event: HostEvent) -> Value {
    match event {
        HostEvent::Parameter(reply) => json!({"type":"parameter","reply":reply_json(reply)}),
        HostEvent::PlanApplied { revision, frame } => {
            json!({"type":"plan_applied","revision":revision,"frame":frame})
        }
        HostEvent::PlanRejected { revision, reason } => {
            json!({"type":"plan_rejected","revision":revision,"reason":reason.to_string()})
        }
    }
}
pub fn graph_json(snapshot: &GraphSnapshot) -> Value {
    json!({"desired":logical_graph_json(&snapshot.desired),"active":logical_graph_json(&snapshot.active),
        "bus":snapshot.bus.as_u64(),"sink":snapshot.sink.as_u64(),
        "active_revision":snapshot.plan.active_revision,"pending_revision":snapshot.plan.pending_revision,
        "compile":{"node_count":snapshot.plan.stats.node_count,"edge_count":snapshot.plan.stats.edge_count,
            "operation_count":snapshot.plan.stats.operation_count,"slot_count":snapshot.plan.stats.slot_count,"audio_bytes":snapshot.plan.stats.audio_bytes},
        "sources":snapshot.sources.iter().map(|s|json!({"id":s.id.0,"generation":s.generation,"node":s.node.as_u64(),
            "gain_node":s.gain_node.as_u64(),"pan_node":s.pan_node.as_u64(),"compressor_node":s.compressor_node.map(|n|n.as_u64()),
            "available":s.available,"gain":s.gain,"pan":s.pan})).collect::<Vec<_>>()})
}
fn logical_graph_json(graph: &LogicalGraph) -> Value {
    json!({"nodes":graph.nodes().iter().map(|n|json!({"id":n.id().as_u64(),"kind":format!("{:?}",n.kind()),"channels":n.channels(),
        "inputs":n.inputs().iter().map(|p|json!({"id":p.id().as_u64(),"channels":p.channels()})).collect::<Vec<_>>(),
        "outputs":n.outputs().iter().map(|p|json!({"id":p.id().as_u64(),"channels":p.channels()})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "edges":graph.edges().iter().map(|e|json!({"id":e.id().as_u64(),"source":e.src().as_u64(),"destination":e.dst().as_u64(),
            "source_port":e.src_port().as_u64(),"destination_port":e.dst_port().as_u64(),"gain":e.params().gain,"pan":e.params().pan,
            "mute":e.params().mute,"tap":format!("{:?}",e.params().tap)})).collect::<Vec<_>>()})
}
