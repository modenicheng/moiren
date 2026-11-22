# Interactive exact IO/Bus control - 2026-10-31

## Outcome and boundary

Independent acquired-archive audit passes93/93 integrity/consistency checks;
**the audio result remains unqualified, not a quality pass**.
`interactive-iobus1` completes with collector exit0 at10:53:24.4252194 UTC,
but `QualityPassed=false` and `QualityComplete=false`. Clean collection is not
accepted waveform, continuity or latency evidence. The first observed MIC
silence count3 is not the total: the acquired Running snapshot retains9.
Startup log order alone does not establish exact timing, origin or PCM impact.

This receipt is separate from the acquired Shared interactive Gain(0.5) holder
history. The new route is exactly virtual Playback Source/Input -> unity Bus ->
virtual MIC Sink/Output, with an owned native MIC recorder. No physical output,
independent reference or calibration is supplied by this route.

Host evidence: `D:\coding\moiren-evidence\20261031-interactive-iobus-waveform`.
This receipt uses the final acquired audit; the Shared history is not rewritten.

## Frozen fixture and actual native shape

`runner-freeze.json` pins probe revision
`5f9eb02526b691ecc988a0c939a32d90c35492a7`,7435776 bytes, SHA256
`17BEDCF5964850B79572FF5D2FD42CD63B8C0325437926E73265AD9DB5F946B3`.
Driver610 source is `883a5be`, expected on-disk SYS SHA256
`94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`.
These on-disk preparation pins are not an in-memory image attestation.

The fixture requests Interactive/Limited execution, Engine32, target1024 /
capacity4800, jitter96, declared packet/gap480 and recovery cap8 periods.
Playback minimum is omitted; MIC Exclusive minimum is requested without fallback.
The existing indexed-v2 source is30s/29 markers; the overall request ceiling is
60s with correctness-mode3000ms grace. WPP and physical output are disabled.
The acquired report retains source Exclusive Float32/stereo48kHz/480 frames;
MIC Exclusive Float32/stereo48kHz/144 frames, query/first Initialize HRESULT0,
request30000hns with no alignment retry or fallback. These native buffer/period
values are not measured end-to-end latency or total route queue depth.

## Launch, collection and retained faults

The first launch refuses the busy guard before task registration. Its audit
retains no task, run directory, launch receipt or outer log. The unrelated
`D:\moiren-two-tier` workload is preserved rather than killed. This refusal
is not a native audio start or another failed waveform acquisition.

The acquired result records session1 and script UTC10:52:46.7541879--
10:53:24.4252194; the task terminal poll is Ready/result0. Probe exit is confirmed,
without forced termination. Collection, source submission and cleanup complete;
runtime/cleanup failures are null, no owned processes remain, root PnP is OK,
driver Running and boot unchanged. Render master changes are empty/default roles
unchanged in the runner's catalog comparison, not continuous/all-path evidence.

`quality_failure="microphone provider silence"` remains. The acquired collection's
`graph.microphone_provider.snapshot` is Running/gate1 with silence_packets9;
host telemetry retains10103 live completions,10104 joined completions and
silence_frames/packets0. Final raw Removed/Closed still retains9. These are
distinct counters, not contradictory
permission to report zero faults or replace the total with first-observed3.

Bounded startup QUERY observations retain0 ->3 ->7 ->9 while retained Playback
state is Paused, before the first Playback Running observation. Their control
QPC read windows do not identify exact increments/native Start times or prove
retained boundary probes refreshed during priming; missing endpoints are unknown.
`probe.stderr.log` lines10--14 record MIC Paused0 -> Running3, then
`microphone_reprime_budget`, then Running3 -> Running9, before the Playback
Paused -> Running observation. These lines lack timestamps: they establish
logged observation order, not exact callback intervals or causal attribution.
They do not locate the driver zero-fill increments in recorded PCM. There is no
wholesale "tail only" classification, startup exemption or quality waiver.
Cleanup epoch reset/recovery1 per provider remains separate from live reset0;
initial prime-muted32 and unknown boundary observations15 are retained.
`quality_complete=false` is literal producer policy, not inferred to be caused
by unknown15. No cumulative startup fault or nonzero final counter is erased.

The archived MIC sink retains10113 complete144-frame packets,1456272 frames,
zero sink sequence gaps/missing/incomplete/nonfinite counts and failure null.
Packet flags are0, not proof of nonzero samples or complete waveform continuity.
Independent morphology finds finite, bit-identical stereo,31441 nonzero samples per
channel, peak0.04945851489901543; artifact SHA256
`810983A5AE598AAEC05A4A6E24E709E30C59DEF28BB9973B7B9ED0347BBF6B3D`.
The9867 wholly zero packets and zero last packet coexist with flags0; scheduled
zero regions are not automatically faults and do not waive raw MIC silence9.
The generic capture `source="physical"` labels its endpoint-capture API path;
the resolved endpoint is the owned virtual MIC, not a physical microphone.

