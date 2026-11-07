# Shared VAD interactive control - 2026-10-31

## Scope and outcome

The v3 control retains nonzero Process and provider PCM, independently audited
75/75 PASS. This is bounded software-boundary evidence, not S1-S3 acceptance.
An earlier unrelated build/test workload under `D:\moiren-two-tier` was preserved,
not killed. Root reports it completed at10:20:36, before requesting v3.

Playback / virtual speaker remains Moiren Input / Source; MIC / virtual
microphone remains Output / Sink. The retained holder is
`Playback -> Gain(0.5) -> MIC`, not the required S1 Input/Bus waveform route.
The control uses an exact owned Shared Playback source and a separate
Process recorder in the verified interactive console, without physical output,
volume/default/privacy writes, driver installation or changed audio budgets.

Host evidence: `D:\coding\moiren-evidence\20261031-shared-vad-interactive-control`.
All timestamps below are UTC from retained receipts, not inferred audio times.

## v1: refused before task registration

`native-launch.stderr.log` retains `Expected Active console absent/unknown`.
The 09:41:51 `launcher-refusal-audit.json` has no task, run directory, launch
receipt or owned process. Raw `quser` contains an English Active console row
whose username includes a space. The original parser split that name as fields.
This is a launcher parsing refusal, not an audio failure. Original scripts and
receipts remain in `v1-preserved/preservation.json`. Original-parser RED16/8,
repaired parser24/0 and inherited contracts63/0 are retained; a username never
substitutes for exact SID/session/token authorization.

## v2: task ran, localized preflight refused

`v2/native-launch.json` records registration/start request for
`Moiren-SharedVAD-interactive1` at09:46:35.9661035. This is a scheduled-task
request only. At09:48:41, `native-poll4.json` records Ready/result1, no run
directory, no Running/result receipt and no owned helper/audio processes.
The task/security audit retains InteractiveToken/Limited and the expected SID
and session1. Scheduling delay is not an audio hang or acquisition duration.

The first context diagnostic also fails and is preserved. The decisive
`context2-readonly.json` (10:05:10--10:05:12) records session1,
expected SID/Explorer owner, medium integrity and `NativeStarted=false`.
The raw task-context `quser` state is localized Chinese running; both extracted
v2 parsers require English `Active` and refuse `Expected one exact Active console
session; absent/duplicate/unknown refused`. The spaced name is now one column.

This localizes the refusal to preflight parsing, not Windows audio policy.
Original v1/v2 tasks, scripts, launch results and raw diagnostic receipts remain.

## v3: reviewed, staged and task requested

Task/run names become `Moiren-SharedVAD-interactive2` / `shared-vad-interactive2`;
v2 is not modified/relaunched. v3 accepts only exact English Active or observed
Chinese running, retains raw state/row and refuses unknown/ambiguous rows.
SID/Explorer/session/token, identity/freshness/gate/revision, overlap and cleanup
guards remain unchanged. Outer logging does not bypass execution policy.

`v3/independent-v3-blocking-review.json` finds no P1/P2 blocker for the exact
frozen version. Its local inert fixtures pass120/0: parser24, inherited
contracts63, localization23 and outer-command10. The actual v2 localization
RED19/4 is retained. These are helper tests, not native signal evidence.

`v3/runner-freeze-v3.json` and stage receipts pin exact helpers. The10:11:19
preflight records external cargo/rustc activity and no v3 task: acquisition
waits. After quiescence, `v3/native-launch.json` records a matching-hash task
request at10:20:49.2679428, not script/native startup. Scheduling delay is
distinct from execution, acquisition and cleanup.

## Frozen experiment and qualification limits

The existing holder is revision `b0a0223`, source `b6b26b7`, recorder `0fd3fbf`;
full revisions, lengths and hashes are pinned in `v3/runner-freeze-v3.json`.
Driver610 remains source `883a5be`, installed on-disk SYS SHA256
`94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`.
The holder requests480000 raw frames during its60s `no_clients_timeline_hold`
fixture (zero probe-owned native clients; the external source is separate);
source is Shared Float32/stereo48kHz,35s ceiling, existing20-marker schedule and
write witness. The6s Process recorder pins PID **and creation time**. Repeated
v1 markers are not uniquely indexed continuity evidence.

## Acquired execution and archive

`shared-vad-control-verified/runs/shared-vad-interactive2/result.json` records
actual script start10:22:49.8543252 and finish10:23:56.0229552, distinct from the
task request. Before/after observations retain session1, expected SID/Explorer
owner, Interactive/Limited, medium integrity and non-admin token. These are
observer snapshots, not a complete transition history.

Collector exit0 retains `Errors=[]` and completed source recording. Holder and
source exit0; recorder exit1 is its retained expected unqualified contract.
All three exits are confirmed without forced termination; no remaining owned
processes are reported. Holder reports service joined/control session released,
null primary/cleanup errors, but `endpoint_absence_confirmed=false` remains.
Driver on-disk hash/Running state and boot are unchanged, root PnP is OK.

