# VAD pause fencing, deployment and bounded native observations

Date: 2026-10-31, Asia/Shanghai. Driver source: `6d9e32f`.
Target: physical `LAPTOP-NP49RG6H`, through the pinned existing SSH identity
at `frp-rug.com:14857`. No driver is installed on the development host.

This closes reproduced source-level cancellation/fencing cases. It does not
establish the cause of the earlier native holder failure, waveform continuity,
calibrated physical latency, a smaller default, or completion of S1-S3.

## Direction invariant and separate mainline integration

| Windows endpoint | Moiren graph | Other applications |
| --- | --- | --- |
| Virtual speaker / Playback | Input / Source | Output / render |
| Virtual microphone / MIC | Output / Sink | Input / capture |

Production mapping remains `Playback => Source`, `Microphone => Sink`, with
`InputSource::VirtualPlayback` and `OutputTarget::VirtualMicrophone` in graph
preparation. It is not reversed in this repair. Main `6b4ffe6` already contains
the independently extracted PR43/44/45/47 foundations; it is an ancestor of the
experiment. Draft PR26 is not merged wholesale.

## Reproduced production-path defects and repair

The host harness extracts the actual production routines, stubbing their DDI
edges rather than maintaining an independent implementation. Its RED cases
demonstrate three distinct boundaries:

- Ordinary Pause purges pending PCM as `Cancelled` without advancing the gate
  epoch. The production Rust same-stamp cancellation check must remain strict.
- Publishing an epoch before acquiring the existing PCM-drain lock is not
  sufficient: a request can acquire that fresh stamp and then be purged without
  another stamp change.
- A request parked after Stop can be cancelled by stream release without a
  further release fence.

`VadPcmDrainGuard` is stack-only and noncopyable. At PASSIVE level it enters a
critical region, acquires rundown and then the endpoint PCM-drain mutex;
destruction releases these in reverse order. An uninitialized or rundown-denied
guard is inert, but native stop/reset bookkeeping must still execute.

Stop drains the timer before taking this guard, then reacquires stream State.
The guard spans fence publication, bookkeeping/description, purge and queue
restart. Pause advances the epoch while preserving the packet clock; reset
retains its separate clock-reset behavior. Release publishes another fence
atomically with Closed/zero shape under the shared lock, rejects fresh admission
and clears ActiveStreams only after purge. There is no wait or purge under a
spinlock, driver ABI change, new buffer default or weakened Rust cancel guard.

## Software and build evidence

- All13 MSVC host suites pass, including54 pause-fence scenarios covering
  event/timer, Playback/MIC, pause/reset, release, saturation and controlled
  fresh-stamp interleavings. Test handshakes/completion are bounded to5seconds;
  the suite has a30-second watchdog.
- All8 actual WSL Ubuntu portable suites pass. They include the extracted
  pause-fence and UTF-16 name regressions, not a native kernel execution.
- The Rust provider selection passes93tests. Integrated cherry-picked source
  is independently rerun for pause54/provider93 and WDK analysis.
- Final WDK analyzed build, InfVerif and catalog generation pass with no
  warnings/errors. The initial C28251 annotation warning is retained and fixed
  with matching declaration annotations and `_Use_decl_annotations_`.
- CI run `41126848979` at older `dec165a` fails only Ubuntu Clippy: the
  non-Windows non-test example leaves `is_ac_diagnostic` unused. `8bf971e`
  gates the helper to Windows/tests. Exact Linux-target RED then example and
  example-test Clippy GREEN, Windows78example tests and strict Clippy are
  retained. This local repair does not relabel that failed remote CI as passed.

These bounded host tests and independent source review do not exhaust kernel
concurrency or prove that every archived native failure has the same cause.

## Installed package and rollback

The signed package is deployed under `D:\MoirenTest\20261031-6d9e32f\driver`.
All8 candidate/rollback signature and catalog-member checks return0 on the
target before install. The pinned existing signer is
`7F17CCDCA257D6ABC84255DD0D1854E93B9B5C1D`; its private key is not exported.
Host verification's untrusted-root failure is preserved; no host/target trust
store change is made.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `MoirenVad.sys` | 139160 | `AED94D18375BFD59B78BA84C3691B4B69C1EEAD4E5210F36759D018ED3443268` |
| `MoirenVad.inf` | 1801 | `227BDAD3BE1A2B840EF2DD3E41081E44E63B96428C4FBC356959DD4782E5EEA5` |
| `moirenvad.cat` | 2484 | `32ADAE603D56E3ADCAA9A39B6967ECD8D0C573FE2B5718300193D2CB100066ED` |

Install and root restart both return0, with no reboot. The installed identity is
`ROOT\MEDIA\0001`, `oem201.inf`, version `3.1.1.607`, service Running/PnP OK,
problem0. Its DriverStore service-file hash matches the signed SYS above. This
is installed-file/PnP verification, not independent in-memory image attestation.
The pnputil log lists46 VAD_RAW binding records plus the root; these records
are not46 independently proven present endpoints or active clients.
Old `1513274` / `oem200.inf` / `3.1.1.606` and its verified rollback package
remain available. The frozen pre-install package manifest intentionally still
says `NativeDeployed:false`; `deployment.json` records the subsequent install.

The app remains `6827f02`:
`CEC7321075D3529EA2B0FBDC903F4AAA0C55BF7DAC36176CFDC5141380C3B288`.
Both observations use the unchanged holder `ac93295`
(`5C6351F0CA53A482E87E5929EBB23943933F3C87B3EBB69AB38750E6DD1F2DE6`)
and source worker `0fd3fbf`
(`651B0596700791E3C0F3C68E35C52D2D9F0432196F50618C5E9FD8E445D1B94E`).
No build overlaps native acquisition and reusable build caches are retained.

