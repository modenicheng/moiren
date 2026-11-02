# Matched requested graph conditions with actual native packet histories

Date: 2026-11-02, Asia/Shanghai. This is one attempted sequential VAD/Process
feature pair, not a stability scan, default promotion or calibrated physical test.
The source-only [packet roundtrip](2026-11-02-capture-history-native-roundtrip.md)
does not substitute for these graph recordings.

## Software regression and development status

Fresh clean-export regressions finish normally: zero-refill5passed,
marker19/rate8/qualification24passed, and measured UI20passed. These are
hardware-free checks. The source export excludes the18preexisting dirty paths;
the matching release probe is the framed-history build from `1255af2`.

Source review finds the original S2/S3 functionality implemented: independent
VAD/Process budgets, bounded recovery admission, exact/custom and measured
Process requests, prepared-owner replacement and requested/active confirmation
in the production desktop and monitor CLI. This does not close native
waveform quality, lower-setting acceptance or physical calibration. First-packet
investigation remains deferred; none of those evidence gaps justifies repeated
unchanged stability scans or new speculative measurement infrastructure.

## Fixed requested conditions and provenance

- Target only: `LAPTOP-NP49RG6H`, SSH `192.168.12.97`; no host audio/UI/device
  operation. Protected user OBS/QQMusic processes remain external.
- Graph: source -> unity Bus -> the same DAWN PRO2 Master render endpoint
  `{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`.
- Engine48kHz/F32/block64. Shared output requests336frames, admits336frames
  with native capacity738; explicit render target512/wake64/callback64.
- VAD and Process policies independently remain Compatible; recovery cap is
  default8. No threshold, production queue, default or recovery policy changes.
- Same20s indexed-v2 source:960000frames,19codes of1024frames at amplitude
  0.025, starts24000+48000*i for i0..18. Source uses latency-sensitive mode,
  shared Float32/stereo/48kHz and native1056-frame storage.
- Each route plans separate24s source-tree and graph-tree Process recordings,
  max1200000frames and32768actual packet offers per recorder. Only VAD completes
  acquisition. The recordings' frame-zero origins are independent; no cross-
  recording subtraction is physical delay.
- VAD routes the source to its owned Playback MMDevice. Process instead routes
  it to existing Voicemeeter In2
  `{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}` and pins PID plus
  creation time. This unavoidable source-routing/native-cadence difference is
  not a calibrated identical-boundary comparison or a path winner.

The known-clean application comes from source `1267f8d`; later changes touch
probe/tests/docs, not production audio. Its executable is reused without edits:

```text
App: D:\MoirenTest\20261102-vad-dawn-exact336-0812\bin\moiren-app.exe
SHA256 A836C1CF83E977650C7B8ECC4D4A99B24C47569ACF62C42317864F260953B043
Probe SHA256 AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685
SYS SHA256 4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73
```

## VAD: complete acquisition, failed teardown harness

Deployment `D:\MoirenTest\20261102-matched-graph-1020`; task
`Moiren-MatchedGraph-vad-20261102-1020`, launched10:22:35.344.
Runner10:23:06.574-10:25:05.096; terminal Ready/result1, `Completed=false`.
Do not replace this receipt with a success or rerun the acquired audio merely
to obtain a green task.

Both24s recordings complete and persist with full actual packet histories:

| Capture | Stored frames | Actual offers | Native flags/loss/storage |
| --- | ---: | ---: | --- |
| Source tree | 1151040 | 2398 | zero dropped/silent/overflow/discontinuity/timestamp errors; no omission/saturation |
| Graph tree | 1151136 | 3426 | zero dropped/silent/overflow/discontinuity/timestamp errors; no omission/saturation |

These are recorder metadata, not proof of whole graph continuity. In particular,
intentional source silence and positionless Process packets cannot expose every
loss. Both recorders exit1under their intentional unqualified contract with no
recording failures. The source submits all960000schedule frames and19456nonzero
frames, joins and exits0normally. Source submission is not presentation proof.

The harness's new stopped-state gate incorrectly requires an `Idle` UIA label.
The retained accessibility tree exposes enabled `Start` but no semantic `Idle`
badge. Actual app logs show cleanup finished and owner reaped `cleaning=0` at
10:24:28.421. The gate nevertheless expires; fallback WM_CLOSE does not exit
the application, and the exact held app is force-terminated with exit-1.
The receipt remains failed, with the later GPUI close errors retained rather
than reclassified as a native audio failure or normal Quit.

