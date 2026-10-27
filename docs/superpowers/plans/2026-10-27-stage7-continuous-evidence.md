# Stage 7 continuous evidence and remaining acceptance

This is a continuation of `2026-10-26-vad-live-qualification.md`, not a new
success definition. Worktree: `minimal-io-latency`; branch:
`feat/minimal-io-latency`; starting commit: `3872621`.

## Unchanged completion gate

Complete the remaining product acceptance and collect two independent
two-hour controlled-load runs of the same proposed default. Both require
calibrated physical `Input -> Bus -> Output` p95 plus uncertainty at most
10 ms, continuous waveform/marker evidence and zero quality failures. Keep
Compatible until those gates pass. VM results never qualify physical E2E.

## Current evidence and next actions

The previous goal turn made progress: reviewed software repairs, completed
combined host verification, and produced new VM evidence. The current target
smoke has four boundary xruns and two recovery/epoch resets, so it is not a
viable candidate for a two-hour qualification. No calibrated physical setup
exists. These limitations do not prevent the following software work.

1. Add real rendered acceptance for native packet/gap/age/padding, unknown
   values and lifecycle fencing, using explicit expected labels rather than
   regenerating the test oracle from the presentation function. Exercise
   scrolling at compact window size. Retain remaining qualified-evidence and
   service-to-native rename acceptance gaps rather than manufacturing DTO
   success or physical results.
2. Add a bounded native capture evidence tap. Preallocate stereo/mono Float32
   packet storage off RT, use a single producer/consumer with finite capacity,
   never wait or allocate/free on RT, and expose packet/position/sequence loss
   explicitly. Attach it through deferred preparation without changing
   default buffers or whole-run legacy artifacts. The native report retains
   producer allocations until control-side join, including errors/unwind.
3. Integrate a non-RT bounded sink and physical measurement capture with
   explicit byte/frame/duration caps. Preserve exact PCM, timestamp/packet
   metadata, observed continuity and failed artifacts. Native cancellation
   must not wait for a stalled sink; no detached writer. Source markers,
   measurement input, graph/output identities and calibration remain distinct.
4. Add supervised controlled-load collection and source-neutral manifests;
   verify disk and memory bounds, finite cleanup and failure propagation.
5. Repeat short native tests, preserving the target's 64 GB storage policy.
   Fix actual target quality failures before longer candidate tests. Then run
   the same-condition physical sweep, 30-minute candidates and two two-hour
   repetitions only when measurement and quality preconditions exist.

## Verification and delivery

- Use focused allocation/deallocation, FIFO, overflow, invalid-packet,
  continuity, error/unwind and UI acceptance regressions first.
- Independently review each responsibility-sized slice. Commit only its
  paths, push promptly, and preserve concurrent changes.
- Run combined build/check/tests/fmt/strict Clippy and full gpui acceptance
  after integration. Do not use narrow checks as physical evidence.
- Build on the host. Archive every completed/failed guest run with verified
  lengths and SHA-256 before deletion; keep one current probe, no build trees.

## Progress

- [x] Native live rendered UI acceptance slice (`b6fbbd2`, independent review
  PASS). Three 1024x720 headless layout/accessibility tests cover explicit
  labels, missing/stale generations, fresh replacement and vertical scroll.
  Physical-result states and full native/service rename acceptance stay open.
- [x] Bounded capture evidence tap and deferred native attachment (`e16ed25`,
  independent spec/quality review PASS; crate 694 passed/5 ignored, strict
  all-target Clippy and format PASS). This is not a sink or qualification.
- [ ] Bounded sink and actual physical measurement capture integration.
  Standalone native recorder plus bounded disk sink implemented in `ab48a9a`,
  independent spec/quality review PASS; example 52 tests and strict test Clippy
  pass. Host CABLE two-second diagnostic has zero tap loss but one native
  discontinuity (retained failure). Source/graph/load supervision, calibrated
  physical setup and true same-condition capture remain open, so this combined
  integration gate is intentionally unchecked.
