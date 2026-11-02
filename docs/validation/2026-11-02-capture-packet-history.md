# Opt-in bounded native capture packet history

Date: 2026-11-02, Asia/Shanghai. Parent revision: `b18248f`.
This is an S1 evidence-retention increment, not a new audio policy, latency
candidate, stability scan or qualification result.

## Missing evidence and implemented boundary

The [existing exact336 VAD/DAWN identity review](2026-11-02-vad-dawn-marker-review.md)
cannot inspect native packet chronology: its two Process recorders retained
aggregate metadata only. The original aggregate is not a packet timeline, and
the refused identities and `invalid_packet_metadata` verdict remain unchanged.
No old timestamps can be recovered by this change.

`PcmArtifactConfig::new(frames)` keeps the previous PCM-only behavior. Explicit
`with_packet_capacity(packets)` prepares at most 65,536 records before native
Start. The WASAPI transfer path copies the QPC already returned by `GetBuffer`,
not a new clock read. Each validated nonempty offer reaching the recorder keeps:

- One-based packet sequence and observed/stored PCM frame frontiers.
- Original offered frames and actual stored frames, including partial/zero
  storage after the separate PCM capacity is exhausted.
- Raw flags, device position and QPC in 100 ns units, without normalization.
- `Some(0)` for actual native zero; `None` for the old unstamped public API.

Records describe offers **before** `ReleaseBuffer`, not successful releases.
A later Release failure remains a capture failure and does not erase its offer.
Rejected oversized/invalid packets never enter the recorder. No inferred epoch,
process presentation clock, source origin or physical boundary is introduced.
Process device-position gap detection remains disabled.

Storage retains a fixed prefix, never a wrapping or growing history. Additional
offers increment explicit omitted-packet/frame counters. Counter saturation is
explicit and conservatively makes the history incomplete. `complete` means all
offered records reaching the recorder were retained, not that PCM storage,
timestamps, releases, waveform quality or the native run passed. The independent
PCM drop/overflow counters remain authoritative.

Append and overflow paths allocate/free nothing. Serialization happens only
after joining the native owner; existing recorder/report ownership transfers
also carry the new preallocated storage back to the control side. Defaults,
native periods, buffer targets/capacities, Engine scheduling, SRC, gates and
recovery behavior do not change.

## Explicit recorder interface

The existing `record-process` command adds only the optional
`--max-packets 1..65536`. Without it, aggregate metadata remains schema2,
the recorder report remains schema1, and no packet file is produced. With it,
aggregate metadata is schema3, the report is schema2 with the exact request,
and `capture.packet-history.jsonl` contains the raw records. Bounds and malformed
options are refused before reserving an output directory.

All files use create-new semantics. PCM, aggregate metadata and packet-history
writes are independently attempted, retaining the other evidence after one
write refusal. An existing packet file is never overwritten. Requested history
omission prevents `recording_complete=true`. Exit1 and `qualified=false` remain
unconditional for a valid recording/dry-run; neither is a quality certificate.

This raw JSONL is deliberately **not** the existing indexed analyzer's
`PacketTimeline` schema. No automatic conversion, invented epoch, acceptance
change or new analyzer result is part of this increment.

## Software verification

Core integration coverage checks opt-in compatibility, raw QPC/flags/positions,
native-zero versus unknown time, invalid/empty offers, independent storage
frontiers, complete packet history despite incomplete PCM, explicit omission,
capacity refusal, and lossless decomposition. A counting allocator checks
recording both before and after storage limits, including no deallocation.

The actual shared capture loop is exercised through fake COM, without opening
host audio. Its native `GetBuffer` supplies QPC1 then1668; the test observes
those exact values and retains the first offer after a failing Release. A
mutation restoring the unstamped production call fails with `None != Some(1)`;
restoring the stamped call passes. Existing physical/Process capacity enforcement
and safety-bound tests remain enabled.

CLI/persistence tests cover default-off behavior, raw serialization, collision
retention, exact dry-run request without native preparation, and zero/over-cap/
malformed/duplicate/missing option refusals. Fresh verification receipts follow
below; these are software tests, not native qualification.

