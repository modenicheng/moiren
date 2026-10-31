# Native source write and provider READ discriminator

Date: 2026-10-31, Asia/Shanghai. This executes the
[bounded witness plan](2026-10-31-source-write-witness-plan.md) after the
[all-zero provider capture](2026-10-31-provider-input-capture.md).
It is a diagnostic, not S1-S3, waveform continuity or physical acceptance.

Virtual speaker/Playback remains Moiren Input/Source; virtual MIC remains
Moiren Output/Sink. Accepted PR43/44/45/47 are already in main `6b4ffe6`,
contained by this experiment. Draft PR26 is not merged wholesale.

## One changed diagnostic boundary

Source implementation `1a14bba`, integrated as `b393807`, adds explicit
`process-source-worker --source-write-witness`. The witness is disabled by default.
Before GetBuffer, it selects the current first genuinely nonzero encoded
frame, validates its offset and reserves fixed metadata. After the existing
copy, at most16bytes are read back with volatile reads from the actual native
destination before the matching ReleaseBuffer. No pointer escapes the lease;
no new allocation, file IO, JSON, wait or native client occurs inside it.
Successful Release confirms the witness; a failed attempt retains its original
HRESULT and does not advance the source cursor. The original audio error wins.
The witness is always unqualified and is not a full-packet waveform recorder.

Actual helper-copy and CLI value-position tests have retained behavioral REDs,
then6focused GREENs. Root independently runs all538example tests, strict example
Clippy, formatting and the Release build successfully. Independent final source
review reports no blocker. Ordinary IO/Release/Stop failure evidence is retained;
an arbitrary panic can still leave only an earlier snapshot and lose the witness.
Linux execution and allocation instrumentation of this helper were not performed.

The frozen PS5.1 runner has151passing dispatch-ready fixtures; its predecessor
READ collector/helper bodies and original evidence remain unchanged. Source
identity, exact schedule, current native frame coordinates, actual arrays,
flags, successful Release, final cursor, clean exit/join and raw QPC units
are validated. Source and READ formats are allowed to differ. Missing or
unconfirmed evidence is inconclusive, never waveform recovery.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| changed `e2e_latency_probe.exe` | 7042560 | `7D2DA911BE4DD9049544513D1F3141EE52E112B9DF459C7D0EB5624B5DF05717` |
| unchanged holder `b0a0223` | 7421952 | `3D66F7518BCF57924505D9B80BF592CFF9BCE64B3D106F4AB182C558792F7ABD` |
| frozen witness runner | 49016 | `9E177F1113F2A37C80F810BF0E632E439F935B1B3AB50963FCA92AED2CB577DF` |

Only the changed source worker is deployed side-by-side. Driver `6d9e32f`,
holder, production app, schedule, buffer/default/period and routing are fixed.
No target build overlaps acquisition. Existing Playback -> Gain0.5 -> MIC
remains the fixture, not exact Input -> unity Bus -> physical Output.
No Process recorder or physical audio client is started.

## Native result

Run `write-witness1` spans07:30:39.668 to07:31:42.513 local on physical target
`LAPTOP-NP49RG6H`. Source PID4152/creation134378766459386777 addresses the exact
owned Shared Playback endpoint. Twenty contiguous1second xorshift markers
command amplitude0.025 at48kHz stereo; the schedule is not shortened.
These are target-local wall times, not cross-host clock synchronization;
all QPC comparisons below use the same target/boot and recorded frequency.

The first witnessed post-preroll packet is480Float32 stereo frames/3840bytes:

- Native preroll1056frames; source frame0/native submission frame1056.
- Frame offset0/byte offset0;16copied bytes are `CD CC CC BC` repeated4times
  in both the encoded array and the independently read native destination.
- Release flags0, HRESULT0, no failed attempt or source/playback failure.
- QPC10MHz: before copy779979855033, after copy779979855036,
  Release return779979855043. These are brackets, not physical delay.
- Final584160acknowledged/nonzero-source frames plus1056preroll gives
  585216native submitted frames; schedule remains incomplete, event timeouts0.
- Start-before to Stop-after14157.8874ms is below the20second schedule.

Actual accepted provider READ retains480000stereo PCM16frames/1920000bytes:

- All960000scalar samples are exactly zero; channel peak/RMS are zero.
- 1000packets of480frames, sequences1..1000, no sequence gaps.
- 1000subsequent carrier publications, zero stale/epoch exclusions.
- Stable owner `MoirenAppVadPlay`, native `[9,779908242902,9,1,1]`,
  reset0/admission2/revision2; seal frame_capacity, worker joined/completed.
