# MIC startup follow-up: matched native deadline rebase - 2026-10-31

## Scope and verdict

The single `mic-startup-trace2` acquisition fails collection, quality and runtime
cleanup. Its retained first Clock rebase and no-copy witnesses identify a
Running/open MIC gate1->2 transition and the first driver silence increment
in the same callback. This supplies the direct native trigger missing from
the [startup-poll receipt](2026-10-31-mic-startup-poll.md); it does not identify
why dispatch was late or explain every subsequent cleanup/silence fault.

The audio executable remains frozen at `99685052997e971c9aacf4cc19c20765ad1f8632`,
not the newer `0ef5a29` final-retired-QUERY repair. Only the WPP task observer
integration changes relative to trace1. No driver, audio period, queue, startup/steady scheduling,
quality gate or default changes. There is no physical render client.

## Observer repair and retained predecessor

Evidence roots are `D:\coding\moiren-evidence\20261031-mic-startup-trace` and
`D:\coding\moiren-evidence\20261031-mic-startup-trace2`. The preexisting trace1
audio run and failed observer are preserved, not replayed or overwritten.
Its `GetNewClosure()` block cannot resolve script-local `Read-WppTask`; the
trace stops about123 seconds before delayed interactive audio launch. Its
8192-byte ETL decodes four events and does not cover audio. The existing
148-check acquired audit is an integrity/classification result, not coverage.

The frozen trace2 wrapper explicitly captures `${function:Read-WppTask}` and
invokes that scriptblock inside the closure. Actual PS5.1 integration fixtures
retain RED3 pass/5 fail and GREEN8/8; WPP guards21/21, runner48/48 and archive
fixtures54/54 pass. Observer repair is not an audio timing fix.

Trace2 runner SHA256 is
`E9A4ED7288A702005D652711CAAE85534D20DC9370AC0C73BFDCEDCB4EE86346`;
wrapper SHA256 is
`0F70408B2FE5A03E753CFD545F580867AD2637D8C66057C2B9D5945E61494847`.
All runner-freeze27, stage9 and helper-freeze29 pins are checked before use.

## Frozen conditions and failed acquisition

- VirtualPlayback / speaker = Input/Source -> unity Bus -> VirtualMicrophone /
  MIC = Output/Sink;48k stereoFloat32, Engine32, no extra DSP.
- Playback Exclusive480; MIC queried/actual minimum Exclusive144.
  Target1024/capacity4800/jitter96, declared packet/gap480, recovery cap8.
- Indexed-v2 source29 identities over30s, correctness3000ms and60s ceiling;
  correctness grace does not authorize an invalid epoch or qualify latency.
- Probe7437312 bytes, SHA256
  `69BB890BDBEADF2F23B994EA70AFDAC225208BD47455CB63746CA83464EB0C77`.
- Unchanged driver610/source `883a5be0a66afa03ef5d2beb2d3c4063cda1bda6`, SYS SHA256
  `94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`,
  boot2026-10-30T09:50:49.6598450+08:00. No driver reinstall or reboot.

One task request completes terminally; controller, task and probe exit1, and
the WPP session stops. The first failure retains MIC identity/gate/shape
change and `slot=2 HRESULT=0x8007051A` shutdown failure. Source acknowledges
303840/1440000 frames; the schedule is incomplete. Capture stores2146 packets /
309024 frames. Clean stored packet flags do not establish waveform continuity.
Collection/cleanup remain incomplete; live MIC silence7 and Removed silence8,
dropped0, are failures, not exemptions. Playback silence0 cannot erase them.
Native capture Stop/worker join succeed, separately from host cleanup failure.

## Matched callback, not merely adjacent messages

`root-trace2-decode/driver.txt:106-108` retains the three fragments with key:
endpoint26, session1266750799707, generation1, stream46173801378424,
callback QPC1266827998581. Cause is1 / `EventDeadline`, frames144, packets2,
completed2139. Its next-packet deadline is frame308160 / QPC1266827985574;
lateness13007 ticks at10MHz is1.3007ms.

Before: slot generation1, gate1, open1, state2 / Running, clock running1,
epoch1266763785574, base0, limit308016. After: same slot/state/open, gate2,
epoch1266827998581, base308016, limit308160. This is native clock rebase,
not a format change or an intentional Stop inferred from host sampling.

Lines109-112 identify the same endpoint/session/generation, gate2/native2138
and callback QPC; no-copy silence changes0->1. Event gate2, packets2/index0,
Float32/stereo144 and successful Notify are retained. The unserved fragments
at113-116 share that identity and state QPC1266827998580, with callback-entry
QPC1266827998579 and serviced deadline1266827955574. Thus callback observation
is4.3007ms after the current serviced deadline, while the rebase compares
the NEXT packet deadline. Entry-to-state is only one tick in this callback;
this is not evidence of a large lock-acquisition delay.

Retirement call windows start later (Playback close before-QPC1266828183506).
This orders the first native rebase before intentional teardown; it does not
locate every later no-copy or make cleanup errors harmless.

Production anchors: `audio/Clock.h::SchedulePacket` checks the next deadline;
`audio/Streams.cpp::VadAudioTimer` rebases/invalidates before transfer;
`Children.cpp::VadAudioRebaseDiscontinuity` records before/after under the
shared lock; `Control.cpp::TracePendingClockDiagnostic` emits the triplet.
`ClockDiagnostic.h` retains only the first rebase per stream generation.
High-resolution WDF timing is already enabled; neither source nor this trace
proves the scheduler dispatch cause. Rebase/epoch safety must not be weakened
to turn a failed run green.