## Old/new native observations

The same frozen collector observes the service-owned route
**Playback -> Gain0.5 -> Microphone**, not the exact S1 unity-Bus fixture.
It selects Playback using stable identity, direction and confirmed graph/native
binding, not friendly name, default role or catalog delta. Each60-second holder
run includes two separately started Shared sources, stopped after about5seconds.
There is no physical output or independently recorded waveform.

| Observation | Old `1513274` | New `6d9e32f` |
| --- | --- | --- |
| Collector / holder exit | 0 / 0, holder passed | 0 / 0, holder passed |
| Holder total / idle ms | 60591 / 60002 | 60815 / 60000 |
| Source1 reported elapsed s / acknowledged frames | 5.2201839 / 250560 | 5.1505511 / 247200 |
| Source2 reported elapsed s / acknowledged frames | 5.230519 / 251040 | 5.1703381 / 248160 |
| Playback stream1 | g1 Running/e1 -> Paused/e1 -> Closed/e3 | g5 Running/e13 -> Paused/e14 -> Closed/e17 |
| Playback stream2 | g2 Running/e4 -> Paused/e4 -> Closed/e6 | g6 Running/e18 -> Paused/e19 -> Closed/e22 |
| Retained transport-error text | 2 Cancelled after Closed, epoch changed | No transport_io_error/pcm_cancel/Cancelled observed |

Independent read-only review finds no nonempty nested error, graph_error or
cleanup_error in the complete holder reports or either set of14progress
snapshots. Source receipts match their selected identities/configuration; all
sources stop and join successfully, with zero timeout. Both holders report
service joined and control session released, without forced termination.
Each source also submits1056preroll frames: total native submissions are
251616/252096frames on the old driver and248256/249216on the new one.
All four source schedules remain intentionally incomplete after early Stop.
Both holders retain one sampled Playback closure/reopen with22ms sampled
closed time; absence of nested errors does not mean uninterrupted admission.

The old native observations retain unchanged-epoch Pause; the new ones retain
an epoch increment at Pause. Startup generation counts differ, and their cause
is not established. Debug transitions lack exact native timestamps: source
process lifetime, source reported elapsed and progress observer intervals are
different quantities. Acknowledged frames prove submission, not delivered PCM.

The original fatal holder failure occurs in neither run, so this is not a
causal A/B demonstration that the native failure is fixed. The raw collector
keeps `Qualified:false` and `PauseExercised:null`; the matching-stream review
above supplements those frozen records without rewriting them. Holder
`endpoint_absence_confirmed:false` also remains; separate final PnP audits see
only the root and no owned children. No-error text is not a general zero-fault
or waveform-continuity verdict.

Boot remains `2026-10-30T09:50:49.659845+08:00`. The final audit sees no owned
audio/build processes. No speaker volume/mute/default role, privacy, autologin,
system package, cache or unrelated process is changed.

## Verified evidence archive

Host evidence root:
`D:\coding\moiren-evidence\20261031-vad-pause-fence`.
Remote original runs and deployment receipt remain under
`D:\MoirenTest\20261031-pause-fence`; no originals are deleted.

The first archive attempt fails in PowerShell5 before writing its manifest/ZIP:
`Measure-Object` cannot read OrderedDictionary `Bytes` as an object property.
The retained retry uses explicit typed accumulation, PSCustomObjects, normalized
entry paths and create-new writes; it does not overwrite an earlier archive.

The completed `pause-fence-6d9e32f-20261031.zip` contains113payload entries
plus its manifest,3591652payload bytes excluding the manifest,404219ZIP bytes.
It includes both complete native runs, deployment receipts, signed package,
frozen runner/deployer and package manifest. Target and host independently
verify every entry's length/SHA-256, count and aggregate bytes. The host also
compares all113entries against its previously downloaded original files.

- ZIP SHA-256: `7AE1C04275642BDAAE85F8D2A731688AEAAA32E9E6EBB51F3FDB879A93270DFD`.
- Archive-manifest SHA-256: `395E1BC8F4C509338BE930C80E281F207B21C860793FE54B9D1E82047D79A5DE`.
- Frozen runner SHA-256: `63DD7B8226649AA03B9B1EEF8ED884F7D3448EAD9951C4B8427BA74214A15506`.
- Frozen deployment script SHA-256: `B9209D188D0C5AC0469BFD4C9B3052B83DEC747F44B051A6D25B97F30EC48E55`.
- Frozen package-manifest SHA-256: `B13AB0A2A3BC8A15677DEA4F1B917C299E4401442F1A3BC3884E1902AB3A4FDD`.

The host root also retains actual-routine RED/GREEN, Windows/WSL/WDK logs,
CI failure, signed-package preparation and matching PDB. They are not all
claimed as members of the smaller native/deployment ZIP above.

## Remaining gates

Earlier Shared all-zero/SILENT microphone, source/recorder quality failures,
Process delivery-gap/window refusals and nested cleanup failures remain dated
evidence, not erased by these lifecycle observations. Further native signal
work needs a substantive discriminant rather than equivalent headless all-zero
retries. Session0/no-user context remains an unproven confound, not a root cause.
S1 changing-waveform continuity/common physical reference, S2 lower-budget
stable delivery and remaining S3 native admission/qualification stay open.
No VAD/Process latency winner or stable calibrated sub-10-ms result is claimed.
