# Usable Process trace export and offline replay CLI

2026-10-29 (Asia/Shanghai), host-only S1/S3 continuation. No native audio
device, VM, driver build/install or deployment was exercised. The previous
turn made authoritative progress (five implementation slices and validation
receipts), not a hardware wait. The full S1-S3 objective remains active.

## Atomic receipts

Each implementation commit was independently pushed to
`origin/feat/minimal-io-latency` and verified against `git ls-remote` before
the next commit. No unrelated `gate.log` content was staged.

| Commit | Responsibility |
| --- | --- |
| `43ea5eb` | Versioned bounded trace artifact and strict scalar history import |
| `b24fe8b` | Real probe `replay-process` CLI and explicit candidate sweeps |
| `400def8` | Actual post-join Process collectors export their own histories |
| `8faa0e7` | Match shadow RT-harness artifact exports without unused warnings |
| `93143b9` | Cap replay output amplification before creating its file |

## Artifact and actual export

`ProcessTraceArtifact` schema1 carries PID/creation time, owner generation,
native/consumer rates, prepared read demand and both packet/read histories.
Schema, labels, rates, demand and histories are mandatory; unknown fields
are refused. Each deserialized history is bounded to256 events independently
of a size hint. Raw diagnostics may be incomplete, empty, overwritten or
unclosed; successful JSON decoding does not make them replayable.

Import always constructs `CallerSupplied`, never `RetainedNativeOwner`.
Expected PID, creation time and generation must be explicitly matched before
structural replay validation. Editable labels cannot authenticate provenance;
saved JSON cannot become trusted live measured-admission proof.

Both real probe paths, ordinary `live-process-loopback` and supervised owned
`--process-collection`, now retain `process-trace.json` after native workers
join and the actual Engine source/renderer is destroyed. The observer reads
the actual offered/accepted packet and consumer request histories; no fake
fill, timestamp, final read or closure marker is supplied by the collector.
The owned graph regression checks that `Graph::finish()` drops its real
DemandRenderer source before exporting the exact retained closed histories.

These probes create one immutable graph/owner per invocation. Artifact
generation1 is explicitly `single_probe_invocation`, not an EngineHost's
production generation or cross-run unique identity. PID plus creation time
still identifies the source. Consumer rate48 kHz and the Engine block bound
describe the actual probe's segmented source reads, not complete native-wake
admission; this export does not fix or certify that separate admission path.

The report/manifest includes the reserved path. Lexically normalized report,
manifest and input aliases are refused before creating the artifact directory.
Dry-run does not fabricate a collected trace. Files are new-only, never
overwrite existing evidence, and the serialized trace including newline is
limited to1 MiB. Actual write/flush errors stay hard failures, with an explicit
failed-export DTO retained alongside native reports. A write failure can still
leave a partial file; no atomic-on-IO-failure guarantee is claimed.

Exported-but-unreplayable history is useful raw evidence, not an operational
failure by itself. Reports separately expose writers-finished, replayable and
the refusal reason. `cleanup_verified=false` intentionally does not substitute
trace closure for native capture/render cleanup receipts.

## Actual offline command

The production probe executable accepts this command without discovering a
process, opening an audio endpoint, loading a driver or changing settings:

```powershell
e2e_latency_probe.exe replay-process `
  --trace C:\MoirenEvidence\run\process-trace.json `
  --candidates C:\MoirenEvidence\candidates.json `
  --process-pid 1234 --process-created-100ns 134000000000000000 `
  --owner-generation 1 --max-input-bytes 1048576 `
  --output C:\MoirenEvidence\replay-new.json