- [ ] Supervised load and genuine continuous evidence collection.
- [ ] Stable target candidate and calibrated same-condition physical sweep.
- [ ] Thirty-minute and independent two-hour qualification repetitions.
- [ ] Remaining complete UI state matrix and final branch verification.
  Rendered held-name rename/refusal/fresh-replacement coverage for Playback
  and Microphone is committed in `4b0deba`, with independent spec/quality
  review PASS. It uses injected snapshots, not a real service/native lifecycle.
  Integrated workspace tests pass 1,621/8 ignored across 71 result suites;
  full feature-enabled app tests pass 489/4 ignored across 20 suites. Physical
  import/qualified states and native rename acceptance remain outstanding.

### Latest target diagnosis, 2026-10-27

`3289ded` corrects erased playback preroll by priming after gate admission.
Its one-second timer/event playback regressions have zero observed silence;
the five-second timer repeat still fails with a changed gate epoch.
`78ce2a1` / `1d30d78` add fixed, correlated user/kernel completion evidence
without changing freshness, buffering or RT logging. Independent review,
production-extracted diagnostic regressions, all seven driver host suites,
analyzed WDK/INF checks, full Windows-audio tests and the remaining workspace
tests pass. `10a33ae` adds an explicitly labeled microphone-only diagnostic
subset so early Playback failures do not prevent the independent investigation.

The matching signed kernel package is installed as `oem32.inf`; short target
tests remain failed/unqualified. The one-second microphone subset completes
but has three dropped writes and 3,840 silent frames total. Correlated kernel
records show actual ages 11.2613 / 10.6646 / 10.0750 ms against the unchanged
10 ms period, including two native first packets. A five-second microphone
subset later changes epoch and exits 1. First-write phase, missing queue
intervals and native callback rebase need further diagnosis; do not infer all
silence or historical failures from these three drops.

All failed/successful short-run payloads, ETL, manifests and signed-package
evidence are verified and retained on the host; trace files were capped at
1 MiB per session. The VM is cleaned to one current 400,896-byte probe with
30.103 GiB free, no live dynamic children or test processes. See the validation
report's bounded freshness section for source pins, exact records and hashes.
No stable candidate, physical calibration, rename acceptance, 30-minute or
two-hour gate is checked off by these diagnostic results.

### 2026-10-27 short target matrix and retention

The host-built native probe at source pin `f6f2718` produced all eight timer
case reports with unchanged defaults and zero reported native discontinuities
or microphone write drops. The event run failed in Microphone Mono Pcm16:
asynchronous `GetOverlappedResult(VAD PCM)` returned `0x8007051A` after 36,960
signal frames and two dropped writes. This narrows the current failure to PCM
completion rather than control gate admission; root cause is not established.
Timer observations contain silence too, so eight completed cases are not the
zero-silence physical/stability gate. The wrapper failed to retain process exit
codes (null); do not substitute report presence for a verified exit status.

Six guest files and their manifest (12,577 bytes total including the manifest)
are archived at `target/vad-target-archive/20261027-native-f6f2718-matrix`.
Every payload length/SHA-256 was checked on the host and rechecked on the guest
before deleting the run, temporary native probe and runner. Guest storage now
holds only `e2e_latency_probe-2c61452.exe` (3,359,744 bytes), no dynamic endpoints
or probe processes, and 30.12 GiB free. No driver/system files were removed.

### 2026-10-27 failure discriminator and bounded repeats

`f098fa9` adds admitted/opened and PCM-shape snapshots plus one best-effort
post-failure, pre-cleanup QUERY. Explicit epoch/state/shape fields and the
separate QUERY/progress errors preserve the original PCM failure. There is no
retry/reopen or weakened admission check. Seven added tests had genuine
RED/GREEN; all 33 native example tests, strict example/test Clippy and host
release build pass. Independent pinned spec/quality review passes.

The same probe then completed one-second event and timer matrices (eight cases
each, recorded exit 0). A five-second-per-case event repeat also exited 0, but
Microphone Float32 reported one mono and three stereo dropped writes, with
1,440 and 3,840 silent frames respectively. `0x8007051A` did not recur, so no
new failure snapshot was collected and its root cause is still unknown. The
probe's signal threshold is not the zero-quality-failure gate. No physical or
stable-candidate claim follows from these exits.

