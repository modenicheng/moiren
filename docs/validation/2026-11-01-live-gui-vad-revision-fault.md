# Live desktop VAD PCM revision fault: fresh log attribution

Date: 2026-11-01, Asia/Shanghai. Source inspected:
`e6076f9f7223ed855dffb2cebca2e7fd8440c6f1` on
`feat/minimal-io-latency`. The remote investigation is read-only; subsequent
local changes add fault diagnostics, not a root repair, a fresh audio run,
or an S1-S3 acceptance result.

## Confirmed current failure

The user's currently running desktop application, PID13120 on
`LAPTOP-NP49RG6H`, has a new fault in its own isolated profile log:

```text
D:\MoirenTest\20261101-manual-dual-track-e6076f9\profile\Moiren\logs\moiren.log
```

The exact source snapshot is10875 bytes, SHA256
`3319787822687DBC037DB84D98AEE5B4CAC43CC2EF3CBF6A3B5CB1D73EA2D142`.
Relevant log coordinates, all +08:00:

| Line | Time | Observation |
| --- | --- | --- |
|35|11:50:10.347353|Audio owner generation3 starts, WASAPI48kHz/F32, Engine256.|
|36|11:54:22.370789|gpui reports `window not found`; no causal link to the later audio fault is established.|
|37|13:02:26.831183|Generation3 reports `WaitingForResources`, `Backend(ResourceUnavailable("GetOverlappedResult(VAD PCM): ... (0x8007051A)"))`.|

This is **not merely the earlier native probe's shutdown failure**. The current
desktop log independently records a PCM completion failure. The binary actually
running is:

```text
D:\MoirenTest\20261101-manual-dual-track-e6076f9\bin\moiren-app.exe
SHA256 23B512987D872E47E704AD921697F2EB4EC040C6A3533FD1A098608A1FFE2FDC
```

The clean Process-only acquisition subsequently opens its native device at
13:18:09.628 and retires at13:19:39.686. Its successful Process owner cleanup
does not exercise or clear the desktop's failed VAD PCM owner. The earlier
attempt's local SSH launch receipt also starts later, at13:11:47.788.

## What the code establishes, and what remains unknown

Microsoft documents Win32 `ERROR_REVISION_MISMATCH` as1306/0x51A. The local
`Win32Exception(0x51A)` description agrees with the log. The error label alone
does not establish an installed binary/driver ABI mismatch.

Relevant current-source paths:

- `driver/vad/src/Pcm.cpp:55`: reservation returns
  `STATUS_REVISION_MISMATCH` when `CanQueuePcm` fails.
- `driver/vad/src/Pcm.cpp:344`: queued transfers start with the same rejection
  and proceed only when `SameStream` and the native shape match.
- `driver/vad/src/Lifecycle.h:160`: the predicate includes endpoint/session/
  stream/gate identity, admission, readiness and native stream state; it is
  more than an application version check.
- `driver/vad/src/Children.cpp:502`: a native clock discontinuity can also
  invalidate the PCM epoch. Gate closure and stream release have other epoch
  invalidation paths. These are candidate triggers, not findings about this run.
- `crates/moiren-windows-audio/src/vad/pcm.rs:114`: a collected non-cancellation
  error becomes `GetOverlappedResult(VAD PCM)` with the original Windows error.
- `crates/moiren-windows-audio/src/vad/provider/transport.rs:140`: non-cancel
  faults remain fatal; only typed cancellation reaches the existing
  stop/admission/epoch recovery check.

The desktop log uses the default INFO filter. Its DEBUG-level
`transport_observed`, `transport_io_fault` and gate snapshots were not retained.
The new log proves the failing operation and generation, but does **not** identify
Playback versus Microphone, the failed request's original stamp, or the exact
driver state transition at13:02:26. Do not claim a particular gate-close race,
timer rebase, app-version incompatibility, or link to the gpui error as proved.
Do not turn this terminal fault into cancellation to make the run pass.

## Correlation and evidence preservation

Remote evidence:
`D:\MoirenTest\20261101-live-vad-fault-1400`.
Local evidence:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

- A bounded `FileStream` snapshot with ReadWrite/Delete sharing reads the
  active GUI log without waiting for its writer or restarting the application.
  `live-gui-moiren-1356.log` matches the remote snapshot's SHA256.
- `live-correlation-1356.json` retains the pinned executable, profile file
  metadata, bounded OBS log copies and the two System events found around the
  failure. Those events are unrelated-provider informational events; absence of
  an audio System event does not exclude a driver epoch transition.
