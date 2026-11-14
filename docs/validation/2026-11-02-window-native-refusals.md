# Native Process window refusal acceptance

Date: 2026-11-02, Asia/Shanghai. Application source: `a8af927` on
`feat/minimal-io-latency`, including the feedback accessibility repair
`c6a34a3`. This is a finite functional acceptance, not a stability soak,
waveform acquisition or physical latency qualification.

## Isolation and provenance

The application is built with `cargo build --locked --release -p moiren-app
--bin moiren-app` in an independent `git archive` export. The release build
exits0 in56.38s. No uncommitted VAD/PCM/SRC experiment is included. All27
original dirty/cache file hashes match before launch; the main worktree is
clean. Only compilation and headless tests run on the development host.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-window-native-20261102-1545`.
Remote root: `D:\MoirenTest\20261102-window-native-1545` on
`LAPTOP-NP49RG6H`. The application SHA256 is
`BE193BBC037F266208BA077DD601DB092E733E15FF798D8EEAEC5DD7DAE66465`.
`native-intent.json` separately pins the source archive, worker and supervisor.

The task uses a Limited Interactive token, an isolated project/preferences/
APPDATA/TEMP, exact source identity and unchanged explicit endpoints. Its held
worker watchdog is150s and the outer scheduled-task limit is240s. Cleanup
can stop only held worker/app/source handles verified against recorded exact
identities. Existing OBS and QQMusic processes are external and protected.
No default endpoint, endpoint volume, driver or low-latency default is changed.

## Acceptance scope

One native attempt starts at15:41:04+08:00. It checks:

- Live policy omission: an owned native feedback label explains all five
  required choices, and Apply is disabled without InvokePattern.
- Closed prefix before collection: native admission/feedback explain the
  missing candidate/evidence; Evaluate and Apply remain disabled.
- A Compatible Process source -> unity Bus -> DAWN output owner: after finite
  exposure, all five live policy choices are explicit. A bounded-history or
  EventLimit refusal must remain readable and preserve requested/active
  Compatible settings and the original owner, with no per-wake fallback.
- After normal joined Stop, retained EventLimit evidence must refuse closed
  evaluation and Apply, without creating a replacement owner. Quit is normal.

The initial activation message is empty. This procedure does not claim native
coverage of a nonempty activation result or successful live/closed-prefix
replacement. Its source schedule is40s of changing markers; submission alone
does not prove captured waveform continuity or latency.

## Results

The single remote worker runs15:41:05-15:42:34+08:00 and completes normally.
The scheduled task is Ready with result0; the supervisor and worker both
complete with exit0. App PID17372 and source PID19844 exit0 without forced
cleanup, residual owners or cleanup errors. The exact protected OBS/QQMusic
creation identities, all default endpoints, render volume/mute settings and
running driver path/hash remain unchanged.

All four functional checks pass. Six status observations across three repaired
rows are unique, owned `ControlType.Text` nodes with `IsOffscreen=false`.
Six disabled inspections of three distinct controls report `IsEnabled=false`
and no InvokePattern; no disabled action is invoked. The explicit live policy
is4096/4096/480/2/100000, with
the span in raw100ns. Live and retained closed-prefix feedback both preserve
the exact `Process trace collection refused: EventLimit` refusal. This is
successful refusal handling, not a passed trace or an evaluated candidate.

Requested and active summaries both remain Compatible with no observed-window
selection. The log contains exactly one owner start. Its cleanup finishes and
it is reaped before the source is stopped. Stopped Diagnostics retain that
source PID/creation identity and session1/native-owner1. Native configuration
remains48kHz/F32/block64, Shared requested period336/actual capacity738,
render target512/wake64/callback64, Process prime2048/capacity8192. These
nominal frame budgets are not measured end-to-end delays.

The graph Stop is requested40.039s after source activation. The source final
reports42.2945181s elapsed and acknowledges the complete1920000-frame,
39-code schedule. There is no recorded waveform in this run. Reviewed live
and closed-prefix screenshots agree with the native metadata. The archive
contains33 files, all independently size/SHA256-verified; its728082-byte ZIP
SHA256 is `9E3F6052F1B28976E3450BBA7CA92A53293FFB1168307EF13A14A6C1E42036F9`.
`parent-assertions.json` also checks the raw terminal receipts, labels,
disabled controls, exact policy, owner count and joined cleanup order.

The requested current-pin headless regressions run concurrently from the same
clean export, with no native host audio/UI:

| Command | Result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib zero_refill` | Exit0;5 passed |
| `cargo test --locked -p moiren-windows-audio --test latency_marker --test latency_marker_rate --test latency_qualification` | Exit0;51 passed across3 surfaces |
| `cargo test --locked -p moiren-app --features gpui-test measured_ui_tests` | Exit0;20 passed |

No failures or ignored tests appear in these filtered runs; other filtered-out
tests are not claimed as run. `current-pin-headless-regression.json` retains
commands, exits, counts, times and logs. This is not a new full workspace,
Linux, CI or Miri run; the preceding full integration checkpoint stays separate.

## Remaining boundary

This closes native missing-policy and initial/retained refusal feedback for
three repaired rows. The empty activation row, a nonempty activation message
and successful live/closed-prefix replacement are not newly certified here.
Waveform correspondence, historical refused traces, native lower-setting
quality and calibrated physical latency remain open. Compatible defaults,
quality gates and first-packet deferral are unchanged; they do not block
independent development. No unchanged long soak or local native test is needed.