- 990READ transfer callbacks fall strictly inside the conservative source-active
  window779980988730..780101374268 and at/after witnessed Release return.
  Their first/last ticks are779981053667/780079953912.
- All1000callbacks follow Release; only990also satisfy that observer window.
  First10packets outside that observer window remain in the original capture.
  Transfer timestamps do not establish sample-duration coverage or source origin.

Raw SHA-256 is unchanged from the earlier all-zero capture:
`124617C1F65E92D3BC895FBD869E4BB16A30754B198F59E6E973949B9AAA1B01`.
Metadata SHA-256:
`9BD034A27604126D3D8A1E69BCFA08BF8B58E94F85D6C5E0110B1D64DB2CF9E0`.
Host analysis reads the entire original PCM; template search is
unknown_zero_energy, not recovered marker alignment or a latency result.

## Meaning and cleanup

This closes the missing observation of the source's actual granted destination
for one nonzero prefix and successful matching Release. Together with the zero
READ, it narrows the unresolved interval after that client Release and before
the accepted provider READ. It does not prove that the complete submitted
waveform was played, that Windows delivered nonzero data to the kernel render
packet, or that the driver alone loses it. It does not establish a common
cause for the independent Process silence. Session0/no-interactive-user remains
an unproven confound. Repeating an equivalent headless Process variant or tuning
queues is not a discriminator for this missing boundary.

Collector/holder/source exit0, errors empty, no forced termination or residual
owners. Holder60754ms includes60001ms idle/59verified windows; recorded timeline
glitches and epoch changes are zero. This idle progress is not PCM continuity.
Service joined and control released are true. Public endpoint_absence_confirmed
remains false; the independent final PnP audit finds only healthy `ROOT\MEDIA\0001`.
Intermediate not_joined_unknown capture snapshots remain preserved; terminal
capture error is null. Driver Running/oem201.inf/version3.1.1.607 and SYS hash
`AED94D18375BFD59B78BA84C3691B4B69C1EEAD4E5210F36759D018ED3443268`
remain unchanged, as does boot2026-10-30T09:50:49.659845+08:00.
No speaker/default/privacy/autologin/cache settings or originals are changed.

Host originals: `D:\coding\moiren-evidence\20261031-provider-input-capture\write-witness1`.
Remote originals: `D:\MoirenTest\20261031-provider-input-write-witness\runs\write-witness1`.
Archive `provider-input-write-witness-20261031.zip` contains38payload members
plus manifest,17897642payload bytes/5577849ZIP bytes. Target verifies each
member length/hash; host repeats that comparison against separately downloaded
originals, the unchanged holder and frozen source/runner. Embedded/external
manifests agree. Archive guards independently pass7Python and10actual PS5.1
fixtures, including failed-collection retention without promoting acceptance.
No original is deleted. RunnerRevision in the manifest is the operator-declared
experiment checkout checkpoint `b393807`, not a claim that the evidence-only
script is git-tracked there; its exact hash is the independent script pin.

- ZIP SHA-256: `C32DB2C991F966072D9801B2019B74F8A453E3334562B71E294A75E46F5CB3D2`.
- Manifest SHA-256: `46FBDFEFBB7A2B5BFC1ABD093417BEFDEE2AED4DAE0EDC27562F11E8DC71C528`.

The separate frozen host-tooling archive contains78payloads plus manifest,
808946payload bytes/207788ZIP bytes. Root independently compares every member
with its original; it includes9source files, source RED/GREENs and selected
runner/archive fixtures/logs. ZIP SHA-256:
`279921BD47D111D210AFF3DA6163B5CCE2723590C1497DB5767E24174F51F8F6`;
manifest SHA-256:
`6FC8710DA2EC7BEFDE6F07E114C1D36528BB057B6B8162EEF99A5178CB46B283`.
The later Linux-target example cargo check succeeds but is not Linux execution;
its log and the final read-only target audit are retained separately, outside
that earlier frozen host archive. The audit again finds no test/build owners,
unchanged boot and healthy driver/root; no cache cleanup is needed.

Prior pushed `ff8fb8b` CI run41142598234 completes20/20 successfully. That is
the prior source checkpoint, not CI for `b393807` or a kernel/native acceptance.

S1-S3, stable calibrated p95<=10ms, lower-buffer defaults and a matched
VAD/Process latency comparison remain open. Exit0 is collection, not acceptance.
