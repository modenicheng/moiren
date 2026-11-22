# Production CLI Process graph packet acquisition

Date: 2026-11-02, Asia/Shanghai. This completes the missing Process acquisition
after the [earlier UIA attempt](2026-11-02-matched-graph-packet-capture.md)
stalled before Start. It uses the existing production CLI and managed service,
not a new audio owner, GUI automation workaround inside the application,
parameter sweep, stability soak or physical latency qualification.

## Result

The finite32s monitor and source exit0normally, with matching active settings
and post-join cleanup evidence. Task `Moiren-ProcessCLI-20261102-1045` finishes
Ready/result0; `completed=true` means operational acquisition/cleanup only,
while `qualified=false` remains explicit. Both24s Process recorders persist
their full PCM and actual packet histories. The source submits the entire20s
schedule:960000frames,19456nonzero frames and all19indexed-v2 codes.

| Recording | Stored frames | Actual offers | Both-channel identity result |
| --- | ---: | ---: | --- |
| Source tree | 1151520 | 2399 | 19/19, NCC1 |
| Process graph tree | 1151472 | 3427 | 18/19; code6refused |

Both recorders retain zero dropped/silent/overflow/discontinuity/timestamp-error
metadata counters, no omitted packets/frames and no saturation. The strict
framed reader reports complete stored coverage and no packet observations.
Every device position remains unknown; native QPC fields are unverified packet
metadata, not sample-age, epoch, hardware-presentation or physical-delay proof.
Recorder completion alone does not prove an intact waveform.

Graph code6is not accepted on either channel. Its strongest nominal candidate
has NCC0.7176877499237301, onset margin0.01071511902044886,
`sections_match=false`, and zero valid onsets. The existing required onset
margin is0.05; no gate is lowered. There are19nonzero support components in
each recording/channel, but component order does not assign source identities.
The graph's remaining accepted nominal NCC values do not certify whole-run
continuity. SRC transformation, sample mapping and actual damage remain distinct
possibilities; this diagnostic does not establish the refusal's cause.

## Fixed conditions and boundaries

The exact production CLI constructs `Process source -> unity Bus -> sink`
through the same `ManagedDriver` used by desktop. It does not open a window.
This validates that service/CLI route, not the earlier failed native desktop
Start/Stop/Quit automation. The pinned caller PID and creation time are checked
before service preparation; the source stays alive until the monitor joins.

The source renders only to existing Voicemeeter In2
`{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}`. The graph renders to
DAWN PRO2 `{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`.
Source Shared/Float32/stereo/48kHz native storage is1056frames. Requested and
confirmed Engine settings are48kHz/F32/block64; Master Shared output admits
Client3period336and capacity738, no fallback, with render target512,
wake64and callback64. The final prepared render target/capacity are512/738.
Independent Process Compatible input is confirmed as capacity8192,
prime/servo2048, with no declared delivery bounds. These are storage/operating
values, not measured end-to-end latency.

The deterministic source is unchanged from the retained VAD run:20s,
19indexed-v2 codes of1024frames at amplitude0.025, starts24000+48000*i.
Both tree recorders request24s, max1200000frames and32768packet offers.
An800ms post-spawn allowance is explicitly **not recorder readiness evidence**;
this existing recorder has no live ready protocol. Offline source19/19and all
19support components establish retention of this marker set, not arbitrary
startup completeness. Source/graph recorder frame-zero origins remain separate.

The retained VAD graph resolves17/19with codes1and6refused; this Process graph
resolves18/19with code6refused under the same unchanged nominal detector.
Native output tuple, Engine, source schedule and render policy are held fixed,
but VAD/Process source endpoints, native delivery and desktop/CLI entry differ;
acquisitions are sequential, with no matched physical calibration or controlled
CPU-load equivalence. This is bounded route evidence, not a latency or quality
winner. The earlier VAD failed teardown and Process aborted task remain failed.

