# Minimal I/O + Bus latency design

Date: 2026-10-26. Authorized scope: implement the previously discussed stages
0 through 7 on an independent branch, including verification and development.
Base: `ec66e85117fbf668c2def334bdb33028afd454e9`.
Worktree: `D:\coding\moiren\.worktrees\minimal-io-latency`.
Branch: `feat/minimal-io-latency`.

## Outcome and qualification

The reference graph is Input -> Bus -> Output at 48 kHz, with no other
processors. The goal is stable measured end-to-end latency with p95 <= 10 ms.
Report p50, p95, p99 and max, rather than accepting one favorable observation.
The p50 exploration target is 8 ms. Capacity, target occupancy, observed age,
algorithm lookahead and external presentation latency are different quantities.

Both Process Loopback and VAD must be compared with the same output endpoint,
format, graph, signal and controlled load. Record source revision, binary hash,
Windows build, endpoint identity, actual period and measurement boundary.
Internal queue estimates cannot qualify source-to-physical-output latency.
A calibrated electrical/external reference is required for that claim.
Virtual-only routing and physical output routing need separate reports.

The final gate requires 30 minutes continuous PCM and ordinary controlled CPU
load without unexpected silence, drops, underruns, discontinuities or resets.
Two-hour runs and target-machine repetition qualify a product default.
VM timing, portable simulations and silent render tests provide supporting
evidence, but cannot replace physical latency and waveform evidence.
Startup, intentional Stop, hotplug and recovery windows are labeled separately;
they are not silently deleted from a steady-state failure report.

## Architecture

Retain one execution owner, render-master scheduling when a physical output
exists, off-thread preparation and control-side reclamation. Keep platform
transport and policy in moiren-windows-audio. Bus runs in the same execution
call; max_block_frames limits computation and is not per-node queue latency.

Extend the existing latency budget and telemetry modules instead of creating a
parallel policy stack. A prepared transport budget states input capacity,
prime/servo/low/high targets, maximum packet/gap assumption, interpolation
lookahead, jitter and recovery age. Native negotiated timing and empirically
observed virtual delivery timing carry distinct provenance. Unknown timing
never becomes a fabricated hardware period.

Compatible preserves existing behavior. Balanced/Low resolve validated
parameters. Expert values are accepted only when the preparation model fits
packet size, render demand and allocation bounds. Requested and active values
are reported separately. Reconfiguration uses owner replacement, not RT
allocation. No hidden automatic profile persistence or audio drop to meet a
displayed latency number.

ProcessLoopbackOptions receives the prepared bridge policy; all production,
professional, monitor and probe paths must propagate it. Output queue policy
is resolved separately from input timing, so a virtual input does not prevent
low-latency physical render where capabilities permit it.

VAD carrier capacity stops implying half-capacity steady residency. Explicit
playback and microphone targets are resolved from native packet timing,
Engine demand, FIR requirements and jitter. Preserve request quota, epoch
fencing, cancellation/rundown and microphone consumption freshness. Existing
MAX_PACKET_FRAMES is a protocol upper bound, not a required normal target.
No native stream before admission remains a valid idle state. Native shape
changes require a prepared/revalidated budget and safe session transition.

## Measurement

Use deterministic, low-amplitude, uniquely identifiable broadband markers and
normalized correlation. Reject silence, clipped/ambiguous matches, missing
markers and insufficient observation windows. Capture PCM into bounded
preallocated storage; perform expensive correlation and JSON output off RT.
Measure on a common sample clock where possible. QPC timestamps alone are
not DAC presentation; source submit time is not source sample generation.

Provide portable tests using known delays, gain, noise, drift/phase variations
and missing signals. Hardware probes record the exact boundary they observe.
An internal or WASAPI loopback measurement is labeled as such. Hardware
qualification cannot pass if physical calibration/evidence is missing.

## Stability and recovery

Slow servo corrects rate drift; high-water/age recovery bounds stale backlog.
Capacity provides burst space and is never intentionally primed in full.
Period counts used for reset decisions must not change a fixed physical
tolerance when block size changes. Packet phase, late completion, multi-period
wakes and callback demand deficits are covered by deterministic tests.
Every recovery is visible and fails a no-recovery qualification window.

A smaller buffer that produces silence or a reset is not a successful latency
improvement. Keep the established quality/freshness gates; do not excuse VM
failures or lower waveform coverage to obtain a pass.

## Naming

Retain the supported safe-recreation rename flow: preserve stable endpoint ID,
binding and project intent; drain owner, close gate, retire old PDO, await its
removal and recreate with the new name. The driver has CREATE name metadata,
not an in-place rename command. Expose pending/actual naming and errors
honestly. Verify held-client, failed creation and repeated edits. An in-place
rename ABI is unnecessary for the currently authorized product semantics.

## Delivery

Save the plan before production edits. Keep focused commits on this branch.
Preserve the original worktree and its staged/unstaged UI edits. Track each
stage and its evidence in the plan and validation report. Complete all host,
driver and hardware gates before declaring stage 7 or the overall goal done.
An unavailable device is a recorded missing gate, not a success.