```text
cargo test --locked --quiet --workspace
  final run: 2178 passed, 0 failed, 8 ignored across 71 summaries
cargo test --locked --quiet -p moiren-windows-audio --example e2e_latency_probe --features vad-probe
  664 passed, 0 failed, 1 ignored
cargo test --locked --quiet -p moiren-app --features gpui-test
  719 passed, 0 failed, 4 ignored
cargo test --locked --quiet -p moiren-windows-audio --test capture_packet_history
  final five-case suite: 5 passed, 0 failed
cargo test --locked --quiet -p moiren-windows-audio --example e2e_latency_probe --features vad-probe process_recording
  12 passed, 0 failed
cargo test --locked --quiet --manifest-path tools/rt-check/Cargo.toml
  239 passed, 0 failed
cargo test --locked --quiet -p moiren-windows-audio --test latency_marker --test latency_marker_rate --test latency_qualification
  51 passed, 0 failed across three suites
cargo test --locked --quiet -p moiren-windows-audio --lib zero_refill
  5 passed, 0 failed
cargo test --locked --quiet -p moiren-app --features gpui-test measured_ui_tests
  20 passed, 0 failed
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe -- -D warnings
cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings
cargo fmt --all -- --check
  all exit 0
cargo build --locked --release -p moiren-windows-audio --example e2e_latency_probe --features vad-probe
  exit 0
```

The initial full workspace run passes2177/8ignored with four integration cases;
the final2178/8 run includes the fifth case for complete packet history after
PCM exhaustion and decomposition. Its focused run also passes. The standalone
239-test RT harness is a separate surface, not these five integration tests.
The valid API RED, native mutation RED and restored GREEN receipts remain;
an earlier wrong allocation-helper compile error is not behavioral RED evidence.
Review is single-agent, not an independent review.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Receipts use `s1-packet-history-`. Checks run against the existing clean export
plus only this increment's six code/test files, excluding all 18 preserved dirty
paths. The preserved-path hash audit confirms all 18 remain unchanged.

## Remote deployment and non-audio check

The release probe is separately deployed, without overwriting the known-good
application/probe directory:

```text
D:\MoirenTest\20261102-packet-history-0910\e2e_latency_probe.exe
7464960 bytes
SHA256 AD527AE78D444A7E0F16C9D8480924B535D66CAE5E6D879975B6BFCF55E3D3A8
```

This binary comes from the clean export plus the six listed code/test files;
`source-manifest.json` records the exact pins. A subsequent doc-comment-only
correction says allocation is before Start, not on a different thread. No
executable change follows the build. The suffix0910 is a directory identifier;
the actual three-case check completes09:09:43.821 Asia/Shanghai.

One bounded remote task verifies enabled/legacy dry-run reports and invalid
zero capacity. Enabled request retains exact cap3/schema2; legacy retains
schema1 and no new field; both exit1 with `qualified=false`, no native capture
and only a report file. Invalid cap exits2 before creating a directory. No
source, recorder, application audio owner or device is opened. No owned probe
remains, and user OBS12648/start1793505454949 and QQMusic15096/start1793504036422
remain unchanged.

The initial inline SSH command is refused locally as too long, before remote
execution. Uploading the hash-pinned PowerShell runner and invoking that file
resolves the transport issue; no native run is retried. The downloaded remote
`result.json` is6262bytes, SHA256
`C9BAC249EAC802F1EAFDD18C3B23C43423D9A64CA5BC06604F4F4D0008E67F3E`,
length/hash-verified against the remote file. Compact receipts only are moved.

## Qualification remains open

This repairs future evidence acquisition only. Existing refused marker windows,
native delivery-bound violations and missing epoch/physical calibration remain
open, with first-packet investigation deferred by user direction. No repeated
native stability run is required merely to exercise this storage increment.
Remote application/device work stays on `LAPTOP-NP49RG6H`; host work is limited
to builds and hardware-free regression. No user process, driver, boot setting,
default, unrelated draft or cache directory is changed. No push or memory update.