## Archive and interpretation limits

`mic-startup-trace2-evidence.zip` is1946420 bytes,44 payloads plus manifest,
SHA256 `2F78001996126D71F1123AE438D69F27F66ADDBE7CF76982748A537F4DA8A7DC`.
Manifest11852 bytes, SHA256
`AE356465B25821DA9065A45FE7A0EC4D47E128421B195442B0B09AB9882FFA5D`.
The root verifier authenticates CRC, exact members, lengths, hashes and pins
before extraction into `mic-startup-trace2-verified`.

The73728-byte ETL has SHA256
`5E4E92A3F16C3198C2889730D225AA016EF2510A4C566C349E723FF0AF02C3EF`.
Matching driver610 TMFs decode170 events, reported EventsLost0, format errors0
and unknowns0, with actual audio coverage. The raw wrapper's TraceLoss remains
unknown; decoder statistics alone do not prove all-interval completeness.
Neither successful archive verification nor clean decoding accepts audio.
The frozen inner runner still serializes `WppEnabled=false`; tracing is owned
by the separate pinned outer wrapper, whose ETL/controller/stop artifacts are
archived independently. Do not rewrite that inner field as a coverage claim.

Exact terminal-task cleanup is separate from runtime cleanup. Its first
read-only guard refuses because unrelated `D:\moiren-two-tier` Cargo/rustc/link
owners are present. Those owners are not stopped, and no task is unregistered
by that refused check. A second check at13:12UTC also refuses. After those
owners finish independently, the13:16:11UTC check passes every guard; the
13:16:27UTC execution repeats identity/archive/catalog/driver/boot/quiescence
checks and unregisters only `Moiren-MicStartupTrace-waveform2`. Six default
roles and12 render master scalar/mute snapshots match; no process, evidence
file, build cache or driver is removed. Native acquisition is terminal and must
not be repeated because a root console-print operation encountered a GBK failure;
raw outputs and command receipts were already saved.
The13:19:09UTC post-check confirms the exact task and trace session absent,
no native/build owners or raw children, root PnP OK, unchanged driver/boot and
matching six roles/12 master controls. Root rechecks all45 ZIP members and44
extracted payloads byte-for-byte, preserving the failed runtime verdict in
`root-acquired-post-audit.json`. Task removal is not successful runtime cleanup.

## Reproducible host verification

`scripts/vad-clock-rebase.py` performs an offline strict join, not a timing
change. It requires complete unique rebase triplets and no-copy quartets,
matching endpoint/session/generation/callback with an unambiguous stream,
the exact old-clock next deadline and lateness, native packet/shape/event/
Notify identity, Running/open gate advance and first silence0->1. It rejects
duplicates, missing/mismatched/ambiguous fragments and malformed fields; it
does not interpolate absent evidence. Successful JSON explicitly preserves
`qualified=false` and `all_interval_trace_complete=false`, pins the decoded
input SHA256 and uses create-new output. Its exit0 means association only.

TDD retains the absent-analyzer RED (nine tests,51 failing subcases), followed
by nine passing strict-join tests; two further CLI tests cover create-new pin/
preservation and refusal without a success receipt. CI runs the offline suite
without devices. The actual trace produces exactly one association with the
QPC/packet/gate/silence values above. Receipt
`root-trace2-decode/matched-rebase.json` SHA256 is
`9BB72E85A0B3E2973351E8B21003034649CD9739CE23212113748E811E5A1506`.
Fresh `pwsh -NoProfile -File driver/vad/test.ps1 -Test clock_diagnostics`
passes real extracted callback/epoch causality and passive WPP checks with
fixed216-byte diagnostics; this does not exercise target scheduler dispatch.
The full17 driver host suites, all11 Python tests, Ruff, workspace rustfmt
and diff whitespace checks also pass. No Rust DSP/transport source changes.
The clock suite now replays the retained MIC144 QPCs through extracted actual
callback/rebase code, plus a counterexample with identical rebase/planning
time and43006-tick State wait instead of one tick. Both retain one bounded
transfer/Notify and the next3ms arm. This is a source-level regression, not
native validation of a new binary or a scheduler guarantee. Fresh feature
library679 passed/2 ignored and probe627 passed/0 ignored; logs are under
`target/mic-startup-clock-rebase-20261031-212232`. These current host results do
not change the frozen9968505 native provenance or failed acquisition.

Source tracing of the terminal fault finds two revision-refusal paths:
reservation can reject an old admission, and transfer completes retrieved
old-epoch requests with `STATUS_REVISION_MISMATCH` before searching the same
bounded loop for a current request. This trace does not identify which path
produced slot2's retained HRESULT. `pcm.rs::shutdown_slots` intentionally
preserves every non-cancel terminal outcome after draining; changing that
error to cancellation or success would hide the failure rather than fix it.

The next discriminating scheduler diagnostic needs the preceding actual
timer-arm interval/due intent as well as callback-entry/State/planning QPC.
Current evidence separates entry from State at the fault, but cannot separate
late rearm/previous callback work from late native dispatch. Already-enabled
high-resolution timing is not a new remedy. Do not infer a buffer/period fix
from this absence or call software timing the physical-device lower bound.

S1-S3 remain incomplete: delivered indexed waveform, sustained lower settings,
matched VAD/Process conditions and calibrated physical measurement are open.
This run establishes neither stable sub10ms nor impossibility, a device
minimum or a path winner. No production scheduling fix follows without further
dispatch/service and invalidated-epoch cleanup evidence.
