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

## Verified binaries and three remote native runs

The repair is committed locally as
`19566361a9157248f51fc04fa7db80d7b7414615`, without push. Release app build
finishes successfully in terminal session26384; the separately rebuilt native
probe also exits0. Production Rust is unchanged between builds and commit.
The unrelated `gate.log` retains its original hash and remains untracked.

| Binary | Bytes | SHA256 |
| --- | ---: | --- |
| `moiren-app.exe` |33873920|`9C7A259ED131F5775B61415C57C2786D311D06772E139E8E0741DD02003A045D`|
| `e2e_latency_probe.exe` |7453184|`3FB5330D790767F68B0114162F137E685AE472D17717CF5C4967D8EFA81A2E28`|

Both are staged under `D:\MoirenTest\20261101-pcm-admission-retest\bin`.
`Start-App.ps1` pins the app hash, refuses an existing Moiren, uses an isolated
profile and `info,moiren::audio::vad=debug`. The desktop is **not launched**
over native tests or silently described as recovered.

The runner reuses the earlier reviewed finite regression; only source/image
pins and fresh evidence/task paths change. Interactive/Limited identity,
exact-route admission, source provenance,120-second process wait, metadata
and failure gates remain. Route: VAD Playback -> unity Bus -> VAD MIC;
48kHz stereo Float32, Engine32, negotiated minimum buffers144/144, target1024,
capacity4800, jitter96, declared packet/gap480, recovery cap8. The original
30-second indexed-v2 schedule/amplitude remain. Correctness jitter grace is
not audio acceptance.

| Run | WPP | Stored PCM frames / seconds | Packets | Position gaps / frames | Exit | Collection / cleanup | Quality |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- |
| `admission-native1` |off|259776 /5.412|1804|2 /720|1|false /false|false|
| `admission-native2` |bounded|1456128 /30.336|10112|6 /1296|0|true /true|false|
| `admission-native3` |same bounded repeat|1456416 /30.342|10114|4 /1440|0|true /true|false|

Native1 stops after5.312 source seconds; only254736 of1440000 source frames
are acknowledged. Both providers observe gate epoch1 ->2 with the gate still
open and same Running stream/shape. Playback reports4 driver drops, MIC6 live/
8 retired silence packets; each worker has one live reset/recovery. Collection
rejects changed identity; teardown retains four Playback `0x8007051A` slot
failures plus an earlier MIC shutdown warning. These are **shutdown-terminal
warnings**, not the desktop's `GetOverlappedResult` call-site failure or a new
`pcm_completion_fault`. Without native1 WPP, its epoch-change cause is unproved.

Native2/3 change only WPP instrumentation, then repeat identical conditions.
Both acknowledge1440000 source frames, stop source/recorder, join the host,
remove owned children and exit0; live reset/recovery stays0. Both nevertheless
fail quality on position gaps. Native3 also has one actual MIC silence packet.
Its decoded first no-copy witness identifies endpoint15, open Running gate1,
native packet40, callback QPC1972881736439,123.0978ms after RUN QPC1972880505461.
This is an early native no-copy observation, not a steady-state or calibrated
delay verdict. No startup counter is subtracted to make it pass.

All native1/2 position gaps precede the first nonzero marker. Native3 also has
a later144-frame native-position gap at stored frame59616, in addition to its
three early gaps. Position gaps are not by themselves a count of missing source
samples. The source intentionally contains silence between sparse codes;
exact-zero prefixes31737/32172/32364 stored frames are not unexpected-startup-
silence counts. All packet flags are0, which does not erase position gaps or
prove waveform continuity.

WPP reuses0xf/level4/perf sequential16MiB policy. Owned sessions stop before ETL
reads. Both ETLs are65536 bytes; matching611 TMFs/pinned tracefmt decode159/165
events, reported lost/format/unknown0. Neither contains `Clock rebase cause`;
this cannot retrospectively explain native1 or establish complete fault absence.
Two successful traced retirements do not prove WPP repairs the race.

## Remote visualization and remaining work

Large PCM/scalar traces/JSON/metadata stay remote. Only bounded logs, compact
summaries, two64KiB ETLs and images transfer. Raw per-file lengths/hashes remain
in105911-byte `final-native-evidence.json`, verified SHA256
`D2276E472C597BBE76E5839C2F90DFE03FA176AC0579ED39356E9997860D2646`.

```text
D:\MoirenTest\20261101-pcm-admission-retest\analysis-native1-native2-native3\report.html
D:\MoirenTest\20261101-pcm-admission-retest\analysis-native1-native2-native3\native-comparison.png
```

The remotely generated, downloaded and visually inspected plot shows10ms PCM
extrema/RMS and native position-increment residuals, retaining failures and
separating stored recorder coordinates from source time/physical delay.

The unchanged strict nominal analyzer on native2 finds29 nonzero-support windows
but resolves16/29 codes;13 remain `missing` identities, **not demonstrated missing
source frames**. All29 codes compete per window, with correlation0.7, margin0.05,
RMS0.0001 unchanged. SRC transformation/source-recorder mapping and packet timing
remain required before a waveform verdict. No delay estimate is invented.

Final audit at16:40:57+08:00 confirms no Moiren/probe owners or raw children.
OBS PID12648/QQMusic PID15096 retain original identities. Running `oem205.inf`,
SYS hash, boot, six defaults and render controls remain unchanged. Three tasks
are terminal and retained; owned traces are stopped. No host existing process
is stopped/controlled and no host audio/UI is opened.

Next owning problems: repeatable epoch-change attribution preserving terminal
errors; native startup/no-copy/position-gap evidence; SRC-aware waveform identity.
Desktop recovery, exact physical S1 routes and calibrated latency remain open.
These results do not close S1-S3 or promote a smaller default.