All nine repeat payloads and both manifests were verified on the host and
rechecked on the guest before deletion: 19,091 bytes in the one-second archive,
8,500 bytes in the five-second archive, including their manifests. Host roots:
`target/vad-target-archive/20261027-native-f098fa9-snapshot` and
`target/vad-target-archive/20261027-native-f098fa9-event5`. Temporary probes and
runners are removed; only the current 3,359,744-byte diagnostic probe remains,
with no dynamic endpoints/probe processes and 30.12 GiB free. Installed driver
INF/SYS/CAT SHA-256 values were refreshed and match the recorded running package.

A final bounded event diagnostic at 15 seconds per case also produced eight
case reports and exit 0, but PCM16 microphone dropped three mono writes and
one stereo write (3,360/1,440 silent frames). No native discontinuity or revision
mismatch was reported. This is another quality failure, not proof the
intermittent mismatch is fixed. Its three payloads plus manifest (8,508 bytes)
are retained in `target/vad-target-archive/20261027-native-f098fa9-event15`;
verified host/guest hashes preceded immediate guest cleanup back to the one
current diagnostic probe and 30.12 GiB free. No long qualification run follows
from these diagnostic exits.

### 2026-10-27 composable measurement owner

`6dcb9e8` extracts the independent recorder into an example-local session with
separate prepare, activation, native-Start acknowledgement, nonjoining stop,
native-first finish and retained partial reports. Review found an ignored
DeadlineReached state in the standalone adapter; `ce91b05` fixes it with a
pre-activation clock and earlier-bound termination. Independent re-review
passes spec/quality and clears the original finding. All 69 example tests,
including 13 owner and four adapter regressions plus the unchanged 15 sink
tests, strict example/test Clippy and format pass. This is a prerequisite,
not source/graph/load supervision or a hardware watchdog.

An explicit host CABLE two-second smoke at `ce91b05` retains 198 packets/
95,009 frames, 760,072 PCM bytes and 25,882 metadata bytes. Lengths, SHA-256,
packet offsets/sequences and PCM length agree. Native starts/stops cleanly;
tap/sink loss counters are zero, but one native discontinuity keeps the result
failed/unqualified. Before the deadline correction, an eight-byte-cap smoke
also retained zero-byte waveform/metadata and stopped native in 0.0201606 s.
These runs do not collect microphone input or change defaults/VM files.

### 2026-10-27 independent source and bounded target smoke

`45268bd` removes the measurement-analysis-to-source schedule dependency.
An explicit bounded source plan now owns marker positions on its own frame
clock; exact input bytes are retained. Native preroll is silent, and source
origin/counts commit only after successful ReleaseBuffer. The stream remains
alive on silence after the schedule is submitted. Independent pinned review
passes spec/quality; all 85 example tests, strict example/test Clippy, scoped
formatting and host release build pass. This is source submission evidence,
not a recorder clock mapping or concurrent measurement collection.

The three-second VM smoke submits all 96,000 planned source frames, with
480 native preroll frames, origin submission frame 480 and 144,960 total
native submitted frames. Native error is null and event timeouts are zero;
the provider has zero reported dropped/silent/stale packets and recovery/epoch
resets. Seven graph-boundary xruns still fail stability. Engine confirms Closed
and the dynamic export confirms Removed. Exit 1 correctly retains unqualified
status; no measurement PCM or calibration was collected.

The 3,570,176-byte host-built binary has SHA-256
`32e6540f17f06c3f6ce11e826770b4db9fbea84cde24beccca674c8d244d3519`.
Seven payloads plus manifest (518,194 bytes) are retained at
`target/vad-target-archive/20261027-source-45268bd-3s/source-45268bd-3s`.
Host length/hash checks, exact source-plan byte comparison and a second guest
manifest/file check preceded deletion of the run, temporary binary, plan and
runner. Guest again holds only the current 3,359,744-byte diagnostic probe,
zero test processes/dynamic endpoints and 30.11 GiB free. No driver/system
files were removed.

Next implement the concrete opt-in VAD source, production graph and independent
MeasurementSession collection in one run, including source health and broadcast
stop before joins. Controlled load, reference wiring, physical Input graph,
calibration and long qualification remain separate unfinished gates.

### 2026-10-27 concrete collection implementation and review repair