```

Those identity values are illustrative; use the actual exported source's
PID/creation time and run-scoped generation. Do not discover a current PID
and silently reinterpret an older trace as that process.

Candidate file shape (an example, not an admitted preset):

```json
{
  "schema_version": 1,
  "candidates": [
    {
      "input": {
        "policy": "target",
        "capacity_frames": 2048,
        "target_frames": 256,
        "jitter_frames": 0
      },
      "delivery": {
        "sample_rate": 48000,
        "max_packet_frames": 48,
        "max_gap_frames": 48,
        "source": "declared"
      },
      "engine_sample_rate": 48000,
      "max_demand_frames": 64
    }
  ]
}
```

Each candidate explicitly supplies input policy, delivery (including explicit
null when unknown), rate and complete prepared read demand. Target policy
requires its full target/capacity/jitter tuple. Age is optional; any requested
Process sample-age enforcement remains refused. Candidates can explicitly
request Compatible, but omitted intent never silently becomes Compatible.
The caller's delivery source is a label/assumption, not authenticated evidence.

CLI flags are mandatory and unique; unknowns, invalid identity integers, missing
values and unsupported candidate shapes are errors. Both input files are
byte-bounded before deserialization, with a mandatory user cap1..1,048,576.
At most16 explicit candidates are parsed. Every requested options tuple is
retained next to its queue result or typed refusal, without fallback.

The final JSON is also capped to1 MiB during serialization before create-new
opens the output. Repeated imported labels cannot amplify a bounded input into
an unbounded output file. Cap/serialization refusal creates no partial output;
subsequent real disk write/flush errors can leave partial evidence and return
an error. The bounded command is a diagnostic sweep, not a production tuner.

| Exit | Meaning |
| --- | --- |
| 0 | Candidate sweep completed, including poor queue results or even all candidates refused; inspect each result |
| 1 | Well-formed trace was refused (identity/generation, incomplete history, ambiguous order, etc.); structured refusal is saved |
| 2 | Malformed arguments/JSON, byte/count/output cap, missing input or IO error |

All reports remain `qualified=false` and `trace_origin=caller_supplied`.
`completed` is not an audio-quality, safe-target or stability verdict. Actual
queue mechanics still assume complete offered packets at observed order,
fresh empty initial state and an open producer throughout the read window.
They do not prove signal continuity, source age, native/drop teardown timing,
counterfactual execution duration or future scheduling.

## Verification

Commands ran in `D:\coding\moiren\.worktrees\minimal-io-latency` with
`CARGO_TARGET_DIR=D:\coding\moiren\target`. Counts overlap and are not additive.

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` | 521 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --lib replay::artifact` | 3 passed, including final public reexport path |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | Final278 passed |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe cli::process_replay` | Final12 passed |
| `cargo test --locked -p moiren-windows-audio --test clock_bridge --test packet_publication --test queue_inventory` | 31, 8 and12 passed |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | Final119 passed; includes18 shadow replay/artifact tests, not119 separate allocation checks |
| `cargo test --locked -p moiren-app --lib --features gpui-test -- --test-threads=4` | 456 passed, 3 ignored |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` | Exit0, including final output cap |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Exit0 |
| `cargo check --locked -p moiren-app --all-targets --features gpui-test` | Exit0 |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --all-targets --target x86_64-unknown-linux-gnu` | Final exit0, warning-free; cross-compilation only |
| `cargo build --locked -p moiren-windows-audio --example e2e_latency_probe --target x86_64-pc-windows-msvc --features vad-probe` | Final exit0, actual binary inspected/executed offline |
| `cargo fmt --all -- --check`; scoped RT rustfmt; `git diff --check` | Exit0 |

The actual executable, not just a parser unit test, was run with a structurally
valid synthetic offline packet/read fixture. It preserved an integer creation
time above JavaScript's exact-number range, produced exit0 with one refused
target and one explicitly requested Compatible replay, and retained
qualifiedfalse/caller-supplied output. A wrong expected generation produced
exit1 with `generation_mismatch`; an existing output produced exit2 and its
SHA-256 was unchanged. Invalid arguments produced exit2; help returned0.
After the final output-cap fix, the rebuilt binary reran the sweep and refused
an amplified output with exit2 before creating the output file.

Test-first and corrected attempts remain recorded:

- Artifact tests first failed to compile because the new artifact API was
  absent; implementation then passed all3. Missing/unknown fields, oversized
  histories, incomplete raw diagnostics, wrong identity/generation and schema
  refusal are covered without treating import as native proof.
- CLI stub first produced0 passed/9 failed; expanded implementation passed11.
  Its green log retains a transient parent missing-API compile blocker before
  the successful rerun. Final cap regression expands coverage to12.
- Export helper tests first failed to compile on absent snapshot/persist API,
  then passed. Hard-IO propagation separately failed on absent `export_into`,
  then4 export tests and the actual Graph-finish test pass in the final suite.
- Output amplification reproduced a true assertion failure: an under1 MiB
  source label appeared repeatedly in a greater1 MiB report. The capped writer
  fixes it; no file is created on that refusal.
- The first RT/Linux runs passed with an unused shadow artifact-reexport
  warning. Matching the harness's real public reexport removes it; final119
  tests and Linux cross-check are warning-free without widening production API.
- One manual PowerShell verification used an out-of-range argument-array
  index. It was not accepted as the existing-output proof. A corrected
  Stop-on-error run verifies exit2 and unchanged output hash; the final rebuilt
  binary also verifies its sweep and output cap.

Logs are in this worktree's `target/s1-s3-20261028/`: `s3-replay-artifact-*`,
`s3-replay-process-cli-*`, `s3-replay-export-*`, `s3-replay-output-cap-*` and
`s3-replay-cli-*`. Actual binary fixtures/results/logs are under
`replay-cli-binary/`; `verified-final.txt` records the final command checks.
Read-only independent review reports no remaining Important/Critical after
the output-cap correction. No remote CI, Miri or Linux test run is claimed.

## Remaining S1-S3 work

- This closes the missing usable probe CLI and actual trace-export surface,
  not a production desktop replay selector or sustained finite-window admission.
  Long runs overwrite256-event histories; genuinely overlapping envelopes are
  still refused. Useful trusted windows need exact starting state or a
  target-independent history model, not invented initial inventory.
- The application's declared/measured settings and real monitor replacement
  remain as previously implemented. Its measured admission is still per-wake,
  not relabeled as the new replay sweep or automatic profile selection.
- Native VAD timestamp/sample-age provenance and actual lower-period
  negotiation remain separate gates. Process positionless sample age remains
  unknown and its enforcement refused.
- Preserve the user's correctness/latency-sensitive split and configurable
  0..3000 ms extra progress grace; do not enlarge production buffers or run VM
  timing to bypass unavailable hardware. Matched physical route timing and
  sustained stable sub-10-ms output remain unqualified/hardware-deferred.
