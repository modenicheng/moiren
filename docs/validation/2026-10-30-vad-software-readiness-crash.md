# Pure-virtual VAD readiness follow-up and interrupted acquisition

Date: 2026-10-30, Asia/Shanghai. Target: `LAPTOP-NP49RG6H` /
`192.168.12.97`. Probe source: `9c86a1f`; the app and installed Moiren driver
are unchanged. This follows the separate
[physical-master source-readiness receipt](2026-10-30-vad-source-readiness.md).
**S1-S3 remain incomplete. No calibrated physical delay or lower default is
qualified. Native audio testing is paused after the target's automatic crash
reboot; no retry, requested reboot or system-setting change follows it.**

## Separate software route and recovery-budget results

All cases request exactly VAD playback -> Bus -> VAD microphone, 48 kHz stereo,
Engine32 and capacity4800. Playback negotiates minimum144-frame Float32 packets;
the shared microphone uses480-frame Pcm16 packets. This is pure-virtual realtime
pacing, not DAWN physical-master pacing. No physical-output flags are passed.
Source schedules contain1440000frames and29 repeated1024-frame markers at
amplitude0.025. Storage is bounded at32MiB PCM /8MiB metadata /100000packets,
with an8192-frame recorder ring,100ms preroll and200ms postroll.

Correctness mode retains3000ms progress grace and a60-second ceiling; it does
not add3000ms of audio inventory or erase quality faults. Identity, native IO,
epoch reset, storage and cleanup errors remain hard failures. Jitter96 and the
declared packet/gap480-frame bounds stay fixed while recovery cap is varied.
Those declared bounds are not an independently observed delivery guarantee.

| Run | Input target | Recovery cap | Acquisition and retirement | Strict quality |
| --- | --- | --- | --- | --- |
| readiness1 | 1024 | 1 | Stops after1.2030068s /57744 acknowledged source frames; collection and cleanup verdicts false | Carrier overflow / epoch reset; driver drops and microphone silence retained |
| readiness1 | 768 | 1 | Stops after4.257389s /204336 acknowledged frames; collection and cleanup verdicts false | Carrier overflow / epoch reset; driver drops and microphone drop/silence retained |
| readiness2 | 1024 | 8 | Completes30.2194939s /1440000 acknowledged frames; collection and cleanup true | False: microphone driver silence and recorder discontinuity retained |
| readiness2 | 768 | 8 | Budget refused before source activation; cleanup true | No acquired PCM and no quality pass |
| readiness3 | 768 | 4 | Interrupted by bugcheck before native reports/logs persist | Unknown/incomplete; no retry |

Both cap1 cases first retain driver drop5, carrier drop1 and provider reset1;
their final driver drops are6 and7. The first-failure microphone silence is
2749/2657frames; final values are2813/2657frames. The target768 microphone also
retains one driver drop and two driver silence packets. Playback queues approach
capacity and the clock servo approaches its correction limit. Discarded virtual
scheduling debt is a candidate explanation, not an established root cause or
proof that changing a production recovery cap is safe.

The cap1 cases ultimately observe both children Removed and the Engine
Dormant/Closed, but the retained `Observe: provider epoch reset` cleanup error
is not overwritten with that later native state. Collection/cleanup are still
failed. Correctness progress grace is not permission to continue on an unsafe
transport identity/epoch.

The completed cap8/target1024 case has source Start/Stop/join, the full acknowledged
schedule,1456320 stored recorder frames and normal retirement. Provider telemetry
retains zero carrier/drop/reset/recovery/silence-frame counters and playback
steady xruns/short reads/recovery-prime muting are zero; initial prime32frames
and native source silent preroll144frames remain explicit. Importantly, the
microphone driver's fresh QUERY snapshot has `silence_packets: 1` both in the
first-quality observation and the final active snapshot, although provider
telemetry has `silence_frames: 0` / `silence_packets: 0`. These are different
observations, not evidence that the driver silence disappeared. The recorder
retains one discontinuity, zero artifact drops/overflow/gaps/timestamp-error
packets and3034complete stored packets. Exit0 means completed collection and
cleanup, not quality pass; no startup fault is waived.