`def3a2c` adds the opt-in `--vad-collection` path in the existing live probe.
It composes a closed-gate independent `MeasurementSession`, the existing VAD
CREATE/native playback owner, the production `VirtualPlayback -> Bus -> pinned
physical Output` graph and the existing revision/gate admission checks. It
waits for actual recorder Running, keeps silent preroll separate from source
submission, polls source/provider/Engine/recorder health without relying on the
optional journal, retains negotiated recorder format and partial artifacts, and
broadcasts source and recorder stops before joins or sink drain. Collection
output stays `qualified=false`, `status=unqualified`; source submission is not
marker arrival or physical capture.

The independent review found two finalization defects. `43c572f` fixes both:
joined source event timeouts are reconciled after the last healthy poll and
before `collection_complete`; first raw provider/Engine failure evidence is
latched before a later QUERY can overwrite it, while later final observations
remain separate. RED/GREEN regressions, focused 14 tests, full example 109
tests, strict Windows example/test Clippy, scoped format and diff checks pass.
The fix review is spec/quality PASS.

The parent also re-ran the full workspace at `def3a2c`: build, all-target
check, format, strict Windows workspace Clippy, 1,621 tests/8 ignored across
71 suites, and feature GPUI 489 tests/4 ignored across 20 suites all pass.
The Linux strict Clippy baseline still fails on two unchanged `capture.rs`
retention fields; no broad CI-green claim is made until that separate issue is
handled. No native collection run has yet been made for this implementation.

### 2026-10-27 first combined collector smoke

The host release probe from `671da8d` was staged with a 48,000-frame source
plan and a 15-second collection config selecting the VM microphone. The opt-in
collector entered its real path but failed while preparing the independent
recorder: `choose_shared_period` returned `0x80070057`. The resulting report
kept `qualified=false`, `status=unqualified`, `collection_complete=false` and
`cleanup_complete=false`, with the primary recorder error plus the provider
cleanup observations. No source/graph/recorder outcome is inferred from this
failed preparation, and it is not a VAD latency or driver-quality result.

Ten failed-run payloads (17,993 bytes) are retained under
`target/vad-target-archive/20261027-collection-671da8d-15s/collection-671da8d-15s`.
Host lengths/SHA-256 and exact source/config bytes were checked; guest files
were rechecked against the host payload manifest before removing the run,
temporary release probe, plans, runner and manifest. Guest is back to one
3,359,744-byte current probe, no processes/dynamic endpoints and 30.11 GiB
free. The first runner's manifest had null SHA fields due to a PowerShell
serialization mistake; it is not used as evidence. Host-generated
`task7-collection-671da8d-payload-manifest.json` is the authoritative archive
checklist for this failed smoke. No driver/system files were removed.

### 2026-10-27 retention repair and capability evidence

`4850ccf` and `bc90dbd` add and push shared-period failure context, native
preparation report retention, and measurement/VAD collection propagation.
The repaired path was checked with 478 library tests plus 2 ignored, 109
probe tests, strict Windows-audio Clippy, all-target check, format and diff
checks.

The `bc90dbd` release probe was run once for 15 seconds on the target. It
exited 1 while preparing the independent recorder, retaining these raw
capabilities in `measurement.native_capture.shared_period`: sample rate
48,000; stereo; fundamental 35 frames; minimum 139; maximum/default 488;
requested period 5.0 ms; `period_selection_failure=invalid_capabilities`;
native stage `period_selection`, HRESULT `0x80070057`. The run remains
unqualified and is not VAD or physical latency evidence.

Five payloads are retained and verified at
`target/vad-target-archive/20261027-collection-bc90dbd-15s/`. Guest cleanup
removed the temporary run, plans, runner, logs and previous probe after hash
verification. Only the new 3,748,352-byte probe remains on the 64 GB guest;
free space is 30.12 GiB. No driver/system files were removed.

### 2026-10-27 period repair and bounded collection repeat

`c3fbc8d` removes the incorrect requirement that `default_frames` be a
fundamental multiple. A VM capability set of `default=488`, `fundamental=35`,
`min=139`, `max=488` is valid; 5 ms selected 245 frames and Client3 accepted
the stream. Focused shared-stream tests, Client3 tests, format, diff checks
and strict Clippy passed before the commit was pushed.

