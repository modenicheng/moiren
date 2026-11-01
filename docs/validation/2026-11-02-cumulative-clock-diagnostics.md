# Cumulative virtual clock diagnostics - 2026-11-02

## Requirement and implementation

S1/S2 require counted recovery, not just a bounded debug-event tail. The native
owner already publishes cumulative `VirtualClockSnapshot` counters and retains
them across owner retirement. This slice exposes that existing evidence in the
desktop Diagnostics view; it does not change clock scheduling or recovery.

The six rows show schema/provenance, actual Engine rate/block/recovery cap,
rendered periods/frames, late wakes and skipped periods/Engine frames, timeline
resets and maximum pre-cap debt, and counter saturation. Skipped Engine frames
are not relabeled as native PCM drops. Counter saturation means a lower bound,
not an exact total or a healthy result. No milliseconds or physical latency are
calculated from these counts.

`None` stays Unknown, including on a physical-master route where this clock is
inapplicable. Unsupported schema versions show their version but leave every
configuration/counter row Unknown. Existing session phase/generation/running
fences still enclose the projection. No stale owner's counters become active
evidence, no Unknown becomes zero, and no integer is converted through floating
point. English/Chinese labels preserve these limits.

Only the application presentation layer, its tests and translations change.
No driver, admission, target/capacity, defaults, servo, RT allocation, or fault
policy changes. The previously withdrawn fail-fast experiment stays withdrawn;
first-packet investigation remains deferred.

## Software verification

Evidence root: `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
`s2-clock-red-0340.log` first records two actual missing-row failures and fifteen
adjacent passing tests. After implementation, the application library passes
521 tests /2 ignored. A later translation assertion adds one test.

Final fresh checks, `s2-clock-*-0331.log`: workspace 2099 passed /8 ignored across
67 targets; GPUI application library 632 passed /3 ignored; strict workspace and
GPUI all-target Clippy pass; release build passes. Separate offline waveform
analyzers pass 12 isolated and 13 held-out linear-clock regressions. These are
software checks, not native audio runs. Read-only review reports no findings.

The eighteen preexisting dirty files are hash-checked against the preceding
receipt and excluded from this slice. No push or memory update is authorized.

## Native evidence boundary

Native audio/UI validation is performed only on the remote target. Builds and
headless tests on the development host do not open the next native test there.
Physical calibration hardware remains unavailable and no physical or
whole-waveform qualification follows from diagnostic projection or UI smoke.

The separately running warm-control recording uses the preceding pinned app,
not the binary built by this slice. Its faults and waveform results belong to
that acquisition and are recorded separately rather than relabeled as a clock
feature result. S1-S3 remain incomplete.

### Remote native smoke receipt

Remote deployment: `D:\MoirenTest\20261102-s2-clock-diagnostics-0342`.
Task `Moiren-S2ClockDiagnostics-20261102-0342` uses a Limited Interactive token,
isolated configuration/TEMP, fresh owned endpoint IDs ending 93/94, a pinned
binary and an eight-minute scheduler limit. No external source, recorder or
physical client opens. The graph is VAD Playback -> unity Bus -> VAD MIC,
Engine48k/F32/block64, declared packet/gap480, target945/capacity4800/jitter96,
recovery cap1. This is a diagnostic feature smoke, not another qualification of
the already failing low candidate.

Application: `bin\moiren-app.exe`, 34040832 bytes, SHA256
`7C40E828DB247589132E480E5D26E451D81C11C542B23921B64C24B42DEC7E35`.
`s2-clock-source-manifest-0342.json` records HEADdb11a64 plus exact dirty source
hashes; it is not a clean-commit-to-binary claim. Runner SHA256:
`A7AAA76F255594F90DF39B41FE36B8100E2BF65A8C5DB0CA46333D93E2F66363`.

Result interval 03:36:57.535..03:38:28.347+08:00. UIA verifies all six new rows
and advancing rendered totals (4529/289856 to5291/338624 periods/Engine frames).
It also honestly observes recovery: late wakes3 to4, skipped13/832 to14/896
periods/Engine frames, maximum pre-cap debt12, resets0, not saturated. These
nonzero clock losses are not a zero-fault native audio verdict. Stop removes
all live clock rows, reaps the owner, and Quit exits0 without forced termination
or cleanup/diagnostic errors. Original OBS12648/QQMusic15096, default roles,
device master controls and boot identity stay unchanged. Task Ready/result0;
final audit03:40:16+08:00 finds no native test owner.

Verified download `s2-clock-native-result-0406.json`:120168 bytes,
SHA256 `96BE91AD8FE4C5A6EB39373F43C0E015758C995BF8672CCE35CD85EF4B6F6144`.
The screenshot `s2-clock-native-plot-0406.png` is visually reviewed, but its
viewport shows the top of Diagnostics, not the six lower rows. Their presence,
values, update and post-Stop disappearance are UIA assertions, not a claim that
the screenshot visually contains them. PNG270965 bytes, SHA256
`1D2CDFA53A75709D2B7A9BFD86B43E1E79E0738FA0C68BBD1D2BF5C9B417B4E0`.
