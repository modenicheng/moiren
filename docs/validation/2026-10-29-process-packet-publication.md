# Process packet publication and bounded ordered replay

Host-only S1/S3 continuation, 2026-10-29 (Asia/Shanghai). No native audio
device was opened; no VM connection, deployment, driver build or installation
occurred. VM timing remains deferred, not a prerequisite for this software work.
The stable sub-10-ms hardware objective is unchanged and is not qualified here.

## Atomic implementation receipts

Each commit was pushed to `origin/feat/minimal-io-latency` and compared with
`git ls-remote` before the next commit. Unrelated `gate.log` was not staged.

| Commit | Responsibility |
| --- | --- |
| `6b7ec12` | Fixed-capacity producer packet publication history |
| `680ff8e` | Actual capture commit brackets, rejected offered supply and native projection |
| `ebc1fbd` | Actual consumer before/after read brackets |
| `c544fb1` | Unique-writer Drop closure, retained suffix and storage tests |
| `1531eff` | Ordered finite trace replay, origin contract, native pinned entry and regressions |
| `87a1e5e` | Scoped standalone RT-harness formatting; no semantic change |

## Actual observation boundaries

The prepared ingress records host QPC immediately before its free-space and
publication work and immediately after `fill_from_iter` commits the accepted
prefix to the SPSC ring. It records offered and accepted frames separately,
including partial and fully rejected packets. `ReleaseBuffer` failure does
not erase a committed prefix and remains a hard native failure.

Each packet retains cumulative offered/published frontiers, native flags,
local buffer generation and origin. A generation is that of the actual
published prefix; overflow advances the next generation afterward. Graph
output writes remain explicitly `GraphOutput` with unknown publication clocks,
not native Process supply. Zero offers and invalid calls create no event.

The sole consumer records QPC before acquiring its actual before-read inventory
and after acquiring its post-read inventory. Priming reads retain both brackets;
invalid/zero-size requests do not read clocks. Native ring removals and held
interpolation frames remain distinct from Engine-rate requested/output frames.
Neither boundary clock is a device/sample stamp or end-to-end delay.

Both histories retain at most 256 events using fixed scalar atomics, coherent
single-attempt version checks, checked totals and visible overwrite/saturation
diagnostics. Unknown and regressing clocks remain visible. No retry, lock,
logging, allocation or deallocation is added to audio publication. Snapshot
copies and replay allocations occur on the control side.

The storage-budget tests cover the actual published structures: packet history
20,544 bytes and queue history 34,864 bytes, excluding Arc allocation metadata.
Both are included in `ClockBridgeConfig::storage_bytes()` and real preparation
budgeting, not hidden in an unaccounted diagnostic allocation.

Only unique-writer Drop sets `writer_finished`. An observer-held Arc keeps
the final storage alive. Registry `running=false`, replacement or clear does
not assert closure; writer closure does not assert transport cleanup success.

## Bounded ordered replay contract

`ObservedProcessTrace::from_observations` validates caller-supplied histories
and identity labels, not their authenticity. Its report remains
`trace_origin=caller_supplied`, even when a public native DTO's labels are
edited to another valid process/generation. Only
`NativeObservationReadHandle::ordered_process_trace(node, process, generation)`
reads the private retained Session's immutable metadata and exact histories
before promoting origin to `retained_native_owner`. Later registry replacement
cannot redirect that handle. Neither origin is hardware qualification.

Construction requires closed, complete sequence-one packet/read histories:
1..256 events per writer, no overwrite, unknown/regressed clocks or saturation.
It reconstructs canonical histories and checks counts, prefix frontiers,
native-only origin, allowed flags, local generations, zero starting retirement,
inventory conservation and complete consumer demand. Work is bounded to
65,536 frames per event and 1,048,576 summed offered/requested frames.

Host envelopes must determine one nonoverlapping order. Cross-writer ties or
overlaps are refused rather than arbitrarily interleaved. Each baseline read's
before/after published frontier must equal accepted packets earlier in that
order. Trailing packets remain terminal inventory; no final read is invented.

For each candidate, the actual `ProcessBridgeOptions::clock_config()` and a
fresh empty capture bridge execute full offered packets and full read requests.
Candidate options must admit the recorded rate/demand; configured delivery
bounds must cover observed packet sizes and conservative publication gaps.
The actual prime, servo, interpolation, trim, overflow and reset mechanics run;
baseline retirement is never substituted for target-independent output demand.

Baseline losses and candidate results are separate. Startup unserved requests
are separate from later starvation/reset gaps. Reports retain drops, underruns,
discards, resets, terminal inventory and actual requested/output counts. Targets
256/512/1024 and capacities 2048/4096 are tested experiment candidates, not
automatically chosen safe presets or production-default changes.

The explicit `complete_packet_at_observed_order` assumption offers the entire
native packet, including frames the baseline rejected. Replay uses synthetic
silent workload and keeps its producer open across the request window. It does
not reproduce teardown/producer-Drop timing, sample values, source age, waveform
continuity, counterfactual execution duration or future scheduling. The queue
DTO deliberately has no synthetic signal/timestamp success fields. Status is
only `Replayed`, always `qualified=false`, never Safe or production admission.

## Software verification

