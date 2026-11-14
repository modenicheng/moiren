# Direct remote waveform checkpoint

Date: 2026-11-07, Asia/Shanghai. Acquisition source: clean `b08f61b`,
tree `0c184d3ee4068d201c3dd791c480c5b1e7411bc2`.
This continues the full S0-S5 pipeline. It does not qualify a route, a lower
default, Windows naming, desktop input or calibrated physical latency.

The standalone `moiren-lan` target is used directly. No VM, unlock, visible
desktop input, endpoint enable/reset, identity rotation, new rename mode,
driver installation or development-host native operation occurs.

## Scope, provenance and admission

Local evidence:
`D:\coding\moiren-evidence\20261107-matched-waveform-2213`.
Remote evidence:
`D:\MoirenTest\20261107-matched-waveform-2213`.

Both current release builds finish successfully. The probe's software-only
example suite passes 741 tests with one ignored. Immutable acquisition copies:

| Binary | Bytes | SHA256 |
| --- | ---: | --- |
| `moiren-app.exe` | 35252736 | `621892064ABF681C641F9A4A5031B17FAB7ED80B8C9AF2AED01486940D43CFEA` |
| `e2e_latency_probe.exe` | 8179200 | `6468E8F1F6BB241085D8854A9F7862C46BA2B858909CF359FBA9690E731338ED` |

The source schedule remains six 1024-frame codes at source frames
12000/60000/108000/156000/204000/252000 of 288000, 48 kHz stereo F32.
Source Shared480/native capacity1056 and physical DAWN output
Shared336/capacity738/target512/wake64/callback64, Engine F32/block64 and
the unity `Input -> Bus -> Output` route are unchanged.

The native isolated environment's 33 non-pyc files retain fingerprint
`CD24FA8C556890903C8D023CFB6F681E04DC0281631FAC3437D08F4893B5CC95`.
The protected QQMusic15096 and PowerShell27444 identities remain distinct
from historical OBS12648, which is absent. The old manifest is not rewritten
to claim that an obsolete guard passed; this is a new evidence scope.

The new eight-file native intent has SHA256
`F21716184B7E28747E5AABDF22C860B73FB8717C3C20D68875957965E60D1815`.
Transferred pins and a Limited/session1 no-audio selfcheck verify before
native execution; the selfcheck task ends Ready/result0 with no owners.

The host controller requires successful matching before audit, one native
launch attempt, observation of the same actually-run terminal task with no
owners, then an after audit. Failed/unknown audit cannot accept acquisition.
Missing or newly registered unrun tasks are not treated as terminal after
a transport timeout. Observe-only recovery is diagnostic and cannot erase a
launch refusal. Twelve pure regressions pass after recorded failures.
Both launch and entry require a matching before receipt aged 0..120 seconds;
nine receipt checks pass separately under PS5 and PS7.

The read-only safety audit extracts definitions only from the pinned prior
deployer; its installation entrypoint is never executed. It verifies the
loaded 3.1.1.612 SYS, installed INF, root PnP health, boot, read tools, all six
default roles, render controls, protected identities and security state.

## Actual acquisition: data retained, pair stops on cleanup failure

The native task starts once at 22:31:31. It ends Ready/result1 with no owners.
The Process source releases its complete schedule and stops/joins normally.
The monitor exits0 with graph and cleanup complete. Independent Process
reference recording reports complete/persisted without recording failures;
its unqualified probe exit1 is retained.

OBS reaches recording active, dispatches Stop and observes inactive. Its
closed MKV is 9709541 bytes, SHA256
`3D25C80996BC294E7329498B3C16B310A9DEE6814856C5A67549256A255C695F`.
However, normal owned-window closure does not join within the existing 12s.
Only the held OBS child is then terminated; forced=true/exit1 remains failure.
The sequential pair stops before VAD and is not rerun.

Recording Stop occurs at 22:31:50.043, websocket module shutdown at
22:31:50.252 and closing-handshake timeout at 22:31:55.253. The selected runner
still holds its client socket while waiting for OBS to exit.

