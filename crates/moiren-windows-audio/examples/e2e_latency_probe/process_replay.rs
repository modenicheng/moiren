//! Offline, bounded queue replay. Imported histories never become native proof.
use anyhow::{Context, Result, ensure};
use moiren_windows_audio::{
    latency::transport::{DeliveryTimingSource, InputDeliveryTiming, TransportInputPolicy},
    process_loopback::{
        ProcessBridgeOptions, ProcessIdentity, ProcessReplayError, ProcessReplayReport,
        ProcessTraceArtifact, ProcessTraceOrigin,
    },
};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{collections::BTreeMap, fmt, fs::OpenOptions, io::Write, path::Path};

const HARD_INPUT_BYTES: u64 = 1_048_576;
const MAX_CANDIDATES: usize = 16;

struct CappedOutput(Vec<u8>);
impl Write for CappedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > HARD_INPUT_BYTES as usize - self.0.len() {
            return Err(std::io::Error::other("replay output exceeds 1 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn write_output(path: &Path, output: &ReplayOutput) -> Result<()> {
    // Serialize within a fixed byte limit before opening the output. Repeated
    // caller-supplied labels across 16 reports must not amplify disk usage or
    // leave an apparently complete partial evidence file on cap refusal.
    let mut bytes = CappedOutput(Vec::new());
    serde_json::to_writer_pretty(&mut bytes, output).context("serialize bounded replay output")?;
    bytes.write_all(b"\n")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create new replay output {}", path.display()))?;
    file.write_all(&bytes.0).context("write replay output")?;
    file.flush().context("flush replay output")
}
const OPTIONS: [&str; 7] = [
    "--trace",
    "--candidates",
    "--process-pid",
    "--process-created-100ns",
    "--owner-generation",
    "--max-input-bytes",
    "--output",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateRequest {
    schema_version: u32,
    #[serde(deserialize_with = "candidate_list")]
    candidates: Vec<ExplicitCandidate>,
}

// ProcessBridgeOptions intentionally supports defaulted application settings.
// A replay experiment must instead supply its policy, delivery, rate and demand
// explicitly, including an explicit null for unknown delivery.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExplicitCandidate {
    input: TransportInputPolicy,
    #[serde(deserialize_with = "required_delivery")]
    delivery: Option<ExplicitDelivery>,
    engine_sample_rate: u32,
    max_demand_frames: u32,
    age_limit_100ns: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExplicitDelivery {
    sample_rate: u32,
    max_packet_frames: u32,
    max_gap_frames: u32,
    source: DeliveryTimingSource,
}

fn required_delivery<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<ExplicitDelivery>, D::Error> {
    Option::deserialize(deserializer)
}

fn candidate_list<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<ExplicitCandidate>, D::Error> {
    struct BoundedCandidates;
    impl<'de> de::Visitor<'de> for BoundedCandidates {
        type Value = Vec<ExplicitCandidate>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("1..16 explicit Process bridge candidates")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut candidates = Vec::with_capacity(MAX_CANDIDATES);
            while candidates.len() < MAX_CANDIDATES {
                match sequence.next_element()? {
                    Some(candidate) => candidates.push(candidate),
                    None if candidates.is_empty() => {
                        return Err(de::Error::custom("at least one candidate is required"));
                    }
                    None => return Ok(candidates),
                }
            }
            // Do not allocate a seventeenth candidate or deserialize its shape.
            if sequence.next_element::<de::IgnoredAny>()?.is_some() {
                return Err(de::Error::custom("candidate cap exceeded (maximum 16)"));
            }
            Ok(candidates)
        }
    }
    deserializer.deserialize_seq(BoundedCandidates)
}

impl From<ExplicitCandidate> for ProcessBridgeOptions {
    fn from(candidate: ExplicitCandidate) -> Self {
        Self {
            input: candidate.input,
            delivery: candidate.delivery.map(|delivery| InputDeliveryTiming {
                sample_rate: delivery.sample_rate,
                max_packet_frames: delivery.max_packet_frames,
                max_gap_frames: delivery.max_gap_frames,
                source: delivery.source,
            }),
            engine_sample_rate: candidate.engine_sample_rate,
            max_demand_frames: candidate.max_demand_frames,
            age_limit_100ns: candidate.age_limit_100ns,
        }
    }
}

#[derive(Serialize)]
struct Refusal {
    stage: &'static str,
    code: &'static str,
    message: String,
}

impl Refusal {
    fn new(stage: &'static str, error: ProcessReplayError) -> Self {
        let code = match error {
            ProcessReplayError::InvalidIdentity => "invalid_identity",
            ProcessReplayError::IdentityMismatch => "identity_mismatch",
            ProcessReplayError::GenerationMismatch => "generation_mismatch",
            ProcessReplayError::IncompleteHistory => "incomplete_history",
            ProcessReplayError::InvalidHistory => "invalid_history",
            ProcessReplayError::ClockFault => "clock_fault",
            ProcessReplayError::AmbiguousOrder => "ambiguous_order",
            ProcessReplayError::InconsistentPublication => "inconsistent_publication",
            ProcessReplayError::UnsupportedShape => "unsupported_shape",
            ProcessReplayError::Candidate(_) => "candidate_rejected",
        };
        Self {
            stage,
            code,
            message: error.to_string(),
        }
    }
}

#[derive(Serialize)]
struct CandidateResult {
    index: usize,
    options: ProcessBridgeOptions,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<ProcessReplayReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Refusal>,
}

#[derive(Serialize)]
struct ReplayOutput {
    schema_version: u32,
    mode: &'static str,
    status: &'static str,
    qualified: bool,
    trace_origin: ProcessTraceOrigin,
    process: ProcessIdentity,
    owner_generation: u64,
    expected_process_pid: u32,
    expected_process_created_100ns: u64,
    expected_owner_generation: u64,
    candidates: Vec<CandidateResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Refusal>,
    limitations: [&'static str; 4],
}

pub(super) fn run(arguments: Vec<String>) -> Result<i32> {
    if arguments == ["--help"] {
        println!("{}", super::HELP);
        return Ok(0);
    }
    let (pairs, remainder) = arguments.as_chunks::<2>();
    ensure!(remainder.is_empty(), "options require separate values");
    let mut options = BTreeMap::new();
    for [name, value] in pairs {
        ensure!(OPTIONS.contains(&name.as_str()), "unknown option {name}");
        ensure!(
            options.insert(name.as_str(), value.as_str()).is_none(),
            "duplicate option {name}"
        );
    }
    for name in OPTIONS {
        ensure!(options.contains_key(name), "missing {name}");
    }
    let limit: u64 = options["--max-input-bytes"]
        .parse()
        .context("parse JSON input byte cap")?;
    ensure!(
        (1..=HARD_INPUT_BYTES).contains(&limit),
        "JSON input byte cap must be 1..1048576 per file"
    );
    let pid: u32 = options["--process-pid"]
        .parse()
        .context("parse Process PID")?;
    let creation_time_100ns: u64 = options["--process-created-100ns"]
        .parse()
        .context("parse Process creation time")?;
    let generation: u64 = options["--owner-generation"]
        .parse()
        .context("parse owner generation")?;
    ensure!(
        pid > 0 && creation_time_100ns > 0 && generation > 0,
        "Process PID, creation time and owner generation must be nonzero"
    );
    let request: CandidateRequest = super::read_json(options["--candidates"], limit)?;
    ensure!(
        request.schema_version == 1,
        "candidate schema_version must be 1"
    );
    // Both files are byte-bounded before serde allocation. The artifact also
    // caps histories; all imported identity labels remain caller-supplied.
    let artifact: ProcessTraceArtifact = super::read_json(options["--trace"], limit)?;
    let expected = ProcessIdentity {
        pid,
        creation_time_100ns,
        executable_name: String::new(),
    };
    let trace = artifact.trace(&expected, generation);
    let trace_error = trace.as_ref().err().copied();
    let mut candidates = Vec::with_capacity(request.candidates.len());
    for (index, candidate) in request.candidates.into_iter().enumerate() {
        let options = ProcessBridgeOptions::from(candidate);
        let result = match &trace {
            Ok(trace) => trace.replay(&expected, generation, options),
            Err(error) => Err(*error),
        };
        candidates.push(match result {
            Ok(report) => CandidateResult {
                index,
                options,
                status: "replayed",
                report: Some(report),
                error: None,
            },
            Err(error) => CandidateResult {
                index,
                options,
                status: "refused",
                report: None,
                error: Some(Refusal::new(
                    if trace_error.is_some() {
                        "trace_validation"
                    } else {
                        "candidate_replay"
                    },
                    error,
                )),
            },
        });
    }
    let output = ReplayOutput {
        schema_version: 1,
        mode: "offline_process_queue_replay",
        status: if trace_error.is_some() {
            "refused"
        } else {
            "completed"
        },
        qualified: false,
        trace_origin: ProcessTraceOrigin::CallerSupplied,
        process: artifact.process,
        owner_generation: artifact.owner_generation,
        expected_process_pid: pid,
        expected_process_created_100ns: creation_time_100ns,
        expected_owner_generation: generation,
        candidates,
        error: trace_error.map(|error| Refusal::new("trace_validation", error)),
        limitations: [
            "Imported identity labels and histories are caller-supplied, not authenticated native-owner provenance.",
            "Finite ordered queue mechanics only: no safety, sustained stability, physical latency or production admission qualification.",
            "The candidate assumes complete offered packets at observed order and a fresh empty queue; baseline accepted prefixes remain separate.",
            "Synthetic replay has no measured signal continuity or sample age; writer closure is not transport cleanup.",
        ],
    };
    write_output(Path::new(options["--output"]), &output)?;
    // A completed experiment sweep is not a quality gate. Candidate refusals
    // stay explicit, and only trace refusal changes a well-formed run to exit 1.
    Ok(i32::from(trace_error.is_some()))
}

#[cfg(test)]
#[path = "process_replay/tests.rs"]
mod tests;
