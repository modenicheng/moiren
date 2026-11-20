# Bounded actual encoded VAD MIC reference

Date: 2026-11-02. Baseline: `5e2cadaef24eb2258ee66430d5f19db7e5c68270`.
This closes an S1 instrumentation gap, not S1-S3 qualification.

## Missing evidence and change

The [input reference](2026-11-02-vad-input-reference.md) retains actual bytes
before Playback decode. It cannot establish what the graph's MIC bridge
actually encoded, which WRITE was accepted, or which completion was observed.
Nominal source schedules, graph counters and host submission QPC cannot fill
that gap.

An explicit `--vad-output-capture-frames 1..480000` now opts a non-soak
`live-vad` MIC route into an independent bounded output witness. Omission
preserves existing behavior. The selected-interface constructor reserves both
directions independently before connecting on the caller's one control root.
The output collector copies the exact post-encode bytes before WRITE,
including any actual bridge zero-fill, without changing existing fences,
consumption, pacing, queue capacities, recovery or error policy.

Limits are 480000 frames, 4 MiB and 4096 records, with whole packets and
preallocated storage. Each record separates encoded attempts, fenced attempts,
accepted/rejected submissions and observed terminal completions. Tracking uses
the complete opaque ticket, not a reused slot index. Identity includes the
actual owner, admission/revision/reset and negotiated shape. Reopen or observed
identity change seals new bytes instead of merging different owners.

Internal owner retirement does not expose every per-ticket terminal result.
Unresolved pending entries therefore remain `retired_completion_unobserved`;
retirement preserves errors already observed. Untracked non-pending completion
observations are counted without labelling pending polls as stale audio.
Echoed WRITE-header sequence/QPC remains host-submission evidence, not a native
callback, sample time, recorder mapping or physical timestamp.

Only actual worker join permits detached `vad-output.raw.bin` and
`vad-output.metadata.json` export. A joined failed worker can retain partial
bytes; empty remains unknown; persistence failure remains unavailable even if
a raw file exists. Output aliases are refused before directory creation and
existing files are never overwritten. Peer exporters and cleanup still run
after an export failure. Every new report stays explicitly unqualified.