This repeats the already documented runner defect in the
[November3 checkpoint](2026-11-03-shared-waveform-recordings.md), rather than
establishing a new product or Windows defect. Selecting the older `0430`
runner overlooked its later `0505` correction. Recover the established
`s1-matched-waveform-ready-20261103-0505/run-close-v2.py`, SHA256
`DBB3515EAA8744F3BE7E2991149BE561F17BCE273FD771701731724FF85D6B59`.
Its exact `acquire.finally` correction closes the socket after confirmed
inactivity and before normal window close/join; unresolved recording retains
ownership. Two reproduced regression failures become 15 passing checks, with
an AST check limiting the change to that block. This corrected local runner
is not deployed or used for another acquisition here.

Full native before/after safety audits both pass, including unchanged
driver/INF/boot, defaults, render controls, security and protected identities.
Successful cleanup auditing does not turn the route's forced OBS exit into
a lifecycle pass.

## Offline analysis of the actual closed media

The first offline selftest entry fails because PS5 treats normal unittest
stderr progress as a terminating error under its native invocation form.
A separately pinned entry redirects through a held child process and retains
the real exit code. Successful stderr and failing exit3 checks pass under
both PS5 and PS7. The original task/receipt remains failed and unchanged.
Child timeout is not proof that a venv interpreter's entire process tree
has joined; final task and owner observations are authoritative.

Before analyzing media, the canonical documentation also identifies that the
older 22-test analyzer mishandles integer Process packet counts. It is never
run against actual PCM here. Reuse the established November3 v3 analyzer
instead, including its Process-count and declared PCM16/F32 schema handling.
Only its exact remote root and current offline Rust probe pin change;
normalized AST and test-source comparison confirm that scope.

The nine-file/tool v3 analysis intent has SHA256
`DB67165C7B8200B64D89FF6AFDEE692C2DC15AB17B7B35463ADBF272A4F7344F`.
Its 28 synthetic checks pass locally and remotely. The actual bounded
analysis task then finishes Ready/result0 with no owners, unchanged input
hashes, partial_acquisition=true and two analyzed Process boundaries.

The full six-code nominal detector retains correlation0.7, margin0.05 and
RMS0.0001. Admitted work is 21331968 correlation samples, below the unchanged
1e9 aggregate cap. Both stereo channels produce the same sets:

| Retained boundary | Stored frames | Resolved identities | Unresolved identities |
| --- | ---: | --- | --- |
| Independent Process reference | 431040 | 0,1,2,3,4,5 | none |
| OBS DAWN output return | 773120 | 2,3,5 | 0,1,4 |

Nominal refusals are not lost-frame counts. No SRC fitting, gain fitting,
threshold reduction, identity-by-order or cross-origin latency subtraction
is introduced. OBS WASAPI output is not physical/DAC measurement. The original
route result explicitly retains `obs_cleanup_error`; the analyzer's narrower
`acquisition_error` field is null and must not conceal that cause.

Both real reference/return figures are generated remotely, transferred with
verification and visually inspected under `archive/analysis-v3-output/`.
Neither analysis completion nor the reference's six identified codes proves
return waveform continuity, route quality or calibrated latency.

## Preservation and next work

All 100 evidence members verify independently against their lengths and
SHA256; the ZIP also contains its manifest. `compact-evidence-v2.zip` is
527593 bytes, SHA256
`2004F765F867986B0CFC35436A3F99A94B16116B567B5B89389B8F028BDBBC43`.
Raw MKV/PCM remains remote. The original 29 dirty/cache file hashes remain
unchanged.

Archive helpers first fail on a missing explicit compression assembly and
then PS5 nested JSON-array enumeration. Failed helper files and the partial
ZIP remain separate. The final archive uses the original frozen manifest,
verifies every source pin and finishes without another native operation.

Next acquisition must use the already corrected close order and a fresh
reviewed evidence namespace. Do not reuse the known older runner/analyzer
merely because their narrower tests pass. Diagnose the retained return-code
refusals without declaring them harmless or changing gates. S1 still needs
matched accepted routes and explained full waveform correspondence; S2-S5,
desktop input, Windows naming/settings preservation and sustained calibrated
p95/p99 qualification remain open.