All owned processes are gone, owned VAD endpoint removed, protected user
process identities/default roles/render volumes unchanged. The original runner
is immutable, SHA256
`7A870624B3256700DEB5C9EB4916489037FCBEA5A3DB7599E90F7ED30F56CBB1`.
The Process deployment uses a new runner requiring enabled Start plus actual
cleanup-finished and owner-reaped logs, not a nonexistent accessibility label.
Its intended teardown stops/joins Process capture before releasing its exact source target;
VAD continues to release the source client before endpoint teardown. Token
publication failure cannot skip the held-process join/kill fallback.

## Process: aborted before graph Start, no matching acquisition

The separate deployment is `D:\MoirenTest\20261102-matched-process-1028`;
task `Moiren-MatchedGraph-process-20261102-1028` launches10:27:39.018.
Runner SHA256
`BFBA7B23C9921CC345083A3D89B1AD38193B742F91936F36CBD5FE11EFB13C36`.
The source reaches ready with PID24032and the restored project contains its
exact creation identity. The app PID20164starts10:28:12.380. Saved UIA state
shows the intended Process source/Bus/DAWN graph and enabled Start, but the
runner makes no recorded Start invocation and no native audio owner starts.
The observation/control request does not return; the precise stuck UIA call is
not established by this receipt. This is a harness failure before acquisition,
not evidence of a Process audio-path defect.

The source expires its120s finite unactivated lifetime and exits with
`process source finite lifetime exhausted`. There are no recorders, packet
histories or Process graph PCM, so there is no completed A/B result. After
verifying exact executable, PID and creation time, the test app is terminated;
the still-held runner is stopped through its exact scheduled task after the
app/source are gone. Terminal Ready/result267014records task termination,
not a clean script exit. No normal result.json exists. An explicit external
cleanup receipt retains the intervention rather than fabricating task success.
No repeat acquisition is launched.

## VAD offline result and visualization

Only after all native owners exit, one isolated remote Python `-I -B` analysis
runs10:33:39.169-10:33:46.152, exit0. Existing environment imports are checked
against its prefix; numerical worker count is one, plotting cache is output-
local. The analysis itself takes2.797s and charges537744384correlation samples.
Original input hashes are identical before/after. Actual framed packet evidence
schema2reports complete stored coverage, no observations/startup issues and
unknown device position for every offer; it does not supply a native epoch.

All19original codes compete independently in every support window. Exact
nonzero components are split only beyond32consecutive zeros and padded64frames;
no schedule labels are assigned by peak order. Existing gates remain NCC0.7,
peak margin0.05and RMS0.0001, including the unchanged section/onset checks.

| Recording, both channels | Support windows | Accepted identities | Refusals |
| --- | ---: | ---: | --- |
| Source tree | 19 | 19/19, NCC1 | none |
| VAD graph tree | 19 | 17/19 | codes1and6not accepted by section checks |

The refused graph windows contain signal; their strongest nominal candidates
have channel0NCC0.762079and0.740310but `sections_match=false` and no valid onset.
Channel1retains the same refusals. They are not accepted by reducing a gate,
renumbered from order or declared packet loss. SRC transformation, recorder
mapping and real waveform damage remain distinct unresolved possibilities;
this nominal-template diagnostic does not establish their cause. Accepted graph
NCC values likewise do not certify arbitrary-audio continuity or calibrated
physical delay. All reports retain `qualified=false`.

The six-panel `marker-identities.png` compares confidence/refusals, accepted
identity indices and10ms RMS, with independent recorder origins explicitly
marked. It is visually reviewed after downloading. The prior failed VAD task
and aborted Process task remain failed despite successful offline analysis.

## Evidence retention

Raw PCM and packet files remain on the test target; only compact reports,
plots, manifests and receipts are transferred and hash-checked. Host receipts
live under `D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
No source/recording origin, native epoch or physical p50/p95 is fabricated.

Compact output is in `s1-matched-vad-compact`; every manifest entry is verified
by exact byte length and SHA256 after transfer. Supporting host receipts include
`s1-matched-vad-final-status-1102.json`, `s1-matched-vad-inputs-1102.json`,
`s1-matched-vad-analysis-receipt-1102.json`,
`s1-matched-vad-download-audit-1102.json`,
`s1-matched-process-hang-1102.json`, and
`s1-matched-process-aborted-final-1102.json`. The external analyzer review fixes
only duplicate-identity accounting and structured metadata-refusal reporting;
it changes no detector, input PCM, gate or native audio policy.

Final remote audit10:36:21.419confirms no native/analysis/runner owner, both
tasks terminal, protected OBS/QQMusic creation identities unchanged, unchanged
default roles/render volumes, and the same running SYS/application hashes.
The18original dirty file hashes remain independently checked; no pycache,
production code, host audio, user process or memory file is changed.
