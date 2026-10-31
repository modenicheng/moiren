# MIC first-start polling control - 2026-10-31

## Scope and verdict

Commit `99685052997e971c9aacf4cc19c20765ad1f8632` changes only the provider's
first admitted, fresh, legal Paused MIC polling wait: half a packet, capped at
the existing10ms. MIC144 at48k requests1.5ms. This is a requested sleep, not a
Windows wake guarantee. Owner construction and WRITE still require a fresh
Running/open/admitted observation; no Paused WRITE, driver/ABI, queue target,
capacity, default, steady event wait or recovery policy changes.

The one actual Interactive/Limited successor `mic-startup-poll1` **fails**.
Its first retained Running startup sample has driver silence0 and3 submitted/
completed host packets, compared with predecessor silence3 and zero attempts.
This bounded observation supports investigating the startup-poll hypothesis;
it does not prove the old9 callbacks are all explained or guarantee startup.
Near the end of about4.7s of source submission the same MIC native generation1
and144 shape remain, but gate epoch1->2 is observed. This is not the exact
native callback timestamp or proof of a clock-rebase cause. The collector refuses the changed
identity, retaining driver silence2 at first fault,6 live and7 after removal.
Host silence counters0 do not override those driver counters.

Probe/task both exit1. Source schedule, collection and runtime cleanup are
incomplete. Cleanup retains `slot=2 HRESULT=0x8007051A`; no error is rewritten
as cancellation or successful recovery. Final target quiescence is separate
from the failed runtime cleanup. No physical render client is opened.

## Frozen conditions and actual acquisition

Root evidence: `D:\coding\moiren-evidence\20261031-mic-startup-poll`.
The frozen helpers/inputs are under `frozen-9968505`; pending parent drafts
remain separate and unchanged.

- Exact graph: VirtualPlayback / speaker = Moiren Input/Source -> unity Bus ->
  VirtualMicrophone / MIC = Moiren Output/Sink. Engine32,48k stereoFloat32.
- Playback default Exclusive480; MIC requested/actual minimum Exclusive144.
  Target1024/capacity4800/jitter96, declared packet/gap480, recovery cap8.
- Same indexed-v2 source:29 identities over30s; correctness3000ms diagnostic
  tolerance and60s requested ceiling. Identity/protocol failures remain hard
  failures; correctness is not permission to cross an invalid native epoch.
- Probe7437312 bytes, SHA256
  `69BB890BDBEADF2F23B994EA70AFDAC225208BD47455CB63746CA83464EB0C77`.
- Driver610/source `883a5be`, installed SYS SHA256
  `94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`,
  unchanged and not reinstalled. Boot remains2026-10-30T09:50:49.6598450+08:00.

One task request is retained at UTC11:41:51.7646712. Actual runner launch is
11:43:55.3418345, about124s later; no duplicate request occurs. Console1 is
Interactive/Limited, medium-integrity with the pinned SID/Explorer owner.
Capture records4.8315167s,1609 packets of144frames,231696 frames/1853568PCM
bytes and212956 metadata bytes. The source acknowledges226560/1440000 source
frames,5120 nonzero frames and an incomplete schedule; ReleaseBuffer
acknowledgement is not delivered audio. No forced process termination occurs.

## Waveform and evidence limits

The unchanged nominal indexed analyzer competes all29 exact source codes in
explicit unlabelled retained windows, without SRC/rate fit, source-origin
alignment, threshold relaxation or peak-order relabelling. It resolves only
identity1. Fully acknowledged source identities are0--4:0,2,3,4 are not
identified;5--28 have no completed submission evidence and were not exercised.
Neither group is a proven waveform-loss count. The1576 nominal packet-QPC
policy invalidations are not1576 proven lost packets. No spacing comparison
or physical delay follows from one unique identity.

`independent-pcm-comparison-9968505-20261031-114726-001107` retains script,
inputs, morphology, exact work28151808 and analyzer outputs. Sparse intended
zero regions, PCM packet flags and driver unserved callbacks are distinct
observations. Zero stored-capture loss flags cannot erase driver silence or
the gate-epoch failure. The earlier30s failure-quality run and this partial
run must not be compared as equal-duration quality or latency trials.

