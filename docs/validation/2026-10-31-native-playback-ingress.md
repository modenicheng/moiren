# Native Playback consumption boundary - 2026-10-31

## Verdict and direction

Physical target `LAPTOP-NP49RG6H` now runs explicitly enabled diagnostic
driver `2a5aee5`, `oem203.inf` / `3.1.1.609`. App `af70aa9` remains deployed
side by side. No app rebuild, target build, reboot, trust-store change,
volume/default change or physical output client occurs in this checkpoint.

| Windows virtual device | Moiren role | Other applications |
| --- | --- | --- |
| Speaker / Playback | Input / Source | Output / render |
| Microphone / MIC | Output / Sink | Input / capture |

Freshly fetched `main` and `origin/main` remain `6b4ffe6`; accepted PR43,
PR44, PR45 and PR47 are merged. Experimental PR26 is not merged wholesale.
Four app-side VAD direction/binding regressions pass again.

The unchanged fixture is `Playback -> Gain0.5 -> MIC`, NOT exact S1 and
not a DAWN physical-output measurement. Source, native event buffer and
provider evidence narrow the unobserved boundary, but do not qualify
waveform delivery, continuity, a sole cause or physical latency.

## Implementation and host checks

Atomic commits are `b630919` (GCC enum/uint16 fallback), `19e2e38` (native
observer, cherry-picked from independently reviewed `72847ca`) and `2a5aee5`
(enabled/default-off harness integration and project header list).

The existing `MOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC` switch still defaults to
0, with isolated enabled build directories. Default builds compile out the
new state, hooks, scans, QPC and exporter. No public ABI or queue/default
policy changes. Enabled per-slot native state is 728 bytes, separate from
the existing 440-byte READ observer. It binds only after an actual accepted
READ selects the exact endpoint/session/stream/gate/format/shape. CREATE
alone resets it; later acquire/resume cannot rearm historical evidence.

Lifetime scan cap is 480000 frames, at most 4800 frames and eight bytes per
frame per observed span. Timer hooks read paired native/Scratch bytes AFTER
memcpy and BEFORE original clear. Event hooks read the live native buffer
ONCE after existing EOS truncation and before transfer/clear; duplicated
destination bytes are explicitly not an independent copy witness. The
enabled event wrapper preserves the original rundown and single Shared-lock
boundary. No new lock, wait, heap allocation or hot-path WPP is introduced.
Added enabled diagnostic overhead is unknown, not a latency result.

Passive QUERY/gate-close publication retains the selected historical identity
even if no later copy occurs. At most one interim and one sealed group are
exported, 26 fixed correlated fragments each. The original 15-fragment READ
schema is unchanged. Prefix offsets can point to a late interesting frame
within a scan; prefix `Bytes`, not the original full scan length from that
shifted offset, bounds the captured prefix.

- Root MSVC passes all 17 suites, including 54 pause-fence scenarios, ten
  READ-copy groups, ten native groups and both actual default-off paths.
- Root GCC/WSL passes all 12 portable suites. Default and enabled WDK Analyze,
  signability and InfVerif checks succeed. Independent source/wrapper review
  reports no blocking finding before deployment.
- The frozen PS5.1 trace wrapper passes 64 fixtures; the rerun source runner
  reports 141, not the older receipt's 142. Both exact script hashes remain
  unchanged. No new fixture-count claim is inferred from the old receipt.
- CI `41153436650` on `6fed7ae` is not green: `vad_driver_host` fails on the
  GCC enum/non-enum conditional under `-Werror`. `b630919` fixes that exact
  reproduced expression with a uint16 cast, not a lint allowance. Actual WSL
  full-suite execution passes. Ubuntu tests now pass the preceding DTO fix;
  the run now finishes with 19 successful jobs and this one failed driver
  host job. No job is canceled or restarted to hide that failure.

## Deployment and rollback

