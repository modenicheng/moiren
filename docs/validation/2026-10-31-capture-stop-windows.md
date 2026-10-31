# Capture retirement call-window evidence

## Scope and fresh checks

`18d1552` integrates independent author commit `0b52a06`: probe-only,
fixed-size retirement observations in `measurement_session.rs` and
`vad_collection.rs`. These retain the control-side close-gate call window,
first stop broadcast, worker observation of that flag, actual native
`stream.stop()` call and outcome, and native join. Missing clocks, unexecuted
operations, errors and panics remain distinct. A joined worker need not have
succeeded; a stop before Start is not reported as a native Stop.

Preparation failures can retain partial receipts. Historical admitted stamps
are not current gate identities. No extra QUERY, public capture-report ABI,
driver, buffer, period, retirement order, quality threshold or default changes.
Only the first true worker stop observation reads the clock; no per-loop
active clock instrumentation is added.

Root independently passes585 full probe tests,22 retirement-focused tests,
strict example Clippy, formatting and Linux-target example/tests compilation.
These integrated counts are not the author's earlier564-test revision, nor
executed Linux/native audio tests. Release build uses two jobs and a frozen
source pin. The7243264-byte probe has SHA256
`77559B340C3E8366F203CBE1DC42A82BE18B5B283E71C0A30F50B1484234463D`.
Evidence root: `D:\coding\moiren-evidence\20261031-stop-window-native`.

## Native acquisition and independent audit

The single actual `stop-window-1` run starts2026-10-31T05:16:42.2339237Z and
finishes05:17:17.9517317Z on the physical target. The runner differs from the
previous reviewed indexed-v2 runner only in paths/trace-session prefix;
period42/42 and scalar-provenance63/63 fixtures pass. It preserves the same
30-second original29-marker1024-frame indexed schedule, Playback default480,
MIC requested/actual minimum144, Engine32, target1024/capacity4800/jitter96,
recovery8 and correctness-mode3000ms progress grace. No physical render output
is opened. Speaker/Playback remains Moiren Input; MIC remains Moiren Output.

The probe exits0 with source submission, collection and cleanup complete.
Recorder stores10109 complete144-frame packets,1455696 frames and11645568
PCM bytes, packet flags0 throughout. This is operational success only:
`quality_complete=false` and `qualified=false`. MIC active silence0 becomes
final Removed silence1; final provider cleanup retains reset1/recovery1.
The existing `quality_passed=true` aggregate does not supersede those faults
or unknown classifications. No waveform-identity pass is claimed for this run.

Archive `stop-window-1-evidence.zip` is1232670 bytes,22 payloads plus manifest,
SHA256 `6B65118796609250CBD899D672B145E191B0A09A440F1E1E6A02CA0CBBB91B49`.
Root verifies CRCs, exact membership, each original/extracted length and hash.
The independent archive/PCM/metadata/call-window audit passes64 checks with
zero findings; its SHA256 is
`4F4470EF94A31A69388FF806A11C82696FF905A38C883DA7EB343605FF81A207`.
The installed2a5aee5 driver609 and deployed74f app remain unchanged. No owner
or virtual child remains; boot, render master states and default roles are
unchanged. No driver installation, volume/default write or cache clean occurs.

## Exact boundary observations, not tail exoneration

All following coordinates are QPC100ns units, not UTC or native raw ticks:

| Observation | Before / value | After | Outcome |
| --- | ---: | ---: | --- |
| close gates | 987858477423 | 987858497362 | succeeded |
| first measurement stop broadcast | 987858497374 | 987858497385 | succeeded |
| worker observes stop flag | 987858510797 | unavailable | observed |
| actual native stream Stop | 987858510808 | 987858526133 | succeeded, started |
| native join | 987858600841 | 987858600928 | succeeded, joined |

Last retained packet10109 has sample timestamp987858510605, full stored
frame range[1455552,1455696),144 frames and all-zero bytes. Its sample stamp
precedes worker stop observation by192 units and native Stop entry by203
units. Packet10108's stamp987858480394 lies inside the close-gate call window.
These sample-position timestamps are not GetBuffer arrival, lease or delivery
times. Nominal144/48000 sample interval overlaps are calculated coordinates,
not measured callback durations. All last five packets are zero, while the
source schedule already includes silence; successful WRITE can also copy zero.

Neither timestamp proximity nor zero PCM identifies the native sequence that
increments driver `SilencePackets`, or proves that increment's inclusion in
the recorder. Current first-copy/first-rejected instrumentation does not retain
the required exact no-copy witness. The tail is not waived as teardown-only.

## Retained driver trace and next boundary

The matching2a5aee5 PDB/TMF decode processes160 events with zero reported loss,
format errors or unknown formats. No rebase or active-MIC-unserved message is
exported; bounded/passively drained instrumentation cannot prove absence over
the whole run. Endpoint removals and PDO cleanup succeed. One control-worker
`STATUS_REVISION_MISMATCH` remains visible without an operation-identifying
payload: do not assign it to QUERY/close/Stop or claim every native operation
succeeded merely because the probe exited0.

Next diagnostic boundary is the exact MIC no-copy counter increment with
original endpoint/session/generation/gate state, native sequence/frames and
existing callback clock. Any instrumentation must be fixed-size under existing
locking, pass host regressions and WDK checks, and preserve transport behavior.
This receipt is not lower-budget promotion, S1-S3 completion, physical latency,
sub-10-ms impossibility evidence or an established device-limited minimum.
