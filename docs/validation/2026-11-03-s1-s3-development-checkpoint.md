# S1-S3 Development Checkpoint

Date: 2026-11-03, Asia/Shanghai. Baseline: `003fc3c`.

## Scope And Priority

Continue application development independently of unavailable calibrated physical
capture equipment. Initial-packet investigation remains deferred. Do not repeat
an unchanged soak, promote a smaller default, weaken a gate, or manufacture new
offline diagnostics merely to keep the qualification goal active.

A bounded original-scope audit finds no new actionable S3 software gap in the
Process bridge's desired/active configuration, exact validation, stale/retained
Apply, or production monitor reporting. This static result is not a native
acceptance pass. Original S1-S3 software integration and unresolved native or
physical acceptance remain separate; see the [scope ledger](2026-10-29-s1-s3-software-ledger.md).

## Fresh Regression

An exact `git archive 003fc3c` excludes all 27 preserved dirty/cache files.
Host execution is compilation and pure/headless tests only, with no native
application window, audio exercise, or existing-process manipulation.

- Windows audio library `zero_refill`: 5 passed, 0 failed.
- Marker / rate / qualification analyzers: 19 + 8 + 24 passed, 0 failed.
- Application `gpui-test measured_ui_tests`: 20 passed, 0 failed.

These 76 tests are software regression, not device stability or delay evidence.

## Remote Resource Recovery

At 06:41:45, read-only preflight finds 1496396 KiB free, below the existing
8388608 KiB native-test guard. No native test starts and no guard is lowered.
A bounded process inventory identifies an obsolete owned report reader,
PowerShell PID 25900, created 04:47:49.332203 +08:00, using approximately 27 GB.
Its decoded command only reads the old 0430 task/results and serializes JSON;
that task is already Ready/result 1. It is not an audio owner or user process.

Cleanup checks its exact executable, complete command SHA256, creation times,
and terminal task before killing the held process object. The first identity
check refuses a four-100ns CIM/.NET timestamp representation difference without
stopping anything. The corrected exact check retains both source-specific times.
The initial five-second termination wait expires; no second kill or relaunch
follows. Independent 06:44:26 readback confirms the reader absent and 27630148
KiB free. Protected original OBS/QQMusic identities remain unchanged, with no
audio owner or running Moiren task. No host process is stopped.

This is test-tool cleanup, not an audio production fix. Future status reads use
bounded plain summaries instead of embedding decorated Get-Content objects.

## Evidence

Local root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-s3-phase-audit-20261103-0647`.

## Finite Changing-Source Application Acceptance

One distinct native question is tested while clean-source workspace integration
runs on the host: does the current production Process CLI perform measured
owner replacement and its whole finite run while the source is still submitting
nonzero changing content? The earlier CLI test's ten-second source ended near
baseline completion; its replacement interval was mostly intentional silence.
This closes that excitation coverage gap, not the missing waveform gate.

Remote root: `D:\MoirenTest\20261103-s3-changing-source-0647`.
The existing clean `e4de0d7` app and `50c97e5` probe are reused with their exact
previously verified binaries; no driver installation, production code, default,
threshold, timer or buffer change occurs. The app remains the separately
packaged [manual-test build](2026-11-03-process-source-period-and-app-handoff.md).

The task-lifecycle runner executes once via SSH session 0 with its elevated
non-desktop token. Source and app have held process handles, separate logs,
finite deadlines and isolated APPDATA/TEMP. PS5 syntax and deployment hash
verification pass before execution; bounded independent safety review finds
no launch blocker. No user OBS/QQMusic, window, default device, endpoint control
or host-native process is operated. No recorder is opened.

The source is 55 contiguous one-second deterministic xorshift-v1 noise markers,
48 kHz stereo, amplitude 0.005, no scheduled silence. The same one-second code
repeats; these are not 55 independent indexed identities. Source Shared480/F32,
capacity1056, actually negotiates Client3 with no fallback/auto-conversion.
The exact graph is Process -> unity Bus -> DAWN PRO2, F32/48k/Engine64,
Shared336/capacity738/render512/wake64/callback64. No smaller target is scanned.

Actual runner interval is 06:52:20.226-06:53:22.818 +08:00. Fresh Compatible
measurement lasts5000ms; replacement owner2 is confirmed with exact measured
1536/16384/96 and remains active40000ms. Source PID23328 and exact creation
134381335459821719 match the held process. Native generation1 is backend-local,
not a global replacement identity. Normal app Stop/join and source exit both
return0, no forced termination or cleanup error. At app join06:53:12.651 the
source's completed-submission receipt does not yet exist; the later final receipt
acknowledges all2640000scheduled/nonzero frames. This is submission evidence,
not capture, continuous delivery or waveform integrity proof.

The replacement captures/publishes1920000frames, consumes1918208, and reports
zero dropped, silent, nonfinite, discontinuity, timestamp-error, underrun, reset,
backlog/age/ingress recovery or low-water guard counters. Startup priming2560
and prime-discard384 remain separate. **Six delivery gap-bound violations**
remain: observed maximum112742*100ns=11.2742ms versus the admitted530frame
bound (11.0416667ms). Packet-bound violations are0. The final master retains
**one native wait timeout**, with no event phase established. Neither timing
observation is erased, explained as harmless or treated as an identified audio
production defect. No unchanged rerun or bound increase follows.

Only256of30012queue events and256of4000producer events remain, with29756and
3744overwritten respectively. The plotted tails cover0.3360897s and2.5502594s,
not the whole40second replacement. Zero reported fault counters and clean tails
do not establish continuous output. Lifecycle completion is true; waveform,
sustained/desktop/retained-Evaluate and calibrated physical acceptance remain
unverified. All qualification flags stay false and physical latency stays null.

The remote isolated Python environment creates a four-panel operational plot,
then rehashes the three original closed result inputs. Only compact logs/JSON/
PNG transfer; no PCM or scalar archive is copied. ZIP571133bytes, SHA256
`715015AFA47277993A8F5829C64D6A39160958DC9F8789AB7FCC36230B084281`;
all17payload entries plus the manifest are exactly verified. The actual PNG is
visually reviewed, including both timing warnings and the retained-tail limits.

Final 06:55:40 remote audit rehashes all17closed payload files and ZIP without
changing them. No app/probe/analysis owner or running Moiren task remains.
Original protected process identities, loaded driver hash/boot and app binary
stay unchanged. Free memory remains27426964KiB; the stale reader is absent.
No background soak is left running.

## Final Integration

The exact clean archive's Windows feature-workspace tests finish with2356passed,
9ignored, 0failed across71harness summaries, including documentation tests.
Tests use `-j1` and `RUST_TEST_THREADS=1`; native-window tests remain ignored.
Feature-on all-target workspace `cargo check`, strict Clippy with `-D warnings`
and workspace rustfmt check all exit0.
All1389tracked source/archive files match committed003fc3cbytes, and all27
original dirty/cache hashes remain unchanged. No Linux, Miri or remote CI result
is implied. This increment changes evidence documentation only, not production
code or a buffer/quality policy.

The exact executed runner is14711bytes, SHA256
`5AF0F05592B762F30A3C639584780C57CEA6326616254683ADE7EA7DFC4DC785`.
The deployed app is34150400bytes, SHA256
`1217C07CAB24BAFF8B57E6E7BC5536C82B4F9A7F913892389B0B50F922D0B128`;
source probe7612416bytes, SHA256
`6290C3878A6136007DC4551E9DBFCBDF4E8AF92C91D2EFE0467EF58E7F703EDB`.
Independent read-only final review confirms the finite native claims and their
limits, including all timing violations and submission-only evidence.