The reviewed narrow deployer is
`D:\coding\moiren-evidence\20261031-native-playback-ingress\deploy-driver-2a5aee5.ps1`,
SHA `ACF1F00592468B2CADB45F1BFD833921CD62ECA13D21CAF80D4B5935604D5ED8`.
Manifest SHA is
`1E2C9E3611ECA742A037B9B0F54108C8101C781EE4ADEF01586A5A9CDBEA1EDB`.
Local package length/hash pins, PS5.1 syntax and independent deployer review
pass before installation. Host verification reports `FailedUntrustedRoot`;
that is NOT trusted verification. Target verification using already-installed
trust must pass for both package signatures and catalog membership before
mutation; all eight candidate/rollback verification calls exit 0.

New signed SYS is 168344 bytes, SHA
`C9F7FF8273239B97436DF75A5D9E7584347F54A79CCFA21E1889EE546E4C591F`.
INF is 1799 bytes, SHA
`A603889EC4DD7C246FAB51469EE38F28BA2E84B5C59E14A67507987A634173CF`;
CAT is 2484 bytes, SHA
`7F7A6D59D6F08EED95FA0D4916AE53BDECBDC03B90F436BDF13FEA20BBD54567`.
Matching PDB SHA is
`B35F2516718E56B7C74B10F070A5E2A9205EBA8979D8A117B2BC2E6FA553C388`.

Installed baseline 608/oem202 and all retained rollback files are verified
before mutation. Candidate install and root restart exit 0; final service
Running, PnP OK/problem 0 and installed file hash match 609/oem203. Boot stays
`2026-10-30T09:50:49.659845+08:00`. Both 608 and older 607 rollback packages
remain. Source revision is operator-declared and tied to build/on-disk
artifacts, not independent in-memory image attestation.

## Acquisition and retained failure

Fresh `nativeingress1` fails before any source starts: MIC changes from
admitted to gate-closed/not-ready with gate epoch 2. Holder exits 1 without
forced termination; provider raw file is empty, with no accepted packets.
Tracefmt sees four events, no diagnostic fragments. No source/PCM conclusion
follows. This is a retained readiness failure, not erased or fixed by retry.

Fresh `nativeingress2` uses unchanged worker, holder, runner, wrapper and
parameters. Collector, source and holder exit 0 with no retained errors,
no forced owners and no remaining audio/build owners or present VAD children.
Holder completes requested 60000 ms idle hold in 60005 ms, 59 verified windows,
zero recorded timeline glitches/epochs. Service joins and control session
releases. Public `endpoint_absence_confirmed` remains false; the separate
final PnP audit finds no present children. Idle zero-PCM progress is not
nonzero audio continuity acceptance.

Source schedule is still active at Stop; observed lifetime envelope is
12407.8475 ms. Source actual Float32 GetBuffer destination retains nonzero
`CD CC CC BC` repeated four times, matching encoded bytes before successful
ReleaseBuffer(flags 0), return QPC `852520794746`. Exact source-owned controls
then report session Active, session/endpoint master 1.0, both unmuted, all
HRESULTs 0. This one snapshot is not effective-gain or played-PCM evidence.

## Actual event/native and READ evidence

Exact stored identity is endpoint 3, session `852467196805`, stream generation
5, gate 13; PCM16 stereo, 480 frames, align 4, frequency 10 MHz. The actual
native path is EVENT (`path=2`, `PacketCount=2`), not timer. Logical packet
size is 1920 bytes; allocation is 4096 bytes. RT packet sizes are 1920/1920,
offsets 2176/0; mapped timer extents are 0. First/last EOS flags are 0;
middle EOS states are not individually exported.

Complete 26-fragment interim revision 1 and sealed revision 1000 are retained.
Native observer records 1000 calls, sequences 2..1001, 480000 frames /
1920000 bytes, no sequence-gap event or counter saturation. All sampled native
bytes/frames are zero. Event destination counters repeat the same read, so
`paired_copy=0`, `event_same_buffer=1`; zero differences are not a copied-data
integrity test. First callback/read bracket is `852521089718`,
`852521089734..852521090125`; last is `852620992529`,
`852620992554..852620993097`.