Commands ran in `D:\coding\moiren\.worktrees\minimal-io-latency` with
`CARGO_TARGET_DIR=D:\coding\moiren\target`. Overlapping counts are not additive.

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` | Final 518 passed, 2 ignored |
| `cargo test --locked -p moiren-windows-audio --lib latency::` | 54 passed, including packet19 and queue23 |
| `cargo test --locked -p moiren-windows-audio --lib process_loopback::replay::` | Final expanded 15 passed |
| `cargo test --locked -p moiren-windows-audio --test clock_bridge --test packet_publication --test queue_inventory` | 31, 8 and 12 passed |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | 261 passed |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | Final 116 passed; includes 15 shadow replay tests, not 116 independent RT allocation tests |
| `cargo test --locked -p moiren-app --lib --features gpui-test -- --test-threads=4` | 456 passed, 3 ignored |
| `cargo test --locked -p moiren-app --target x86_64-pc-windows-msvc --test ui_acceptance --features gpui-test native_live:: -- --test-threads=1` | 3 rendered fixture tests passed; no actual native device |
| `cargo test --locked -p moiren-app --target x86_64-pc-windows-msvc --test cli` | 2 actual-binary help tests passed |
| `cargo clippy --locked -p moiren-windows-audio --all-targets --features vad-probe -- -D warnings` | Final exit 0 |
| `cargo clippy --locked -p moiren-app --all-targets --features gpui-test -- -D warnings` | Final exit 0 |
| `cargo check --locked -p moiren-app --all-targets --features gpui-test` | Final exit 0 |
| `cargo check --locked --manifest-path tools/rt-check/Cargo.toml --all-targets --target x86_64-unknown-linux-gnu` | Final exit 0, warning-free; cross-compilation only |
| `cargo fmt --all -- --check`; scoped RT-harness rustfmt; `git diff --check` | Exit 0 |

Replay coverage includes all four consumer rates (44.1/48/88.2/96 kHz), actual
SRC demand, repeated discontinuity/reset recovery, partial/zero acceptance,
trailing packets, startup versus post-start starvation, capacity independence,
overflow/trim, identity/generation mismatch, unadmitted options, malformed
frontiers, unknown timing, ambiguous order, closure, bounded work and origin.
The existing RT allocation/deallocation regressions remain in the full suites.

Test-first failures and corrected verification attempts are retained:

- Packet history: 1 passed/15 failed before implementation; then16 passed.
  Consumer envelopes: 7 passed/13 failed before implementation; then20 passed.
- Closure: packets15 passed/4 failed and queue19 passed/4 failed before the
  markers; then packet19 and queue23 passed. The final latency selection
  independently reruns all54 history/telemetry tests.
- Actual ingress and consumer after-clock omission mutations produced assertion
  failures; restoring the real brackets passes the integrations.
- Replay: initial4 failing tests, then4 passing; delivery-bound and reset-sum
  regressions next produced4 passed/2 failed, then all passed. Final expanded
  module is15 passing; public-origin/pinned-handle regression is also passing.
- Initial origin regression failed to compile on the missing origin field/API;
  its final result passes. Read-only review's Important about editable DTO
  provenance was fixed and closed, not silently reclassified as native proof.
- An initial packet RT build lacked the mock native tests' `stats` import.
  The later replay RT build exposed separate-crate private API/type mismatches,
  followed by module-path/private-constant mistakes while extending the shadow
  harness. Corrected harness compiles real bridge/replay modules without
  widening production visibility; final116 tests pass.
- The first Linux check exposed an unintended Windows-only DTO dependency in
  portable replay. Origin promotion now has no such dependency. An intermediate
  unused harness import was removed; final Linux cross-check is warning-free.
- The standalone RT-harness rustfmt check found one existing long Pump
  preparation call outside the root workspace's formatter scope. Formatting
  that call fixes the scoped check without a semantic change.

Logs are in this worktree's `target/s1-s3-20261028/`: `s3-packet-*`,
`s3-consumer-envelope-*`, `s3-history-closure-*`, `s3-ordered-*`. Final full
logs are `s3-ordered-audio-final.log`, `s3-ordered-rt-post-format.log`,
`s3-ordered-clippy-*-final.log`, `s3-ordered-check-app-final.log` and
`s3-ordered-linux-final.log`. Failed attempts are not overwritten by green logs.
No Ubuntu test execution, remote CI, Miri or hardware qualification is claimed.

## Remaining work and policy

- This API is bounded diagnostic replay, not the complete sustained feasible
  queue/admission workflow. Overwritten long runs and overlapping true native
  envelopes are explicitly refused. Support for trustworthy finite windows
  without invented initial state, and an actual app/CLI replay selection/report
  surface, remain software work. The existing measured CLI still uses per-wake
  admission; do not relabel it as replay or automatic target selection.
- Positionless Process packets still do not establish source sample age;
  unsupported age enforcement remains explicitly refused. Native VAD sample
  provenance and actual lower-period negotiation remain separate open gates.
- Preserve `--validation-mode correctness` versus `latency-sensitive`:
  correctness records timing/quality faults and continues; strict fails fast.
  Extra progress grace stays configurable 0..3000 ms (default 3000 only in
  correctness). Native/storage/identity/structural errors remain hard. No
  production buffer enlargement is used to hide the VM's scheduling defects.
- Physical matched-route comparison and stable p95/p99 below10 ms need suitable
  hardware. Do not retry VM audio timing, deploy or install on the host to get
  around that gate. The complete S1-S3 objective remains active, not complete.
