# Drained VAD worker telemetry retention

## Demonstrated race

The [retired worker quality gate](2026-11-01-retired-worker-quality.md)
validates the raw worker snapshot retained by the provider. Its source review
identified an unresolved ordering problem in `Lifecycle::poll`: telemetry was
copied before an Acquire read of the worker's `drained` flag. A worker could
write its last counters and publish drained after that copy but before the
session-removal decision. Poll then removed the session while retaining the
older snapshot, so subsequent polls could not recover those final counters.
Joining the worker later did not restore a session already removed by poll.

Endpoint preparation had a second removal path: it discarded drained sessions
without copying their telemetry. Both paths must retain the same completed
worker's final writes before discarding the last tracked state.

## Repair and guarantee

The two control-side paths now share `EndpointRecord::reap_sessions`. For each
tracked session it reads `drained` with Acquire **before** copying telemetry
and removes the session only if that earlier read was true. The production
worker publishes drained with Release after its worker body, capture finish,
failure handling and PCM cleanup have completed.

When the Acquire observes true, preceding worker telemetry writes happen
before the ensuing relaxed counter loads, and that completed session's
snapshot is retained before removal. When it observes false, even if the
worker finishes during the copy, the session remains available for another
control-side read. No retry loop, blocking wait or stronger per-packet counter
ordering is added. The helper is called only on the existing control paths.
It does not move work onto an audio callback, change worker shutdown, resize
buffers or modify driver/native/RT timing.

This is a final drained-worker retention guarantee, not coherent live
telemetry publication. A running worker's individual relaxed counters may
still describe different instants. The output snapshot remains the existing
latest session view, not an aggregate/history across replacement owners.
Driver QUERY freshness, worker completion, admission identity and whole-audio
quality are separate concepts. The existing report schema and provenance
labels are unchanged, and `quality_complete` remains false.

## Evidence scope

Repository base is `275406c`. Host evidence root is
`D:\coding\moiren-evidence\20261101-drained-worker-retention`.
The hypothesis freeze pins eight source/plan inputs and the untouched
`gate.log` before test or implementation changes. Previous goal turn is
classified as progress: it committed the final worker quality check, without
claiming completion of the larger S1-S3 goal.

The RED regression executes real `SessionState`, telemetry and lifecycle poll
code with a native-control fake; it opens no devices. A test-only thread-local
hook runs immediately after the first telemetry copy. Two bounded channels
release a real worker thread to write tail faults and publish drained before
poll resumes. There are no scheduling sleeps or probability-based race loops.
The hook is absent from ordinary library builds and follows the existing
telemetry snapshot-hook testing pattern.

On the old implementation the test compiles and fails its final snapshot
assertion, exit101: poll retains all-zero counters rather than the worker's
silence64, aggregate drop3, missing-sequence3 and clock-regression1. Each raw
loss subtype, stale/silence/sequence counter and lifecycle total is included
in the full expected snapshot comparison. This is not a compilation failure.

With the repair the racing session survives the first copy and a subsequent
poll retains its full final snapshot before removal. Two additional focused
tests cover already-drained retention through a failed QUERY and later
Removed state, plus the shared preparation/poll reaper retaining running
sessions and preserving completed evidence across empty reaps. All three
focused tests pass. The tests do not establish actual native fault timing or
target audio qualification.

The final integrated checks use the corrected source and exit0:

```text
cargo test --locked -p moiren-windows-audio --all-features
    974 passed / 3 ignored across library, integration and doc-test targets
    library subset: 704 passed / 2 ignored
cargo test --locked -p moiren-windows-audio --all-features --example e2e_latency_probe --example shared_render_preflight --example vad_lifecycle --example vad_native --example vad_active_lifecycle
    737 passed across the five targets; latency probe subset: 635 passed
cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe
    672 passed / 4 ignored across all executed targets
cargo test --locked -p moiren-app --features gpui-test,vad-integration-probe --example vad_integration
    116 passed
cargo test --locked --manifest-path tools/rt-check/Cargo.toml
    218 passed
cargo clippy --locked -p moiren-windows-audio --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
cargo check --locked -p moiren-windows-audio --all-features --all-targets --target x86_64-unknown-linux-gnu
```

These are host Windows hardware-free tests and Linux cross-target compilation,
not Linux execution, full-workspace tests, Miri, remote CI or native audio
validation. The application/RT checks follow the downstream dependency closure
in `.github/scripts/detect-changes.sh`; unchanged engine/core/signal crates are
not claimed independently retested. The RED, focused GREEN and integrated
receipts retain argv, UTC times and exits. The RT build's package-cache waits
remain in its log; the same live session is polled, not relaunched.
`final-checks.json` pins the three checked Rust source hashes. A final audit
verifies the original input pins, unchanged dependencies, exact commit paths,
preserved `gate.log`, and evidence lengths/SHA-256. No prior acquisition or
software-failure artifact is overwritten.

## Remaining acceptance

The specific poll-before-drained loss demonstrated here is repaired in
software; actual native tail-fault detection and retention still need target
verification. Final-only lifecycle changes remain unattributed, and complete
live snapshot coherence, owner-history accounting, waveform/clock handling,
continuous quality, sustained lower-budget delivery, matched VAD/Process
comparison and calibrated physical acceptance remain open. Historical
reports and refusals remain unchanged. S1-S3 are not complete and no lower
default, stable sub-10-ms result, device floor or impossibility claim follows.

This slice performs no native replay, remote/audio process launch, physical
render, driver install, reboot, default-role/volume/mute/privacy write, push
or subagent work. The delivery remains a scoped local commit and preserves
unrelated `gate.log`.