The authenticated unchanged nominal analysis reports4 unique identities21/26/27/28 and
25 missing detections: **not25 proven dropped markers**. The9616 nominal
packet-QPC-policy invalidations are not9616 lost packets. The existing29 windows,
thresholds0.7/0.05/0.0001 and exact-nominal assumptions are retained, not tuned
to this PCM. Finite/nonzero PCM is not an indexed-marker or latency pass.

Native-first retirement retains successful Stop and native join before MIC gate
close. The last packet QPC label falls after stop broadcast and before worker
stop-observed; this is not callback/arrival identity or a mapping of any driver
silence increment to a PCM packet. No WPP trace was acquired.

## Original archive refusal and V2 correction

The original archive helper refuses `Archive byte bound exceeded` at its64MiB
aggregate bound. `native-archive.stderr.log` and
`archive-size-refusal-audit.json` retain the failure and original run files.
That audit records no V1 ZIP, manifest or archive receipt. V2 changes the aggregate
payload cap to128MiB, retains40MiB/file and no-overwrite guards, and uses a new
terminal snapshot while preserving original helpers/snapshot/refusal receipts.
`archive-iobus-v2-freeze.json` retains35/35 host fixtures including large-payload
roundtrip and cap refusals. This is archive capacity correction only: **no native
rerun, original PCM change or audio/quality threshold adjustment**.

`interactive-iobus1-evidence.zip`:2182554 bytes,37 payloads plus manifest,
SHA256 `62AF10688AEA489A466CA72C63000ECFE7B73BD429104623BB319967564A4431`.
Manifest9883 bytes, SHA256
`7840314722807F0A7951AD415F889C32A68060BCD91E8B40F2A24911081954B5`.
Root verification records archive/provenance verified, `present_unqualified`;
extraction is `interactive-iobus1-verified`. `root-loose-archive-equality.json`
confirms exact bytes for PCM, packet JSONL, collection.json and report.json.
`independent-interactive-iobus1-acquired-audit-final.json` is23061 bytes, SHA256
`9AD201A960E8ECC7492A82C6D21A0E8BC2A71E57DE6F234260FDCB19B63EBC53`.
Its93/93 checks authenticate archive/provenance/context/route/PCM/packet/counters
and the unchanged nominal analysis; `qualified=false`, `quality_passed=false`.
The original92/93 audit and failures remain: its checker incorrectly required
config amplitude0.025 to have native-outcome Float32 serialization
0.02500000037252903. Only that checker assumption is corrected, not evidence,
templates, tolerances or thresholds. Collector0/integrity does not erase faults.

## Post-archive cleanup

`post-iobus-cleanup-freeze.json` and root's actual guard receipt retain38/38
PS5.1 host fixtures, no native execution by fixtures. The11:10:47--11:10:50
`root-post-iobus-readonly.json` passes all guards, comparing six default roles
and12 render master scalar/mute values with the pinned baseline.
`root-post-iobus-cleanup.json` removes only exact terminal root-path task
`Moiren-InteractiveIOBus-waveform1`, with all guards passed and `Error=null`.

At11:11:45, `root-post-iobus-cleanup-verification.json` retains task absent,
owners empty, no raw children in PnP inventory, root OK, driver610 Running with
unchanged on-disk SHA/boot. Free bytes are216770719744 on D: and9531121664 on C:.
The helper unregisters the owned task only: no run/archive/file/cache removal,
process kill or volume/default write. Catalog equality is before/after evidence,
not continuous/global state proof; disk hashes do not attest a loaded image.

## Unimplemented next-step hypothesis

`root-mic-startup-hypothesis.json` records source inspection, not a proven cause.
At frozen5f9 source, `vad/provider.rs:37` polls idle state every10ms; its
Running/open/admitted branch at1124 prepares the bridge and `PcmOwner` before
first WRITE. `vad/pcm.rs:154` constructs four request slots. The first observed
driver silence3 coexists with zero host submission attempts; later9 is logged
before Playback Running. Logs do not time those allocations or each callback.

A candidate is to preallocate an exact admitted Paused MIC owner, then WRITE
**only after fresh Running validation**, with bounded shorter startup observation.
This is neither implemented nor validated here; it must preserve identity,
gate/epoch freshness, cancellation, error paths and steady-state allocation
rules. No accepted-WRITE-before-Running, silence waiver, latency improvement or
root-cause conclusion follows from this proposal.

## Goal and evidence limits

Defaults, budgets and quality thresholds are unchanged. S1-S3 remains incomplete;
this software route cannot establish calibrated physical latency or a stable
sub10ms result. The user preference remains stable real output below10ms. Only
evidence proving that impossible under current device conditions permits a
defensible theoretical minimum/best achievable stable fallback. A failed run,
API minimum, nonzero artifact or current model does not establish impossibility.
No fallback qualification, default promotion or quality waiver follows here.