- The latest user OBS log shows VAD initialization at11:57:37.903 and recording
  stop at11:59:44.131, but no13:02 entry. Its age and log coverage do not prove
  an uninterrupted native stream or exclude an OBS-side transition.
- Two timed-out read-only diagnostic scans remained running. Cleanup verifies
  each exact PID, creation timestamp and decoded command SHA256 before stopping
  only those scans. The user's Moiren PID13120 and OBS PID12648 remain present.

At14:28:37+08:00 a fresh bounded remote snapshot is still10875 bytes and has
the identical SHA256. The log has no later recovery or audio-start entry.
Moiren PID13120 and OBS PID12648 still have their original executable paths
and creation identities. This confirms only the retained failure and process
presence, not responsive UI or resumed audio.

No local or remote playback/recording is started in this investigation. No
driver, audio defaults, volume, OBS settings or user process is changed. No
fault handling is relaxed. The pre-existing untracked `gate.log` remains
untouched.

The next focused diagnosis needs fault-time request identity and a non-mutating
endpoint observation before worker-exit cleanup changes that state, followed by
a deterministic regression for the observed interleaving. This remains an open
VAD correctness issue; the unanalysed clean recording and calibrated physical
latency gates are separate work, not reasons to dismiss this failure.

## Fault-only diagnostic increment

Local worktree changes based on e6076f9 add:

- ERROR-level `pcm_submit_fault` / `pcm_completion_fault` with ticket, IO
  direction, submitted shape (including endpoint/session/stream/gate), packet
  stamp, pending count and numeric HRESULT. Incomplete polls, cancellation,
  quota deferral and intentional late MIC drops keep their existing behavior
  and are not promoted to fault logging. No PCM bytes are logged.
- ERROR-level `transport_io_fault` before any diagnostic wait, followed by
  exactly one existing same-file QUERY and `transport_io_fault_observed`
  before this worker's exit cleanup. This bypasses the mutating recovery
  poll: no gate operation, reset or admission change is authorized by it.
- The original terminal error is returned unchanged even if stop/revocation
  races, the observed epoch changes, or QUERY fails. QUERY uses the existing
  control deadline (5 seconds); it can delay fault cleanup, not RT rendering.
  Concurrent lifecycle activity may still change the endpoint before QUERY:
  its result is a later observation, not an atomic cause snapshot.

The new injected-control regression first fails because the original branch
performs zero queries instead of one, then passes after the increment. Its
five cases cover stop, revoked admission, newer closed stream/gate, QUERY
failure and unchanged state, preserving the original error, zero gate calls,
admission, zero reset/recovery counters and the requirement to drain peers.
The low-level terminal-fault regression now covers both READ and WRITE for
`ERROR_REVISION_MISMATCH` and device-disconnected errors, preserving the
Windows operation/code and retiring only the completed ticket.

The original desktop is not restarted or hot-patched. It cannot emit these
new fields retroactively; further runtime attribution needs a run of the
diagnostic binary without a second competing VAD control owner.

## Software verification and remote diagnostic binary

Fresh local software verification (no host playback/capture/application UI):

```text
cargo test --locked -p moiren-windows-audio --lib
  728 passed, 4 ignored, 0 failed
cargo test --locked -p moiren-windows-audio --test latency_marker --test latency_marker_rate --test latency_qualification
  19 + 8 + 24 passed, 0 failed
cargo test --locked -p moiren-app --features gpui-test measured_ui_tests
  15 passed, 0 failed
cargo clippy --locked -p moiren-windows-audio --all-targets -- -D warnings
  passed
cargo fmt --all -- --check
  passed
cargo build --locked --release -p moiren-app
  passed
```

After splitting fault logging into before/after-QUERY events, the full library
suite is rerun on final source:728 passed,4 ignored,0 failed. Strict Clippy,
the two focused terminal-fault regressions, the15 headless UI tests and the
release build also pass on that final source. This is not a full workspace/CI
check.
Receipts remain in the local evidence root (`vad-diagnostics-*.log`).

The diagnostic binary is staged side by side on the target, without replacing
the running image or copying/writing its live profile:

```text
D:\MoirenTest\20261101-vad-fault-diagnostics-e6076f9\bin\moiren-app.exe
33873408 bytes
SHA256 7D35CB763992F9C8F5CC254DA9AC870BC61A9082349EC8346F3B00F2B901710B
```