The `c3fbc8d` release probe was verified on the guest as 3,748,352 bytes with
SHA-256 `7490E742A9297982431AA3DBD28C24D50264D8C5E196CE6F33C4A3F6306B71B`.
The repeated 15-second collection progressed past recorder preparation and
recorded actual period 279 frames. It remains unqualified: VAD had 36
completion gaps, the virtual boundary had one xrun and 33 soft events, and
the independent physical capture had 74 gap events, 1,835 gap frames and two
discontinuities. Only 18,720 of 48,000 source frames were acknowledged.

All seven run payloads (1,327,509 bytes) were copied to and verified in
`target/vad-target-archive/20261027-collection-c3fbc8d-15s/`. Guest cleanup
left only the current probe and 30.11 GiB free. The next work item remains a
real supervised controlled-load contract; `--controlled-load-id` is still
metadata only and must not make a report qualify.

### 2026-10-27 supervised controlled-load contract

`3c94765` implements the supervised controlled-load contract and pushes it
before target execution. The VAD collector starts the worker only after the
recorder reaches `Running`, checks that it remains alive, sends an explicit
stop signal during retirement, validates the worker result identity and exit
status, requires coverage of source duration plus postroll, and records
cleanup/validity separately from collection success. Unit coverage includes
missing-result, short-window, stale-result and clean-stop cases.

The first VM run used the same 48 kHz, 48,000-frame schedule and endpoint as
the repaired collection repeat. It exited 1 before the required load window
completed: `elapsed_seconds=0.5462443` versus
`required_window_seconds=1.2`; the worker itself exited zero and observed the
stop signal, so the failure is correctly attributed to insufficient collection
coverage rather than an orphaned load process. The same run also retained one
virtual xrun, 33 soft events, 41 VAD completion gaps, 87 independent capture
gap events, 2,128 gap frames and two discontinuities.

The run is archived at
`target/vad-target-archive/20261027-controlled-load-3c94765-15s/` and the guest
now retains only `e2e_latency_probe-3c94765.exe`. Stage 7 remains open. Before
any 30-minute or two-hour candidate run, fix the underlying early quality
failure, add a Process Loopback supervisor using the same evidence contract,
and run a matched VAD/Process sweep over target, render queue, engine block,
period and CPU-load settings. Only then proceed to calibrated physical
Input-to-Bus-to-Output measurements and the 10 ms gate.

### 2026-10-27 Process Loopback controlled-load supervision

Commit `6f18fd5` extends the same controlled-load contract to Process Loopback.
The worker now starts only after both native capture and render owners report
started, uses an `UntilStopped` session while supervised, has a bounded startup
deadline, detects early exit during polling, and must cover the requested
window. Result identity, exit code, stop acknowledgement, timeout and cleanup
are validated and retained under `native.controlled_load`; missing start,
short coverage, timeout or cleanup failure keep the run failed/unqualified.
Native owner stop is requested before the capture/render owners are joined.

The focused probe tests (115 passed), strict example Clippy, format, diff
checks and the host release build pass. No new native Process Loopback smoke
is claimed by this commit. The current VM still contains only the previous
probe binary; deploy and verify the new binary before collecting a short smoke.
Stage 7 and the physical qualification gates remain open.

The acceptance target is unchanged: for an `Input -> Bus -> Output` graph with
no extra DSP nodes, achieve stable continuous physical output with calibrated
end-to-end p95 latency below 10 ms, measurement uncertainty at most 10 ms,
continuous marker evidence and zero quality failures. Do not promote smaller
defaults or claim VAD/Process superiority until matched, calibrated runs pass;
VM measurements are diagnostic only.

The host release probe from `96a03de` was copied to the VM as
`e2e_latency_probe-96a03de.exe` and verified on both sides at 3,843,584 bytes
with SHA-256
`C5CC1A01A98952BD3F75CF2AE8BC6E2F144B9336E9703D7B0E0933A739739B3F`.
The previous guest probe was deleted only after that verification. The guest
now retains only the current probe, with no run artifacts or active probe
process, and has 30.11 GiB free. A short Process Loopback smoke was not run:
the guest had no real audio-producing source process or suitable source PID.
The probe's controlled-load supervision is therefore code-tested but still
lacks fresh native Process Loopback evidence.
