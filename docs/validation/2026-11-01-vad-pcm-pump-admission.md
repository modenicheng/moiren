# VAD PCM pump: stop new IO after local admission revocation

Date: 2026-11-01, Asia/Shanghai. Parent source: `ffc77c3` on
`feat/minimal-io-latency`. This repairs a demonstrated local pump contract,
not an established cause of the desktop's `0x8007051A` completion failure.

## Reproduced defect and repair

The outer worker already checks session admission. Playback could nevertheless
refill every READ slot after revocation inside a pump, and MIC could consume
and submit fresh carrier audio after revocation. The deterministic initial
regression run fails all four tests: `pcm-admission-red-161246.log`.

- Playback rechecks `SessionState::admitted()` before each new READ and before
  reporting a current pump. It retains already-completed PCM and pending IO.
- MIC collects pending completions first, preserving terminal errors. It then
  checks admission before consuming carrier audio and again before WRITE.
  Revocation after encoding counts the consumed packet as a carrier drop.
- Revoked pumps return the existing `PumpState::Reset` teardown signal, without
  incrementing epoch-reset or recovery counters. Native waits and phase sleeps
  are skipped, but pending peer requests still require owner drain.
- Existing tests begin with the production precondition of admitted sessions.
  Capture-race expectations now require teardown after revoked/cancelled/
  inactive/drained states while retaining PCM, artifacts, telemetry and
  capture-enabled/disabled equivalence assertions.

Ten final regressions also cover ready PCM, completion/submission errors in
both directions, peer drain, no allocations/deallocations, and every local
authority condition. Revocation *inside* native submission preserves genuine
READ/WRITE failures; no revision error becomes cancellation or recovery.

These checks are not an atomic lock against native gate/stream epoch changes.
IO racing after the final check can still fail, and that error remains fatal.

## Current-source software checks

Evidence directory:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.

- Final focused admission tests: 10 passed, zero failed;
  `pcm-admission-focused-final-161859.log`.
- `cargo test --locked --workspace`: 2075 passed, 8 ignored, zero failed
  across 67 result summaries; audio library: 743 passed, 4 ignored.
  Receipt: `pcm-admission-workspace-final-161913.log`.
- Headless `measured_ui_tests`: 15 passed, zero failed;
  `pcm-admission-headless-ui-162230.log`.
- Strict workspace/all-target Clippy: exit 0;
  `pcm-admission-clippy-162408.log`.
- `cargo fmt --all -- --check`: exit 0 in terminal session 26384. The
  successful empty formatter output does not create a Tee-Object log file.
- Release build and native revalidation are recorded separately after they
  actually finish. Software checks do not qualify native waveform or latency.

## Authorized remote owner handoff

The user authorizes stopping obsolete test-machine processes, explicitly
forbidding host-process manipulation. The bounded SSH script validates
`LAPTOP-NP49RG6H`, PID 13120, creation time, executable path/hash and driver
hash before preserving the old 10875-byte log and process/profile metadata.

At 16:23:05.523+08:00, normal window close returns false; exact verified PID
13120 is forcibly terminated and confirmed absent. OBS PID 12648 remains
unchanged. At 16:25:35.672+08:00 there are no Moiren/probe/python owners or
VAD children, only the healthy `ROOT\MEDIA\0001` control bus. QQMusic and OBS
remain running. The running SYS retains SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.

Remote evidence: `D:\MoirenTest\20261101-pcm-admission-retest`.
Local receipts: `owner-preserve-stop-162400.stdout.log`,
`pcm-admission-retest-preflight-1626.stdout.log`. No host audio/UI/process
control, driver replacement, reboot, default-role or volume changes occur.
The old failed owner is not described as recovered. S1-S3 remain incomplete.
