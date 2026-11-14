# Process Target1280 Native Acceptance

Date: 2026-11-03, Asia/Shanghai. Production source: `2dd538e`.

## Question And Boundary

One distinct post-fix question: can the production Process monitor empirically
admit and confirm target1280 with the exact528 master SRC demand? This follows
the [whole-wake admission repair](2026-11-03-exact-master-admission.md), not an
unchanged soak, default promotion, target scan or first-packet investigation.
It does not test physical latency or establish waveform continuity.

Host actions are package preparation and device-free runner tests only. Native
audio, source and isolated OBS execute over SSH on `LAPTOP-NP49RG6H` in limited
interactive session1. The existing user OBS and QQMusic are external owners,
not test children. Their processes/settings and all host audio remain untouched
by test control. Unchanged process identity does not prove uninterrupted audio.

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-target1280-20261103-1520`.
Remote root: `D:\MoirenTest\20261103-target1280-1520`.

## Frozen Configuration And Launch

- App34249216bytes, SHA256
  `D6965020D88086AF86E816A2BD48AB73BA78076ECC91E1F1BEB84C20B40D2AF7`.
  This is the final pinned `2dd538e` release, not the predecessor's overlay build.
- Source probe7629312bytes, SHA256
  `853EF19F21D9184D0C97BEDC4E6505549E272F6E2B3650C2AD0555152450EA5D`.
  Reuse the already-pinned sibling probe; no driver installation or rebuild.
- Indexed-v2 source:48k stereoF32, amplitude0.005,40contiguous24000-frame
  codes,960000frames/20s. Source Shared Client3 requested/actual/engine480,
  capacity1056, no fallback/Windows auto-conversion. The3.84MB codebook fits
  the existing4MiB admission limit without enlargement.
- Actual route: Process -> unity Bus -> DAWN PRO2. Engine64/f32, Shared336,
  capacity738, render512/wake64/callback64. Compatible measurement2s,
  confirmed target1280/capacity16384/jitter96 for8s. No second candidate.
- Both owners retain cursor/native PCM and encoded native render evidence,
  each with65536-frame prepared capacity. Independent isolated portable OBS
  captures DAWN stereo48k PCM24 MKV,track1. No monitoring, gain, sync-offset
  or default-device change. This is software capture, not DAC/physical.

The runner requires the final joined Process boundary's actual prepared and
admitted demands to equal528, input policy to equal1280/16384/96, provenance
to be empirical and measured source generation to equal1. Stale704, a different
target, declared evidence and another generation are negative pure-test cases.
The unused inherited VAD helper is not executed or claimed current.

Local pure tests pass47invocations; PS5/PS7 wrapper checks pass29each with
synthetic children. Independent bounded review finds no Process-only launch
blocker. The12,680,598-byte package has SHA256
`8B13728AFBB12FD02E726C9C76E41771501BFACA4179ACE98A91E80AFC611EE3`.
Deployment verifies pins, copies only the isolated Python environment and runs
device-free tests, without launching audio. Final selfcheck exits0, binds intent
SHA256 `A790E8D61288A97103A21814B200DB7C5650F462D0694FC3A12253A1ED474B3E`,
and observes no foreign test owner before the one native launch.

Task lifecycle retains125s cooperative case,170s held Python and420s outer
task limits. Each entry has a create-new once receipt. No timeout/relaunch,
second acquisition, forced cleanup, foreign-window operation or name-based
process kill occurs. A hard wrapper timeout would not itself prove descendant
closure; this run instead has actual terminal child receipts and final audit.

## Actual Result

Case interval15:27:15.041-15:27:55.695 +08:00. The production CLI confirms
measurement generation1 -> replacement owner2; backend-local generation1 is
not a global owner identity. Actual joined demand528, empirical source1 and
target1280/16384/96 match the frozen intent. Confirmed running time is8001ms.
At app join the source's completed-submission receipt does not yet exist;
its later receipt acknowledges all960000scheduled/nonzero frames. This is
submission/lifecycle evidence, not proof every frame reached audible output.

Both immutable native render archives close completely:

| Owner | Releases | Encoded frames | Engine requested | Difference |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 289 | 97120 | 97136 | 16 |
| Confirmed replacement | 1147 | 385408 | 385424 | 16 |

Each16-frame difference is retained FIR input, not measured physical delay.
Exact packet sequence, stored-before offsets, releaseHRESULT0, payload byte
counts and closed cursor/chunk accounting agree. All evidence loss, rejected
release, unobserved packet/frame and pending-abandonment counts are0. Input PCM
retains95520/384000frames. Replacement cursor emitted382880frames agrees with
Process output; requested minus emitted2544 agrees with startup priming.

Reported replacement dropped/silent/nonfinite/discontinuity/timestamp-error,
underrun/reset, backlog/age/ingress recovery and low-water counters are0.
Startup2544priming and160prime-discard frames remain separate. **Timing gate
is not clear:**3delivery gap-bound violations, observed maximum111103*100ns
=11.1103ms, versus admitted522frames=10.875ms. Packet-bound violations are0.
Master reports1native wait timeout; phase is unknown and not investigated.
These observations are not removed, called harmless or attributed to Windows.
They do not by themselves identify an audible corruption or production cause.

Retained queue history covers256of6882events,6626overwritten,0.293919s.
Packet history covers256of800events,544overwritten,2.5496988s. Interpolation
tail covers256of15385events; the separately persisted whole cursor archive
is complete. A clean tail or zero reported steady fault counters is not a
whole-run waveform verdict. All qualification flags remainfalse and physical
latency remainsnull. No larger buffer, weakened bound, repeat-for-green or
lower-default change follows.

App/source/isolatedOBS exit0 without force; recording stop is observed inactive.
The user OBS12648/creation1793505454949 and QQMusic15096/creation1793504036422
remain unchanged. No source, app, isolated OBS, analysis owner or running Moiren
task remains after closure.

## Closed Analysis And Preservation

The remote accounting makes no waveform alignment attempt and does not refit
old recordings. It produces `closed-accounting-v3/candidate-accounting.png`
and scalar JSON, retaining timing warnings and the queue-tail limit visibly.
The actual PNG is inspected. Original MKV/PCM stays remote.

Two analysis-tool failures remain separately preserved: v1 used the acquisition
venv, which lacks matplotlib; v2 correctly used the existing isolated analysis
venv but assumed interpolation events have a `sample` field. V3 changes only
environment selection and optional timing-field inspection, not recording,
counter, quality gate or original evidence. Failed output directories/logs
are not overwritten; no native acquisition is repeated.

All228closed input pins rehash unchanged. Compact ZIP375325bytes, SHA256
`92B7646AF2D58A1D5DA7F9295590934B9E713448E417D7AD9553045D66DCF40E`,
and its7payload entries are length/hash-verified locally. At15:34:22 final
remote audit verifies231input/software/output pins, unchanged app, loaded
driver/boot, render endpoint formats/volume/mute/defaults and bare control bus.
Native task is Ready/result0, no running Moiren task or owned process remains.
Independent final read-only review finds no serious accounting or claim-boundary
issue, confirms both owner totals and timing warnings, and does not run tests,
analysis, native acquisition or network operations.

All27preexisting dirty/cache hashes,13production code pins and unrelated
main-worktree audit files remain unchanged. This increment changes evidence
documentation only; the previous final2436-test production integration is not
misrepresented as a new run. No push occurs.

An independent bounded original-scope audit finds no new actionable S1-S3
software gap independent of physical/desktop acceptance and deferred first
packet work. Retained-prefix conservative refusal is not evidence for adding
a new RT ordering mechanism. Do not manufacture a task or repeat this soak
to keep a goal active. Product work may continue independently; S1-S3 overall
native, waveform, sustained and physical acceptance is not declared complete.