## Software verification

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-vad-output-reference-20261102-2045`.
`exact-export-final.json` verifies a clean 1360-file archive plus exactly eleven
hash-pinned intended overlays: eight modified and three new files. All 1352
non-overlay baseline files are byte-identical; no extra paths are admitted.
The 27 original dirty/cache files are excluded from these builds.

- Genuine assertion RED precedes collector implementation and probe flag
  support. Review findings reproduce error-overwriting-on-retirement and
  pending-poll miscounting before correction. Fifteen collector tests pass.
- Six actual-pump/session tests cover PCM16/Float32 encoded bytes, zero-fill,
  quota recovery, drops/wrong shape, post-encode fencing, in-WRITE revocation,
  enabled/disabled equivalence after capture capacity, prepared no-allocation
  and actual stop/Drop/panic join gating. A separate disabled-hook mutation
  fails the raw-byte assertion, then the restored implementation passes.
  The first mutation's compile error is retained, not counted as behavioral RED.
- Final Windows audio library: 790 passed, 4 ignored. Exact example suites
  feature-off/on: 709 passed and 1 ignored each. Scoped strict Clippy, formatting
  and optimized probe build pass. Focused counts overlap these suites.
- Full Windows workspace with `moiren-app/gpui-test,moiren-windows-audio/vad-probe`:
  2336 passed, 9 ignored, 0 failed across 71 test/doc surfaces. All-target check
  and strict Clippy pass. This is host headless verification, not host audio,
  Linux/Miri or remote CI.

Independent read-only reviews find no concrete blocker in parent hook policy,
collector ticket/identity/bounds/retirement, or probe alias/join/error handling.
The first integration compile exposes two existing teardown test callsites
needing the new optional argument; strict Clippy then requests a final-use
borrow simplification. Those integration failures and their corrected checks
remain in the evidence directory.

## One bounded remote acquisition

Remote root: `D:\MoirenTest\20261102-vad-output-reference-2110`.
Probe SHA-256:
`17AFB122E008B8A254E30E332EA7540F14259A728E8659F2AD228B163FF2F03C`.
It is a clean baseline plus the exact eleven overlays, not an unmodified-commit
binary and not a new application or driver package. The pinned create-once
wrapper passes a separate dry-run before its one native execution. Both use
isolated environment directories and a retained child handle with a 90-second
deadline; there is no identical retry or parameter scan.

At 21:10:38.947-21:10:50.062 +08:00, Compatible Playback -> unity Bus -> MIC,
Engine64 and the six-second indexed schedule run once. The source submits all
288000 scheduled frames. The native child exits 1 normally in 11.112 seconds,
without timeout, forced termination or cleanup error.

| Witness | Shape | Frames | Packets | Result |
| --- | --- | --- | --- | --- |
| Actual Playback input | Stereo Float32 | 312000 | 650 | Joined retained prefix |
| Actual pre-WRITE MIC output | Stereo PCM16 | 325920 | 679 | 679 accepted, 679 observed completed |
| Independent native MIC recording | Stereo Float32 | 325920 | 679 | Recording retained; overall collection refused |

Both witness workers complete and join, retaining original identities and
`identity_changed` seals. All output records report current identity after
submission and zero bridge zero-fill frames. This is actual encoded/IO
evidence; the subsequent offline comparison below checks recorder values
separately. Different recording origins and prefixes are not aligned by frame
counts.

The native measurement retains one discontinuity flag and refuses collection
on its unchanged continuity/timestamp gate. Its recorder counters show zero
dropped/gap/overflow/silent/timestamp-error frames or packets. Source submission
and cleanup complete, but collection does not. Audit completion is not a
functional/quality PASS. The startup flag is not exempted and this run does not
reopen the deferred first-packet investigation.

The flag is on the first retained packet: sequence1, stored frame0, device
position480. All subsequent retained packet positions are contiguous. This
locates the observation but does not establish a Windows startup cause.
`native-counter-summary.json` separates pre-retirement counters from final
cleanup: both directions show zero resets/recoveries before retirement, while
final cleanup shows Playback2/2 and MIC1/1 reset/recovery counts. Those final
increments are retained, not hidden or attributed to steady-state faults.

All owned exports reach Removed/Closed, only the healthy control bus remains,
and no test audio/Python owner remains. Protected OBS/QQMusic PID-plus-creation
identities, loaded SYS and boot remain unchanged. Twelve selected receipt/log/
metadata files are transferred with individual length/SHA-256 verification;
raw PCM and the full SRC archive stay on the test machine.

## Closed-data waveform inspection

Remote isolated Python analysis produces `independent-origins.png`: raw
waveforms and 10 ms RMS for input, encoded output and recorder, with each
stream's own stored-frame origin clearly labelled. The plot is visually
reviewed; it does not overlay or fit unknown clocks. Input peak is
0.02500000037, while output and recorder peak are 0.04983520508. This
observation does not establish the cause of the input/output shape difference
or certify source-to-output waveform correctness.

The output and recorder have the same 325920 stereo frames. Decoding the
entire PCM16 output as sample/32768 yields exactly equal values to all 651840
stored Float32 recorder samples at stored-index zero. No search, shift, gain,
drift or tolerance fit is performed. The result is
`exact_stored_samples_equal_only`, not a sample-clock origin, native-time
association, complete source-route pass or latency estimate. The discontinuity
flag, collection refusal and unqualified result remain unchanged.

The small evidence-only helper has seven passing pure tests, including
shift/gain/single-sample inequality, nonfinite refusal and hashing the same
bounded provenance snapshot that was parsed. Genuine assertion RED records
precede the additions. An independent review finds no concrete issue in the
final helper and comparison call site. Final analyzer SHA-256:
`770A8DE1FA05E6024DDF22EDD76E7C62E61DA19F8B79CE8B784875173BA399F0`.

The first offline wrapper refuses before launching Python because a PowerShell
5.1 JSON array was nested. Its failed audit is preserved; a corrected wrapper
uses fresh v2 paths. The first successful descriptive run takes 6.316 seconds.
Review then corrects an unbounded provenance-file reopen and adds the new exact
stored-sample question; the final v3 offline execution takes 6.501 seconds,
exits0 and retains all thirteen original artifact hashes before/after. No new
native acquisition occurs for either offline correction. Six final compact
plot/summary/audit/owner files are independently length/hash verified. Final
safety audit has no test owners and unchanged protected processes/driver/boot.

## Remaining boundary

Successful bounded native witness acquisition and exact output/recorder stored
sample equality advance development despite the unavailable physical setup.
Source/input-to-output waveform interpretation, sample-clock mapping, the
earlier smaller-target failure and calibrated physical latency remain separate.
No quality threshold is relaxed, buffer default promoted, path winner declared,
or sub-10-ms result inferred. First-packet work stays deferred and the original
S1-S3 goal remains active.