Cap8/target768 refuses `input target cannot cover render demand, capture gap,
jitter and lookahead`. Source never starts, acknowledged source frames remain0,
and no larger target is silently substituted. The physical-master768 result
must not be copied onto this different clock/recovery route.

## Retained PCM and packet diagnostics

Host-only known-template analysis of the completed cap8/target1024 recording
finds29 exploratory peaks on each channel, matching the schedule's count.
The explicit absolute-correlation floor0.5 and minimum separation24000frames
are diagnostic settings reused from earlier retained-waveform analysis, not
production qualification thresholds. Every qualifying interior peak is retained;
there is no top29 truncation. Mean-centered and uncentered cross-checks identify
the same peak frames. Centered correlations range0.5688293348..0.9522425969;
neighboring detected peaks are47901..47990stored frames apart. These are
waveform observations, not physical delay, lost-frame counts or calibrated drift.

PCM is11650560bytes /1456320stereo Float32 frames, SHA-256
`F76E1F600A77346B6400D71F5F18E5AC912D655F0BD2C3B6B5DC6506D78AF4B1`.
All2912640samples are finite; per-channel absolute peak is0.047576904296875,
RMS0.00334216511503884. Channels are sample-identical and therefore are not
independent pre-route/processed references. The same marker repeats; count
agreement alone does not establish unique marker identity, source-to-recorder
mapping or complete continuity. No source-index subtraction produces a delay.

All3034metadata records contain480frames and cover stored PCM exactly once,
in order, with adjacent sequence and device positions. Only packet1 has flags1
(discontinuity); later flags are0. No QPC is zero, repeated or regressed.
Later QPC intervals span9.4858..10.5276ms, median9.9911ms. Independently evaluating
the existing one-frame-plus1000ppm nominal-rate diagnostic rule identifies2653
intervals outside that policy. That count is not2653raw native faults, proven
PCM holes or measured clock skew. Startup discontinuity and the driver's
microphone silence remain strict quality failures; they are not excused by
adjacent stored metadata or29detected peaks. The production analyzer and
qualification thresholds are not changed or claimed passed.

The analysis verifies selected inputs against the already verified archive,
records its script/library identities and is rerun locally to a separate output
with an exact JSON equality check. Script and results are retained as
`analyze-vad-software-readiness2-input1024.py` and
`vad-software-readiness2-input1024-waveform.analysis.json` under the evidence
root; no new target acquisition, build or setting change is needed.

## Bugcheck evidence and matched-image analysis

During cap4/target768 the target disconnects. The minidump records
**2026-10-30T09:50:31.992+08:00**; subsequent boot is
**2026-10-30T09:50:47.500+08:00**. Windows event1001 retains bugcheck0x7E
(`0x1000007E` in the dump), exception0xC0000005 and fault address
`fffff800b0f42fa6`. The agent did not request a reboot.

The first local debugger run lacks matching kernel symbols and produces a
`WRONG_SYMBOLS` bucket. It is retained as a failed analysis, not a root-cause
verdict. A second pass retrieves the exact target `ntoskrnl.exe` and
`nvlddmkm.sys` read-only, verifies their lengths and SHA-256 against a fresh
target audit, and uses the kernel image's matching Microsoft public PDB:
`ntkrnlmp.pdb`, GUID `C6F2581196CED620CCA4074DFBB1F513`, age1.
Image timestamp/hash, size and checksum match the dump modules; no forced
symbol mismatch option is used.

With the matching images and kernel symbols, the recovered context is:

```text
nvlddmkm+0x262fa6: mov rax,qword ptr [rcx+10h]
rcx=0x8000; read address=0x8010
nvlddmkm+0x262fa6 -> +0xf7d1a -> +0x149811 -> +0xd9bce
nt!IopProcessWorkItem -> nt!ExpWorkerThread -> nt!PspSystemThreadStartup
Process: System
Bucket: AV_nvlddmkm!unknown_function
```