## Archive and retained failures

`mic-startup-poll1-evidence.zip` is1903562 bytes,28 payloads plus manifest,
SHA256 `287F3300B15530F8EC4763ABB59518BCF06CBC215C14616B10DBFC97CC1D0089`.
Manifest7650 bytes, SHA256
`052A11F46CD64DEA56B44F9E9EBA7835F8773527F6B096B84E9A535242632B40`.
Root's frozen verifier authenticates CRC, members, lengths/hashes, inputs,
exact task action/principal and failed result provenance before extraction to
`frozen-9968505/mic-startup-poll1-verified`. Integrity is not quality acceptance.

The independent host-only acquired audit passes111/111 checks with no failed
checks, explicitly retaining `qualified=false`, probe/task1, incomplete
collection and the terminal cleanup error. Its receipt
`independent-mic-startup-poll1-acquired-audit.json` is23100 bytes, SHA256
`18FC52CB4FEDAF1724738667C3412B9433ADE7BA5BCF73A7F5870AC46023DAB8`.
The audit authenticates and classifies this failed acquisition; it does not
convert retained PCM or zero capture flags into waveform acceptance.

The initial root verifier used the parent pending-draft directory and correctly
refused `trusted local mismatch: run-mic-startup-poll.ps1`, before extraction.
The correct frozen directory succeeds; no pending input is replaced. Poll3's
shared-log read failure is retained; poll-v2 permits shared read/write handles
and bounds content without changing the native runner or replaying audio.
The stale outer build-prep receipt resolved main instead of the subprocess
worktree; `root-build-pin-correction.json` explicitly retains/corrects that
provenance error. It does not relabel the previously deployed5f9 app.

## Host checks and application deployment

Root reruns7 focused tests,701 feature-library tests (2 ignored),622 probe
tests, strict library/tests Clippy and fmt, all passing. Root additionally
reruns48 runner/launcher and38 archive inert PS5.1 fixtures. Independent source
review finds no helper blocker. These host checks do not override native exit1.

An explicit separate app build at9968505 produces33781248 bytes, SHA256
`3CA9CB0D006E00688DF939DD4AA8CFC5514DA20E557425F59C70EB795B6EBF7C`.
Target deployment is side by side at
`D:\MoirenTest\20261031-mic-startup-poll\app\bin\moiren-app.exe`.
UTC11:46:49 receipt confirms help, monitor-help and offline gain exits0 with
the expected eight output samples, isolated smoke profile, no GUI/native audio,
driver install, volume/mute/default-role writes or remaining owners. It is not
desktop/native acceptance, a new preferred build or a stability promotion.

The fresh UTC11:48:02 root audit confirms no audio/build owners or raw children,
root PnP OK, unchanged driver/boot, six default roles and12render-master
scalar/mute snapshots. A frozen helper with50/50 guard fixtures,4/4 flow
fixtures and independent20/20 review first passes a read-only actual audit,
then unregisters only the exact terminal root task after repeating every guard.
The UTC11:54:07 verification confirms that task absent with owners/children
empty, root OK and the same driver/boot/defaults/render masters. No run/archive/
history/cache/file is deleted and no process is killed.

## Remaining work

Identify the later gate-epoch transition before another production fix.
Existing driver Clock rebase cause/before/after and no-copy witnesses can
distinguish a native deadline rebase from host lifecycle actions only when
actually collected with matched identity and complete trace semantics. This
run has no WPP acquisition; source inspection alone does not prove its cause.

S1--S3, sustained delivered waveform, matched VAD/Process measurements and
calibrated physical latency remain open. Stable below10ms on exactly
Input -> Bus -> Output remains preferred. Only evidence proving that impossible
under the declared current-device conditions permits a defensible device
minimum/best stable achievable fallback. Neither impossibility nor such a
minimum is established; no buffer default or quality gate is relaxed.
