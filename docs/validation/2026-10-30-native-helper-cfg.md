# Native helper compilation boundaries

Date: 2026-10-30, Asia/Shanghai. Initial source: `e71b9ea`.
Final integration source: `c222df2`.
This follows the [clock-loss deployment](2026-10-30-virtual-clock-loss.md).
It is an offline integration repair, not native audio or S1-S3 qualification.

## Observed failures and minimal repair

PR26 run41039374596 at `5939394` fails both Linux-target audio Miri jobs
before executing tests. Three internal helpers use `cfg(any(windows, test))`,
but all their callers belong to `engine_backend`, whose module is Windows-only:

- `MeasuredProcessDelivery::with_publication_gap_100ns`.
- `ObservedProcessTrace::with_retained_owner_origin`.
- `ProcessTraceArchive::replay_retained_live_window`.

The Linux library-test build consequently rejects unused methods under the
workspace's denied-unused lint. The host reproduces exactly those three errors
with `cargo check --locked -p moiren-windows-audio --target
x86_64-unknown-linux-gnu --lib --tests` and `RUSTFLAGS=-D warnings`.
The initial repair changes their compile condition to `cfg(windows)` and
passes the library/test cross-check. A subsequent full RT-harness cross-check
finds that this narrower condition excludes legitimate shadow-backend test
callers. The final repair described below supersedes that initial strategy;
the failed intermediate checks are retained, not treated as a final PASS.

The same run's Ubuntu test/Clippy jobs also retain a non-test example failure:
`ValidationPolicy::progress_grace` has only Windows production callers and
portable test callers. An all-target Linux Clippy run reproduces the error
after the first three fixes. Both its method and `Duration` import now use
`cfg(any(windows, test))`. The grace arithmetic, 0..3000ms bounds, defaults and
strict-versus-correctness policy are unchanged.

No lint is suppressed, warning policy weakened, suite disabled or workflow
edited. The exact failed CI logs and both local RED cases are retained.

## Verification

- Linux-target library/test strict check passes after the helper repair.
- Linux-target all-target strict Clippy with `vad-probe` passes after the
  example repair.
- Windows audio library615passed /2ignored, counting allocator4passed,
  probe528passed; final all-target strict Clippy with `vad-probe` passes.
- All9validation-policy regressions pass, followed by the full528probe suite.
- Linux-target Miri Tree Borrows39passed and Stacked Borrows39passed, using
  the host's installed nightly with explicit `x86_64-unknown-linux-gnu` target.
  This is actual interpreter execution, not cross-compilation alone. Final
  target build/deployment are recorded after their terminal results below.

## Independent RT-harness integration

The same CI run's Windows test job later fails compiling
`tools/rt-check/Cargo.toml`: its shadow `native_observations` tests instantiate
the library's externally imported `VirtualClockProbe` but cannot call its
private constructor/publication. Local `--no-run` reproduces all fourE0624
errors. The harness now shadow-compiles the real `clock_observation.rs` too,
reexports those local clock types through its existing virtual-owner facade,
and retains the public `ClockTick` contract. Production visibility is not
widened. The real source, generation-retention tests and counting allocator
remain in use; no fake counter implementation is substituted.

Linux cross-checking the whole harness then exposes the initial helper-cfg
repair's missing methods in the intentionally portable backend shadow. The
final `c222df2` restores `cfg(any(windows, test))` for the three helpers and adds
three meaningful portable regressions, rather than excluding that harness or
suppressing unused warnings:

- Publication envelope only widens the reserve, preserves every other field,
  rounds upward and refuses overflow.
- Closed replay origin promotion changes provenance only, not qualification
  or replay mechanics.
- Live retained prefix propagates both outer/nested provenance while keeping
  writers open and qualification false.

The new test callers first fail Linux compilation against the narrower cfg;
restoring the test availability makes both audio and RT-harness all-target
Linux strict Clippy pass. Final Windows audio library618passed /2ignored,
counting allocator4passed, probe528passed, RT harness210passed. Strict Windows
audio and RT-harness Clippy and both format checks pass. Each new portable
regression also executes successfully under Linux-target Miri Tree Borrows.
Final39-test VAD Miri repetitions pass under both Tree and Stacked Borrows.
The new clock integration suite also passes4tests under each borrow model.
Neither earlier failure is erased.

Read-only review finds no P1/P2 issue in the final seven-file code slice.
Windows method bodies, runtime behavior, scheduling and budgets remain
unchanged. Only the probe example retains a narrower compile condition, with
its portable tests still enabled.

## Target integration history and retained evidence

The intermediate `e71b9ea` passes all six offline release stages on the owned
physical target: strict app Clippy, exact graph regression, full app suite
630passed /4ignored, probe528passed and both release builds. It is deployed
at2026-10-30T18:33:22+08:00 with verified rollback files; an independent
18:33:24audit confirms clean source, no app/probe/build processes, unchanged
driver Running /root PnP OK and no audio/system/volume/default-role writes.
Its application hash is
`B789F6FC006FCFE559AA210B141D21C060B111531122155A0F667938E6D3EA2D`,
probe hash `80F6F106E99640125E71A041A39370E5835BFAD7880E9FD844DA6E3B88D4D7D3`.
That receipt proves its own exact source, not the later portable-test follow-up.

Intermediate archive `native-helper-cfg-e71b9ea.zip` is17095bytes with26manifest
files plus the manifest, SHA-256
`4AF1548AB445C3F672AC4E4AC8BEF0D8F6165A5047E1FAF44FF05A7DF96A0FAC`.
Host verification checks the full entry set, every length/hash, exact six-stage
source/exits, test counts, installed/build hashes, rollback and final audit.
Original CI failures and RED logs stay separate; no failed run becomes a pass.

All new logs, verifiers, bundles, scripts and target archives are retained
outside Cargo output at `D:\coding\moiren-evidence\20261030-native-helper-cfg`.
Reusable remote caches remain on D: with jobs2; there is no routine clean,
driver reinstall, requested reboot or native playback retry.

Final `c222df2` repeats all six remote stages successfully, including the full
630passed /4ignored app suite and528probe tests. At18:46:59+08:00 its formal
app/probe entry points are replaced with verified rollback. Final artifact
identity is:

| Binary | Bytes | SHA-256 |
| --- | ---: | --- |
| `moiren-app.exe` | 33490432 | `60228B9F74D99B3DCFE5C379CB4F1E079A826939C743781CBE9B464B3F041EC5` |
| `e2e_latency_probe.exe` | 7026688 | `295E2C4CC99F234C1C5ABC907D7B98B2AFE832BF0F575779FF308D1417187F0C` |

The18:47:01independent audit verifies those installed hashes and both old
backups, clean exact source, no app/probe/build process, unchanged driver
Running /root PnP OK. D free254198317056bytes, C free10351284224bytes;
3941070035bytes of reusable Cargo outputs are retained.
`portable-rt-c222df2.zip` is16768bytes,26manifest files plus its manifest,
SHA-256 `C64D9478C5BB77387C80603235AF84E394FE3BF241A7F7B517774191F82F7F62`.
The host independently verifies its exact entry set, all sizes/hashes,
six-stage results, source/deployed identities and test counts.

This checkpoint does not establish historical clock-loss totals or native
waveform continuity. S1 common calibrated reference/waveform quality, S2 native
lower-budget feasibility and the post-bugcheck testing decision remain open;
S3's earlier real desktop Apply/replacement evidence is not invalidated or
retested by this offline work. No VAD/Process winner or sub-10ms stability
claim follows. S1-S3 and the full deployment/testing goal remain incomplete.