## Joined-owner counters

The final monitor report schema3confirms32000ms Running, exact requested/active
audio and transport, `run_complete=true`, `cleanup_complete=true`, no failure,
and a matching final joined owner. Process capture records1536000captured and
written frames,1534144output frames,3264priming frames and352prime-discarded
frames. These independently timed quantities are not a frame-loss equation.
Zero dropped/silent/nonfinite/discontinuity/timestamp/underrun/reset and
backlog/age/ingress-recovery counters are retained. Final fill1755,
min1507/max2477and correction42.43051763533781ppm are diagnostics, not physical
latency or calibrated drift.

Native delivery retains3200packets, maximum480frames and maximum interarrival
111562x100ns. Reported packet/gap-bound violations are zero, but Compatible has
no declared delivery bound, so this is not evidence that an explicit candidate
deadline passed. Startup priming is separate; first-packet investigation remains
deferred. No smaller target/default, buffer enlargement or recovery change is
introduced.

## Provenance, lifecycle and verification

Target only: `LAPTOP-NP49RG6H`, limited interactive session1. Deployment
`D:\MoirenTest\20261102-process-cli-1045`; evidence is under `run`.
Task lifecycle is bounded to4minutes and refuses duplicate task/artifact reuse.
Launch10:50:52.501; runner10:51:24.091-10:52:03.803. Native32s duration starts
from matching Running confirmation, not task launch. Held process identity and
exact paths govern cleanup; source is stopped only after graph join. No forced
termination or cleanup error occurs.

```text
Application: clean1267f8d, production audio unchanged by later probe/test/docs
D:\MoirenTest\20261102-vad-dawn-exact336-0812\bin\moiren-app.exe
SHA256 A836C1CF83E977650C7B8ECC4D4A99B24C47569ACF62C42317864F260953B043
Probe: clean1255af2framed packet-history build
SHA256 AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685
Running SYS SHA256
4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73
Runner SHA256
B9801DBEE8D7613C31A89AB29B84744CBE0C41C69D964A58E3F2AB42C8513A31
```

Preflight10:42:10.406confirms no native owner, unchanged protected user OBS/
QQMusic identities, matching pins and181954347008free bytes on D. The source
activation publisher is locally checked for atomic create-new exact byte49and
duplicate refusal. Static review checks32s/24s timing, source identity,
normal cleanup order and fail-closed actual native tuples; PowerShell parsing
passes. A fresh clean-export `cargo test --locked -p moiren-app --lib
cli::tests::monitor_ -- --test-threads=1` passes all4matching tests, covering
exact creation input, Compatible omission, help and independent native period.
This narrow software check is not a workspace-wide rerun.

Only after native exit, isolated remote Python `-I -B` analysis runs
10:53:53.297-10:53:59.750, exits0; analysis itself takes1.922s and charges
213821440correlation samples. Existing environment imports are prefix-checked,
numerical threads are one and plotting cache is output-local. Exact support
window construction,19-code competition and gates0.7/0.05/0.0001are unchanged.
All original input hashes match before/after. The six-panel confidence/identity/
10ms-RMS plot is visually reviewed after transfer.

Raw PCM/history stays remote. Compact outputs are retained under
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-process-cli-compact-1045`;
all manifest entries are checked by exact bytes and SHA256 after download.
Supporting host receipts use `s1-process-cli-*-1045.json`, with preflight in
`s1-process-cli-preflight-1102.json`. Application/source/task exits are reported
separately from waveform/physical refusal. No application/driver code, user
process, default, volume, host audio, original dirty path or memory is changed.

Final10:56:17.057audit confirms task Ready/result0, no native/analysis/runner
owner, unchanged protected process creation identities, defaults/volumes and
the pinned running SYS/application hashes. S1-S3qualification remains open:
neither incomplete nominal waveform correspondence nor missing physical
calibration is replaced by the successful acquisition lifecycle.
