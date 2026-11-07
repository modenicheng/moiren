# Target1280 Native Render Timeline Acceptance

Date: 2026-11-03, Asia/Shanghai. Production source: `002aa21`.

## Question And Isolation

One distinct post-instrumentation acquisition asks whether both production
owners produce complete, mutually reconcilable encoded PCM packet and native
operation histories. It does not retry the old waveform association, scan
targets, investigate first packets or qualify physical latency. A successful
archive receipt is not a continuous-waveform or audio-quality result.

The render timeline implementation and software checks are documented in
`2026-11-03-render-timeline.md`. Package preparation runs in parallel with
development. All native source, app and isolated OBS activity is on
`LAPTOP-NP49RG6H` over `ssh moiren-lan`; no host audio is started or stopped.
External user OBS12648/creation1793505454949 and QQMusic15096/creation1793504036422
remain protected. Only held test children may be closed.

Local root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-render-timeline-20261103-1618`.
Remote root: `D:\MoirenTest\20261103-target1280-timeline-1618`.

## Frozen Build And Configuration

- Clean-source app34350080bytes, SHA256
  `B56B0AECD297A8BC708CAC4878D97FD6D7040184D09CCD751CB151A4BDAF3E75`.
  Exact12code pins match `002aa21`; preexisting unrelated changes are excluded.
- Flat package12719762bytes, SHA256
  `6D3E573A7982F1863F5B6BDBB40F2475B56A8FD7DF09EBF3BD710F207B087B7E`.
  Reviewed intent SHA256
  `91D341D9A1F254009C228D9A072FBF1C73920FCF899805E336A9B1F407EB7DF6`.
- Process -> unity Bus -> DAWN PRO2, stereo48k/f32, Engine64. Shared336,
  capacity738; render target512/wake64/callback64. Compatible measurement2s,
  confirmed target1280/capacity16384/jitter96 for8s. No default changes.
- Source probe reused unchanged: Shared480/capacity1056, indexed-v2 changing
  signal, amplitude0.005,40contiguous24000-frame codes/960000frames/20s.
  Cursor PCM/render capacity65536; separate preallocated4096-event ring.
- Isolated portable OBS records DAWN stereo48k PCM24 MKV. It is a software
  capture boundary, not a calibrated DAC/acoustic measurement. No waveform
  fitting, resampling or alignment search is performed for this receipt.

Finalized package56synthetic Python tests pass. Wrapper checks29each on PS5
and PS7 pass, with synthetic children only. Deployment rehashes payload and app
pins, copies only the isolated Python environment and runs device-free tests.
Final selfcheck is Ready/result0, native_launched=false and bound to the exact
reviewed intent. Only then is the native task launched once.

The case/held-Python/outer-task deadlines remain125/170/420seconds. Launch
attempt markers and tasks are create-new; ambiguous outcomes require inspection,
not relaunch. Native task `Moiren-Target1280-Timeline-20261103-1618-native` is
terminal Ready/result0 at17:04:20 with complete=true and no stderr/audit error.

## Actual Observation

Measurement is2000ms, confirmed operation8002ms. App generation2/revision2
confirms target1280; source-generation1 and actual prepared/admitted consumer
demand528 match the frozen request. Empirical delivery bound is480packet frames
and527gap frames; modeled minimum1153, low-water530, independent capacity16384.

| Owner Ordinal | Events | Linked Packets | Encoded Frames | Engine Requested |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 582 | 290 | 97616 | 97632 |
| Replacement | 2296 | 1147 | 385568 | 385584 |

Both local owner-generation values are1; directory ordinal distinguishes
baseline and replacement. Both histories contain one Start, alternating
Write/Wait operations and final Stop. Every packet release has HRESULT0 and
exactly one matching Write with correct frame count, HRESULT and accepted-frame
frontier. Event sequences are contiguous; missing, dropped, reader-abandoned,
pending and saturated-event counts are zero. Missing/regressed QPC counts,
operation failures, zero writes and Wait timeouts are all zero in this run.

The16-frame difference between Engine demand and encoded submission exists for
each owner; it is retained as separate-boundary accounting, not called physical
sample loss or latency. Operation QPC envelopes include method work, not just
the COM call, and padding is only the actual existing write-side query.

Reported replacement Process drops, steady silence, underruns, resets,
discontinuities and timestamp errors are zero. Nevertheless, there are **2gap
bound violations**, max111165*100ns=11.1165ms against527frames/10.9792ms. This
does not satisfy the declared delivery envelope. No threshold is widened,
counter erased or candidate/default promoted. Storage/submission acceptance
does not override that observation or establish continuous waveform quality.

At app join the source-completion receipt does not yet exist; this is not a
direct observation of continuous submission at that instant. The source later
reports all960000frames submitted. Held graph24736, source25220
and isolated OBS18640 all exit0 without forced cleanup. Driver/settings and
external user-process protection are checked separately, not inferred from
child exit. No second native attempt follows.

Startup/prime accounting is separate: priming3216frames and prime discard160
frames are retained, not called zero total loss. The monitor's latest-only
producer/consumer histories overwrite543/891events; this is bounded-history
coverage, not lost audio frames. Completeness claims here refer specifically
to the independent render operation/packet and cursor archives, not every
rolling monitor history or the recorded output waveform.

## Closed Accounting And Artifacts

A separate bounded offline audit reopens both packet/event ledgers and confirms
exact correspondence, immutable source pins and replacement Wait-timeout tally
against the terminal backend. It creates `closed-timeline` once, does not repair
files, and reports complete=true, qualified=false, physical_latency_ms=null.
Its229input pins rehash unchanged. The launch wrapper observed successful wait
and complete stdout but returned a null Process.ExitCode field; this field is
not fabricated as zero. Native scheduled-task result0 and the immutable closed
audit receipt provide distinct observed closure evidence.

The compact native/monitor/closed receipts are under local `native-compact`;
full MKV, encoded PCM, packet/event JSONL and source PCM stay remote.

## Visualization

A single offline plot runs in the existing isolated analysis environment,
without installations or audio/OBS/UI activity. It independently revalidates
234input pins before/after and uses each owner's own Start-relative QPC origin.
Six inspected panels show actual write padding, Write-before minus prior
Write-after service gaps, Wait envelopes and accepted-submission frontier.
All events remain in the denominator. Neither owner is spliced into the other,
and no waveform model, lag search, resampling or latency estimate is used.

Both padding histories range0..176frames. Baseline/replacement maximum Write
service gaps are9.5888/7.7519ms; maximum Wait envelopes9.2697/7.6519ms. These
are method-level observations, not inferred native underruns or physical E2E.
The image explicitly rejects an audio-quality/continuity verdict. A plot of
accepted frames proves only the observed submission frontier.

Local visualization:
`timeline-visualize-compact\timeline.png`, with `summary.json` and `pins.json`.
The319705-byte compact ZIP has SHA256
`B6508C2487D628BAD78748ABFA6CDEA20492670EFF4DA2925DC06962918A35D5`;
the five-entry ZIP and its four declared payload pins are verified after
transfer. Offline process26232 is
confirmed closed; native task remains Ready/result0 and protected user-process
identities are unchanged. Raw recordings are not transferred.

## Initial Final Audit Limitation

The final independent comprehensive environment/pin audit cannot be claimed
complete. Its SSH transport times out; a17:15:25read-only check sees neither
the intended remote script nor its receipt. At17:15:52a related remote
PowerShell process26688 is observed, but its subsequent state is not verified.
No final-audit exit0, fresh all-pins pass or final cleanup is inferred from
that ambiguous transport. Local tool calls are terminal; no additional audit,
network diagnosis, audio run or process-name cleanup is attempted.

Earlier distinct evidence remains: native task Ready/result0 with before/after
driver/device/settings/protected-owner checks;229closed accounting pins
unchanged;234visualization input pins unchanged; offline26232closed and native
task Ready/result0 with user process identities unchanged after plotting. These
do not replace the missing final comprehensive audit. On next remote access,
inspect26688and create-once receipt/script state before taking further action;
never blindly relaunch or terminate an unrelated process.

### Bounded Recovery Without Another Acquisition

A17:26:45read-only query confirms26688still has its original creation time
1793697230303ms and exact audit-transfer command. The decoded command is blocked
on `Console.In.ReadToEnd`; neither intended script nor receipt exists. No audio
or analysis owner or running Moiren task exists, and protected user identities
match. The original timeout is retained as a failed transport, not a native
audio failure or successful audit.

At17:28:41the process is closed through its held handle only after rechecking
that creation identity and exact command. No other process is modified. The
original script is transferred through SCP, rehashed and executed once without
another audio/OBS/UI run. The17:29:35..17:29:52audit exits0, as does its SSH
transport; the create-once4154-byte receipt SHA256 is
`EEB9F64D090E63D0C37635B5DBE822D0323E14AA0010E42A9C58F450BB7B94C4`.

The audit verifies498pin references across257unique files, including229closed
accounting input pins,234visualization input pins, package/software/output pins
and the deployed app. Original default roles, render settings, driver/boot and
protected OBS/QQMusic identities remain unchanged. Native task is Ready/result0;
no native/analysis owner or running Moiren task remains. The earlier closed-audit
wrapper's null Process.ExitCode stays null; this new observed audit exit does
not retroactively turn that field into0. No raw recording is transferred.

Local receipts include `timeline-final-audit-recovery-shortstate-20261103-172636.json`,
`timeline-final-audit-aborted-transport-local.json`, `timeline-final-audit.json`
and `timeline-final-audit-completion-summary.json` under the evidence root.
The27preexisting dirty/cache pins and unrelated main-worktree audit files remain
preserved. No push. No old packet855 coordinate or prior lag/gain model is
applied to this recording. Physical latency remains unknown and S1-S3 remain open.
