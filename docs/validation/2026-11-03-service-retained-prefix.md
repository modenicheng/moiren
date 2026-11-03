# Native service retained-prefix refusal

Date: 2026-11-03, Asia/Shanghai. Clean baseline: `0147db2`.
This advances the original S3 service acceptance without another UIA retry.
It does not establish desktop, waveform, sustained or physical acceptance.

## Scope and isolation

Local evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-service-retained-20261103-0105`.
Remote evidence: `D:\MoirenTest\20261103-service-retained-0105`.
One task `Moiren-ServiceRetained-20261103-0105` uses Limited interactive session1,
held app/source handles, worker150s/task240s and an application90s watchdog.
Native audio runs remotely only. Host work is compilation and pure/headless
regression, concurrent with the native task. No foreign process is stopped.

The application example uses public StartupDocuments, ManagedDriver,
ServiceRuntime and retained native evidence APIs. Default/help is inert;
explicit `--exercise` and absolute project/preferences/new-output paths are
required. It requests ApplyGraph, captures400ms from immediately before
enqueue, requests Stop and requires observed consumption by900ms. Actual
Closed joined cleanup precedes Evaluate. Only a real in-process opaque
candidate can authorize the separate replacement Apply. No imported JSON can.

The frozen graph is exact Process -> unity Bus -> physical DAWN PRO2 output,
48kHz/F32/Engine64, Shared336, render512/wake64/callback64 and Compatible
baseline. The proposed candidate remains capacity16384/target2048/jitter96;
policy remains4096initial-unserved/4096prime-discard/480output-frames/
2output-reads/100000100ns span. No threshold, default, period or buffer changes.
The independently frozen predecessor uses held source PID/creation identity,
app/native generation1, document0, NodeId(0), graph revision2, host revision1,
48k input/consumer and prepared whole-wake demand704.

The exact clean archive excludes all27 original dirty/cache files. Only an
evidence-only example and its explicit opt-in Cargo declaration overlay it.
The executed example is not committed as a production entry point:

```text
source archive SHA256:
8B10D7D780124264E5F6F4DE339BFBD25AC9399258A4F14A9A7400EEB4588951
example source SHA256:
F6C8EB59F4EC45A8B550766DFD5B565C6F08E48E2422F167E15D5EC1DE3F24C3
process_retained.exe: 5,193,216 bytes, SHA256:
37826D1BDF155F51C7C796C508664D25B8E9F45D4E166CA10798F239325FFED2
```

## Actual outcome

The task launches once at01:24:32 +08:00. Worker observation spans
01:24:33.985-01:24:44.636. Baseline Running confirms the exact requested/active
settings and native route. Stop enqueue enters/returns400ms; consumption is
observed401ms, within the unchanged900ms bound. Cleanup retains owner
generation1, Host Closed, no fault/cleanup error and the joined trace. App
intent generation2 after Stop is distinct from the retired owner generation1.
Final service exit and runtime join are confirmed.

Evaluate refuses with:

```text
Process packet/read envelopes overlap or tie; order is ambiguous
```

No candidate, replacement Apply, replacement owner or successful native
acceptance is claimed. App exits1 normally; worker/supervisor/task retain
failure1. The source completes its six-second288000-frame deterministic
schedule, then stops and joins normally after service exit, exit0. No forced
termination, remaining owner or cleanup error occurs. Protected OBS/QQMusic
creation identities, defaults, render volume/mute and running driver hash
remain unchanged.

The closed archive contains17packet and150read events, both writers finished,
zero overwritten events and no collection refusal. All8160 offered packet
frames are published; this is diagnostic accounting, not waveform continuity.
Offline inspection reproduces one ambiguous adjacent pair: packet7 encloses
111.9us; read73 encloses55.6-62.8us relative to packet7 begin. Their intersection
is7.2us; the packet envelope ends56.3us after the read begins. Observational
enclosures cannot establish the actual cross-writer linearization order.
This does not prove dropped audio, an audio defect or physical delay.

The unchanged isolated offline checker exits1 and retains
`numeric_consistency_audited=false`, with the same measurement refusal.
No measurement is fabricated from the archive. Remote closed-data analysis
produces `envelope-analysis/summary.json` and visually reviewed `envelopes.png`,
explicitly NOT AUDIO LATENCY. No PCM transfer or new audio attempt follows.

All34 compact files are length/SHA256-verified against both remote closed
files and archive entries, preserving relative directories. Archive45310bytes:
`F4224D974192380989692A3C749DD18AAE3E150EDD9874FB84B774EEF0E42030`.
An initial SSH stdout manifest stalls after8192bytes; its client is interrupted
only after native termination is independently confirmed. The existing ZIP
is not regenerated: a separate bounded verifier writes a new manifest, copied
with SCP and hash-checked. This is acquisition recovery, not a native rerun.

## Software and runner follow-up

Executed example: four pure tests, full Windows non-test strict Clippy and
release build pass. Worker helpers pass77 pure/static checks on PS5 and PS7,
including actual temporary-file atomic replacement. Genuine RED/GREEN covers
service-join validation and the explicit `.previous` backup required by PS5.
The unchanged checker passes27 tests with the original nine Rust fixtures.
Concurrent clean-source regression passes5SRC zero-refill,51analyzer and
20headless measured-UI tests. These are not native or physical passes.

Post-run review finds two runner-only false failures in the unreachable
replacement branch: it expects owner generation2 although Stop advanced
intent2 and next Apply would create owner3; it also expects live-window
metadata on a closed retained candidate. These do not affect this actual
Evaluate refusal. A separately preserved `runner-v2` corrects both, checks
policy directly on the opaque candidate assessment and keeps predecessor1
in its request. Two assertion REDs become six passing pure tests; cargo test
and full Windows non-test strict Clippy pass. This revision is not deployed
or run natively, and never replaces executed-source/binary provenance.

The existing production software-host retained-Apply test is strengthened in
both F32/F64: predecessor1 -> stopped intent2 -> replacement owner3 -> stopped
intent4, cleanup tied to owner3, original measured request preserved, and no
live-window metadata expected. This is regression coverage of existing
semantics, not a production audio fix.
Focused retained-Apply tests pass10; the final app library passes556/2ignored.
Strict app library/tests Clippy with gpui-test and isolated workspace formatting
pass. No new full-workspace test pass is claimed for this test-only slice.

### Publication Frontier Does Not Resolve The Refused Order

A bounded read-only review at `ab803d0` checks whether the existing consumer
frontiers can safely remove the original overlap refusal. Packet7's recorded
publication before this packet is2880frames and it accepts480frames. Read73
observes2880at both boundaries; read74observes3360. These acquired-tail samples
show that read73has not observed packet7's publication, not a total order for
the whole packet operation.

The producer checks free capacity before its later write/publication; that
check may precede the overlapping read even when publication follows it.
Moving the whole packet after that read during an alternate-capacity replay
would grant unproved free space. `QueueBoundary::published_frames` explicitly
describes consumer-observed publication, not a cross-writer operation ordering
receipt. Both live and retained paths use the same conservative validator.

The existing packet/read envelopes therefore remain insufficient for the
requested counterfactual replay. No ignored authoritative ordering fact,
reproducible product contradiction or safe local relaxation is established.
No product patch, new ordering machinery, changed timing contract or repeated
native capture follows. This closes one feasibility question; successful native
Evaluate/Apply remains unverified, not silently declared complete.

## Stop rule

Close the UIA timing dependency for this service-level experiment and retain
the conservative order refusal. Do not rerun an unchanged capture to obtain
a lucky non-overlapping trace, infer drop/latency from these intervals, relax
admission or add speculative real-time ordering machinery. The branch is not
fully native-accepted. Successful Evaluate/Apply, desktop acceptance,
lower-setting waveform quality, matched routes/clocks and calibrated physical
qualification remain open. First-packet work remains deferred; these evidence
limits do not block independent development or justify unchanged long soaks.