Complete unchanged 15-fragment READ groups retain 1000 copies, sequences
1..1000, 480000 frames, both transfer source and READ MDL destination zero.
First callback/read bracket is `852520992080`, `852520992142..852520992630`;
last is `852620890278`, `852620890963..852620891496`. Provider metadata has
the same stored stamp `[3,852467196805,3,5,13]`, reset 0, ticket 2, active
revision 2, 1000 published packets, no stale/epoch exclusions and clean join.
All raw scalar samples are zero. All provider callbacks follow witnessed
Release; 990 fall strictly inside the conservative source-active observer
interval `852521916047..852642760995`.

The selecting READ exactly matches sequence 1. Native begins at sequence 2
and ends at 1001, which provider capacity does not retain. The common logical
IDs are 2..1000: 999 packets / 479520 frames. First/last callback ranges
intersect, but these are not complete per-packet native history, exact sample
alignment, source recovery, all-interval coverage or latency measurements.

Matching PDB/TMF tracefmt reports 87 total events, 0 EventsLost, 0 format
errors and 0 unknowns. Its separate report is not a decoder loss-proof claim.
The strict offline decoder retains `qualified=false`, `trace_loss=unknown`,
and requires explicit matching identity/path/count with all 26 fragments.
Independent adversarial review finds two P2 decoder gaps: different serialized
scans could claim overlapping QPC brackets, and four timer observations could
claim an impossible same-segment span. Both receive RED/GREEN regressions.
The evidence-only fix preserves legal same-scan prefix brackets and two-span
timer copies; no driver/target rerun or source change is required. Root reruns
24 native groups, nine independent controls/reproductions, the frozen 17 READ
groups and actual run2 decode successfully. Final native decoder SHA is
`811893FEB2069D3A1D3109B080E88FE14991DB70A197A4A729CB291185D71E2C`.
Its actual result remains all-zero event observations and never qualification.

Run2 raw SHA is
`124617C1F65E92D3BC895FBD869E4BB16A30754B198F59E6E973949B9AAA1B01`;
provider metadata SHA is
`E7043FC7493EC7CEBC91E6BA7C05746F59FB624484A45D1DA073548CDA1EDBB0`;
ETL SHA is
`A577109A5DD6A70B5A0384257C2344BA252782BB93B3AFFD13F4E71703D4354E`.

## Retention, limitations and next boundary

Evidence root is
`D:\coding\moiren-evidence\20261031-native-playback-ingress`.
Target-generated manifest SHA
`FCCC097B17B16D0D7BB34DC64340E4ABDD7D048BEB2DFDD538FBD270BD4BD9E3`
pins 96 payload members / 4896528 bytes. Root and an independent verifier
check every copied member's length/SHA, then every ZIP member's bytes/hash
and CRC. Archive
`native-playback-target-20261031.zip` is 272714 bytes, SHA
`6FDC2C2FEE7FB33582897F94C7CA5E5FAF61C9D75DC973ACBF2EB587C9495F3F`.
The separate independent cross-artifact check passes 119 checks, including
exact deployment, failed attempt, source/provider/native correspondence and
cleanup. No originals or rollback packages are deleted. D: still has about
225 GiB free; no cache clean is needed.

Failed tracefmt invocation used an unsupported `-summary` argument; actual
tool help established the supported output/suffix. Corrected decode retains
both runs and its original logs. Manifest creation completed before a
dictionary `Measure-Object` receipt failed; the existing unchanged manifest
was audited and downloaded, not regenerated. A separate read-only failure
log-tail audit stalled; only that exact owned PowerShell process was ended
after command/start/parent verification. No audio owner was terminated.

The zero observation now reaches the consumed native event buffer after EOS
handling and before original clear. It does not establish where the source
became zero upstream, a layout defect, a Windows/session cause or a driver-only
cause. No equivalent headless Process-zero run is repeated. Current SSH
context is Session 0; no explorer is observed and `quser` reports no users,
while CIM still contains old type-2 logon records. Interactive availability
and Session-0 causality are unproven; no autologin/privacy/session change is
made. A distinct source/native or interactive-context control, rather than
another identical zero-data replay, is needed next.

Exact `Input -> Bus -> Output`, S1-S3, nonzero continuous delivery, calibrated
physical p95 <= 10 ms, lower-default promotion and a matched VAD/Process
winner remain open. No acceptance or default gate is relaxed.