Target SHA256 and length match. An isolated-profile, remote `--help` smoke
exits0 without desktop/audio activation. The first PowerShell wrapper reports
a null `Process.ExitCode` after `Refresh`; that wrapper failure and its output
are retained, not reported as application failure or success. A direct native
invocation with checked `$LASTEXITCODE` confirms0. Final remote verification
at14:39:59+08:00 retains the original Moiren PID13120 and OBS PID12648 with
unchanged executable/creation identities. No competing VAD owner is opened.
No native reproduction, waveform acceptance or root repair is claimed for
this diagnostic build; source changes are uncommitted and not pushed.

## Windows references

- https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--1300-1699-
- https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getoverlappedresult

## Follow-up: retained fault and verified diagnostic handoff

At15:42:40.466+08:00 a fresh shared read of the original desktop log is still
10875 bytes with SHA256
`3319787822687DBC037DB84D98AEE5B4CAC43CC2EF3CBF6A3B5CB1D73EA2D142`.
The last audio entry remains the13:02:26.831 fault; there is no later audio
start or recovery entry. Original Moiren PID13120 and OBS PID12648 remain.
Receipt: `remote-current-fault-20261101-154228.stdout.log` in the existing local
evidence root. Process presence does not demonstrate resumed audio.

The actual old `Start-Moiren.ps1` sets
`info,moiren_windows_audio::vad=debug`. That module-path filter does not match
the explicit `moiren::audio::vad` target used by the VAD lifecycle and transport
macros. This explains missing DEBUG instrumentation, not the PCM fault. The
logger regression checks both filters directly through the application's
existing `flexi_logger::LogSpecification`, without changing the global logger
or process environment. ERROR logging remains enabled by either filter.
The fault-only ERROR events added in584c201 are absent from the older running
e6076f9 binary and cannot be recovered retroactively.

The existing diagnostic binary now has a separate, verified launcher:

```text
D:\MoirenTest\20261101-vad-fault-diagnostics-e6076f9\Start-Diagnostics.ps1
launcher SHA256 0EF00934EA8FFB8AAC291721F5DD71A07B9D0C96F81D2A53B6D7A940DA8FA117
RUST_LOG=info,moiren::audio::vad=debug
```

It pins the already-staged executable's SHA256, uses its own profile, restores
the caller environment after exit and refuses any existing Moiren instance.
At15:48:52.927+08:00, remote PowerShell5.1 parsing reports zero syntax errors.
Invoking it while the original GUI is present reaches precisely the intended
existing-owner rejection, before profile/environment changes or application
launch. Executable length33873408 and SHA256
`7D35CB763992F9C8F5CC254DA9AC870BC61A9082349EC8346F3B00F2B901710B`
are reverified. Both audio processes retain their executable paths and creation
identities. Receipt: `verify-diagnostic-handoff-20261101-154840.stdout.log`.
This is a refusal-path smoke, not a new audio acquisition or successful GUI
handover. The old launcher, live profile and running binary are not replaced.

A new injected `EndpointControl`/`PcmIo` contract test deterministically models
QUERY observing a Running/open epoch5, then a terminal PCM completion after
the same stream becomes Paused/open epoch6. It checks original0x8007051A
propagation, no refill or carrier publication, only the completed ticket
retired, its peer still requiring drain, one fault-only query, no gate mutation,
and zero reset/recovery changes. The existing fault-observation matrix adds
same-stream/open newer epochs in both Running and Paused states (seven cases).
These tests pass on current diagnostic code; they are not a newly failing
production-defect reproduction and do not establish which transition happened
in the desktop. No admission, pumping, cancellation or recovery behavior is
changed in this follow-up.

Fresh verification on9f1a262 plus these test-only changes:

- `cargo test --locked -p moiren-windows-audio --lib`:729 passed,4 ignored,
  zero failures; the new interleaving and seven-case fault observation pass.
- `cargo test --locked -p moiren-app --lib logging::tests`:4 passed.
- `cargo fmt --all -- --check`:passed.
- `cargo clippy --locked -p moiren-windows-audio -p moiren-app --all-targets -- -D warnings`:
  passed. No full-workspace, headless-UI, release-build or CI rerun is claimed
  for this test-only increment.

Earlier software suites and the separate Process-only OBS analysis remain
their own evidence, not native VAD qualification. A real fault-time request
stamp and endpoint observation still require a diagnostic run after normal,
controlled ownership handover. There is no proven root repair, fresh VAD
waveform pass or calibrated physical latency result. The unrelated `gate.log`
remains byte-identical and excluded; no commit, push, restart, driver install,
audio playback or recording is performed in this follow-up.