`shared-vad-control-evidence.zip` is598275 bytes,84 payloads plus manifest,
SHA256 `A9AF00F144D17895B079753D9BD53F5447B5CBC034C11125A1823905ACC3FDFE`.
The22054-byte manifest SHA256 is
`D9C887C70762424B13E8F3ADC18C8E847A4FBA9778D12667A598264778A87281`.
Root verification records `present_unqualified`; ZIP rehash agrees. Separate
`independent-shared-vad-control-audit.json` verifies all85 members, exact set,
CRCs and archived/extracted hashes, not PCM quality or endpoint absence.

## Independently verified PCM

`root-pcm-morphology.json` and
`independent-shared-vad-interactive2-acquired-audit.json` agree on actual formats:

| Boundary | Format | Stored frames | Nonzero samples L / R | Peak |
| --- | --- | ---: | ---: | ---: |
| Exact source Process recorder | Float32/stereo48kHz | 287520 | 286016 / 286016 | 0.02500000037 |
| Accepted provider READ before decode | PCM16/stereo48kHz | 332640 | 331553 / 331555 | 0.02505493164 normalized |

Process PCM is2300160 bytes, SHA256
`604033DF4CF2D40B5259177AE176B17AEFD7D9D98CA0CB559EFE527591343131`;
both channels have zero nonfinite samples. Metadata retains599 packets, zero
silent/dropped frames and overflow/discontinuity/timestamp-error counts.
The1504 leading all-zero stereo frames are actual samples, not SILENT flags;
`silent_frames=0` is not a sample-nonzero test. Both stored channels are equal.
Position-gap detection is disabled; complete continuity is not established.
The unconsumed compatibility bridge separately retains287520 dropped frames;
this is not the native artifact's drop count or an end-to-end quality pass.
The recorder retains unknown `buffer_frames=null` despite an unusable raw
GetBufferSize result; its reported number is not an accepted native capacity.

Provider PCM is1330560 bytes, SHA256
`7C8866977CAB60AEF2B558EB585594F1868C4081664D63CAA4AA77F3A50BA61D`.
Its693 packets bind native stamp `[17,1171229636140,17,5,13]`, admission/revision2,
format1, two channels and480 frames/packet. **Wire format1 is PCM16**, per
`capture.identity.format` and `driver/vad/include/moiren_vad.h`, not Float32.
The initial Float32 interpretation is invalid/superseded, with no corruption
inference. Capture seals `identity_changed` after332640 of requested480000 frames,
worker joined/completed: partial single-identity evidence, not MIC output.

Source confirms333120 successfully submitted nonzero frames plus1056 silent
preroll frames; the20-marker schedule is not exhausted. The16-byte Float32
encoded/destination witness agrees (`CD CC CC BC` repeated), Release flags0 /
HRESULT0. This prefix/submission proves neither all playback nor delay.
The6s recorder's exact source PID4220/creation134379157788794470 matches source
receipts; recorder join precedes source Stop within the unexhausted schedule.
Provider sequences1--693, byte coverage and positive increasing QPC labels agree;
two stored packet labels follow Stop-token publication. Labels/lifecycle overlap
are not observer arrival times, sample-origin mapping or cross-stream latency.
Provider WASAPI flags and Process per-packet timestamp/position logs are absent.

Independent audit SHA256 is
`76700FFBBC9C9EF8E15D198F5F2AB6C80CE387B790133B943F2782F7BDC302F5`.
All75 integrity/identity/PCM/overlap checks pass, with no findings, while
`qualified=false` and `acceptance=false`. Small provider channel differences do
not identify a conversion/dither mechanism. Neither captured boundary is the
post-Gain MIC output; holder `pcm_verified=false` remains unchanged.

## Post-control audit and retained limits

`root-post-control-readonly.json` passes all guards at10:33:13--10:33:19.
The post-control catalog matches the pinned earlier baseline's six default roles
and12 render master volume/mute values. This supplements the collector's zero
write counters, not continuous/all-path settings evidence; the single source
client master1/unmuted snapshot is also point-in-time only. Fresh root audit
retains no owned processes, boot unchanged and driver610 Running. On-disk hashes
are not in-memory image attestations.

`root-post-control-cleanup.json` finishes10:33:38 with all guards passed,
`Error=null` and all four exact terminal tasks in `Removed`. Original failed
task/launch/context receipts remain; removal does not rewrite their outcomes.
No process kill, audio rerun, settings/default write or driver change follows.

Historical binaries/driver conditions differ: this is not a matched session-only
causal A/B. Historical Shared all-zero capture's root cause remains unproved;
neither launcher refusal was an audio result. No physical latency, sub10ms
feasibility, minimum buffer, S1-S3 completion, quality waiver or default
promotion follows. The goal remains incomplete.

Separately, retained `20261031-latest7023-deployment/ci7023-final.json` verifies
head `7023b9a212e46d67bc3ff95ff76cdf6cd2e1f27b`, run41188651758, all20 jobs
successful, updated10:02:17. Separately, PR52 head `7e48c7f` CI run41191570851
completes successfully (19 successful jobs/one skipped), and
`20261031-wasapi-exclusive-capture-extraction/pr52-merge-receipt.json` confirms
squash merge `74ef5ddb82c0edffaafa1fa883f49d485cb73447`. This mainline extraction
does not change the control's already frozen binaries or acquisition. Neither
CI nor the merge qualifies this acquisition. Defaults, admission/quality gates
and canonical STATUS are unchanged by this receipt.
