# Process source period and application handoff

Date: 2026-11-03, Asia/Shanghai. Baseline: `e4de0d7` on
`feat/minimal-io-latency`. This is one new-question source preparation check
and a fresh clean-source application package, not another stability soak or
an overall S1-S3 qualification PASS.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-process-source-period-20261103-0410`.

## Software status and scope

The original S1-S3 requirements remain in
`docs/vad-low-latency-goals-2026-10-28.md:205`. Bounded independent audits of
the current S2 preparation/servo/recovery implementation and S3 API/CLI/gpui
configuration/replacement flows identify no new mandatory software gap. This
does not close native waveform, measured-desktop or physical evidence gates.
There is no speculative production fix or new optional framework in this slice.

Recent substantive changes remain:

- `6ec4910`: prepare Compatible physical-master virtual resources using the
  negotiated whole-wake demand, not a provisional demand later contradicted
  by diagnostics. Portable tests cover both precisions and failed preparation
  cleanup; the native smoke is recorded separately.
- `50c97e5`: optional exact Shared source periods for owned VAD and Process
  sources, with Client3 caps/readback and no rounding, conversion or fallback.
- `e4de0d7`: retain the actual source480 VAD experiment, including its running
  anomaly rather than upgrading it to zero-fault waveform evidence.

First-packet investigation stays deferred. The previous VAD running anomaly
is not first-packet behavior, but no reproduced production root cause follows.
Keep it in the evidence backlog without another unchanged scan, larger
buffers, relaxed quality thresholds or a default change. S4 naming and S5
lower-default qualification are separate scope.

## Preparation-only native question

Can the existing Voicemeeter In 2 owned Process source prepare an exact480
Shared period and close normally without Start? The selected render endpoint
is `{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}`. No graph,
Process capture owner, recorder or nonzero source payload is activated.

The remote package is
`D:\MoirenTest\20261103-process-source-period-0410`. It reuses the release
probe from `50c97e5`, SHA256
`6290C3878A6136007DC4551E9DBFCBDF4E8AF92C91D2EFE0467EF58E7F703EDB`.
The reviewed worker and launcher require a matching dry-run selfcheck, exact
machine/root, Limited Interactive session1, fresh paths, isolated environment,
six payload pins/seven deployment pins, protected-process identities and
resource guards. The held-child deadline is45s; the outer task bound is150s.

The existing IPC protocol accepts a preexisting valid stop token: prepare,
publish Ready, consume stop before activate, then join and publish Final.
Preparation still submits silent preroll through ReleaseBuffer. Static review
catches two evidence-runner mistakes before launch: requiring zero submitted
frames and requiring native Stop result=false. The correct unstarted contract
is preroll-only submissions and Stop attempted=false/result=null. The runner
is corrected without changing Rust, playback lifecycle or quality policy.

All37pure runner checks pass on PS7 and PS5 after the correction. An initial
PS5 test invocation with an omitted Base argument fails; its output is retained,
and the successful invocation explicitly supplies Base. Both entrypoints and
the held-child helper parse successfully. Independent final review finds no
remaining blocker before native launch.

Selfcheck worker runs04:14:23-04:14:39 +08:00 and completes its audit. Dry-run
exit1 is the expected unqualified report, not a native acquisition. The one
native worker runs04:15:43-04:15:58. Its child PID19580 runs0.3275412seconds,
exits0 and is joined; both tasks finish Ready/0. There is no retry, timeout,
forced termination, native Start, native Stop or running/submitted receipt.

| Observation | Actual result |
| --- | --- |
| Native Client3 caps | default480, fundamental1, min128, max480 |
| Requested / raw engine / actual period | 480 / 480 / 480 frames at48k |
| Mix and client formats | identical stereo F32, no auto conversion |
| Native capacity | 1056frames, not480 |
| Silent preroll submitted | 1056frames |
| Acknowledged scheduled / nonzero payload | 0 / 0 frames |
| Started / stopped / joined | false / false / true |
| Native Stop attempted / result | false / null |

Ready, Final and joined attempt agree. Defaults, source volume/mute/format,
loaded driver/boot and protected OBS/QQMusic creation identities remain
unchanged. There are no remaining test owners or running Moiren tasks.

All39compact metadata/log artifacts are individually length/SHA256 verified.
The34208-byte archive SHA256 is
`54831B2DC23CDCA9E78F10E28D039716BF720817D580F0E1A5C48E50AECC4CE9`.
No raw PCM or scalar stream is transferred. This result proves preparation
and closure only. It is not a new Process waveform, matched-route comparison,
streaming-quality, ten-millisecond latency or physical result.

## Fresh software verification

The tested export is reverified against all1383tracked code/source files in
the committed `50c97e5` archive, excluding only its separately updated validation
document: zero mismatches. Production code also matches `e4de0d7`; the latter
changes only the validation document. All27original dirty/cache file hashes
remain unchanged and outside build/commit scope.

| Fresh check | Result |
| --- | --- |
| Probe with `probe-tools,vad-probe` | 733passed,1ignored,0failed |
| `moiren-app --features gpui-test measured_ui_tests` | 20passed,0failed |
| Windows audio library `zero_refill` | 5passed,0failed |
| Marker / rate / qualification analyzers | 19+8+24passed,0failed |
| `cargo build --release --locked -p moiren-app` | Exit0 |

The initial PowerShell aggregation of the last two invocations accidentally
includes printed result lines and reports an aggregation failure; both Cargo
exit codes are actually0. The original mixed record and corrected two-result
record are retained. No production or test threshold is changed. These are
portable/headless tests and compilation, not host native audio/UI launches.
Full workspace/check/strict Clippy/fmt results for the unchanged code are in
the preceding exact-period validation; they are not claimed as rerun here.

## Application available for manual remote testing

Fresh clean-source release package, deployed and remotely length/hash verified:

```text
root:     D:\MoirenTest\20261103-app-e4de0d7
app:      bin\moiren-app.exe
launcher: run-isolated.cmd
notes:    README.txt
manifest: manifest.json
```

The application is34150400bytes, SHA256
`1217C07CAB24BAFF8B57E6E7BC5536C82B4F9A7F913892389B0B50F922D0B128`.
It includes `353f0db` SRC zero-refill and `6ec4910` Compatible whole-wake fixes.
It is not a build containing the preserved uncommitted overlays. No driver
installation/update is needed for these software-only changes. The existing
loaded remote driver remains Running with SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`.

The launcher checks exact machine/root/binary, rejects an existing app/probe,
and gives each manual invocation fresh TEMP and app-data directories under
`runs\<UTC>-<id>`. Logs are under that run's
`appdata\Moiren\logs\moiren.log`. It does not change the original user profile,
install a driver, terminate processes, set default devices or start music/OBS.
Its normal first-launch application may automatically start the default
session; it is explicitly not a stopped-only fixture. Stop via the UI before
editing a different route. Final review checks these boundaries and PS5 syntax.

Deployment does not launch the application: no runs directory exists at the
04:21:18 remote verification. There are no app/probe owners or running test
tasks, and protected OBS/QQMusic remain unchanged. No new native desktop
acceptance follows from preparing a usable binary.

For user testing, use Process -> unity Bus -> chosen output, then VAD -> unity
Bus -> the same output separately. Keep rate/format/output fixed, verify
confirmed active rather than draft settings, and exercise Stop/Start/Apply/Quit.
Retain separate OBS reference/output tracks if available; a mixed track does
not supply independent timing. Check waveform integrity before delay, and keep
startup separate. Missing calibrated physical capture hardware does not block
independent application development. Future long tests stay finite, remote,
isolated and parallel to development only when answering a specific new question.