The mapped NVIDIA image is32.0.15.7652 (576.52). Its private PDB is unavailable;
the NVIDIA frames therefore remain offsets. No Moiren frame appears in this
recovered call chain. This establishes the immediate faulting module and a
system worker context, **not** that NVIDIA is the sole root cause or that Moiren
cannot have contributed earlier. The small kernel triage dump lacks enough
memory/history to exclude prior corruption or indirect triggers. No speculative
driver patch, GPU update, Verifier/security change or unsafe reproduction is
made from this one dump.

The third task is Ready with result0 after reboot, but only config/schedule
artifacts exist. An explicit `interrupted-by-bugcheck.json` records collection
false, cleanup false, missing result and unqualified status. Task result0 is
not treated as completed acquisition, valid retirement or successful cap4
operation. Post-reboot absence of child devices is not pre-crash cleanup proof.

## Verified artifacts and final target audit

- `vad-software-readiness.zip`:1496335bytes,65 manifest files /66entries,
  SHA-256 `AA5A8D012FF5F46F78393FD4C4164555CA89ACCEB74E4BEA8E530E17A8695453`.
  The host verifies the exact file set, every length/hash and the archive hash.
  The65files include all three run folders, scripts, manifest, dump and events;
  later matched-image/debugger analysis and the final audit are separate.
- Dump `103026-17406-01.dmp`:7754334bytes,
  SHA-256 `F9FEBA50E669CADD19EA1168E434657BFF734E4C6946FB8D72168931423D6238`.
  The original system dump is preserved. Host-only symbol/debugger work does
  not run a live target debugger or change the target's installed files.
- The fresh final audit at2026-10-30T10:05:26.9511457+08:00 has no app/probe/build
  process, MoirenVad Running and only present `ROOT\MEDIA\0001`, status OK.
  It checks all three exact owned terminal task actions and archived receipts
  before unregistering them; none remains. No external/user process is stopped.
- All render volume/mute values and default roles remain unchanged. DAWN is
  unmuted at volume1; Realtek stays muted at volume0. No speaker, volume/default,
  clock, security or driver-installation writes occur. D free254855270400bytes,
  C free8865341440bytes;3407485343bytes of reusable Cargo outputs are retained.
- Final audit SHA-256
  `A855ECE84C62C76FDE907651E71AD224038F99DB254EB1C05EABF7F0B6C3052B` is checked
  after download. Its installed Moiren SYS hash remains
  `CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.

Evidence, verifier, matched images, symbol commands and debugger outputs live
under `target/physical-1921681297-20261029/`. A separately hash/length-verified
copy of both new archives, verifiers, summaries, audit, matching dump/images/PDB
and final test logs is retained outside Cargo output at
`D:\coding\moiren-evidence\20261030-vad-readiness` (29files plus its backup
manifest); a later `cargo clean` must not be allowed to be its only retention
mechanism. No original evidence is deleted or moved.
`verify-software-readiness.py`
checks complete/refused/failed/interrupted outcomes separately, including the
nonzero final microphone driver silence and successful underlying retirement
without erasing the failed cap1 cleanup verdicts. Local final525probe tests,
strict probe/test Clippy and fmt pass again; the archived remote
release525-test/build results belong to `9c86a1f`. No new driver or application
source change is made by this receipt.

At this checkpoint, origin/main remains `c0bd006` after a fresh fetch and is an
ancestor of this feature branch. Original S3 desktop requested/active Apply and
replacement retirement are already verified in the DAWN receipt; this crash
does not erase that evidence. S1 common calibrated physical reference and
accepted waveform continuity, plus S2 native lower-budget feasibility, remain
open. Do not expand S1-S3 into every optional policy combination or present a
new short run as S5 sustained qualification. Resume native work only after a
safe target-testing decision; no VAD/Process winner or stable sub-10ms output
is established here.
