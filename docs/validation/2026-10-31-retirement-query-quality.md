# Final retired QUERY quality accounting

Date: 2026-10-31 (Asia/Shanghai). Branch: `feat/minimal-io-latency`.
This is a host-side S1/S2 evidence-accounting repair, not a transport fix,
native replay, lower-buffer qualification or physical latency measurement.

## Reproduced gap

The existing native microphone-retirement receipt retained live driver
silence_packets=0 and final Removed silence_packets=1, while its collection
reported quality_passed=true and quality_complete=false. Fresh read-only
inspection of the original archived `collection.json` confirms the same
endpoint/PDO/session/export identity, gate epoch1 to6 and that counter change.
The archive is under
`D:\coding\moiren-evidence\20261031-mic-retirement-order\target\capture`.
No historical evidence or result was rewritten.

`CollectionGuard::finish` already joins owners and retains final provider
QUERY observations. The collector previously validated the final graph
summary but not the final QUERY silence/drop counters. Therefore faults
observed after its final Running check could escape the quality conclusion.

## Change and invariants

The collector now checks both owned endpoints after retirement and before
serializing its result. Physical routes check Playback only. It requires
final Removed evidence, unchanged endpoint/PDO/session/export identity,
proper unsigned nonsaturated counters and no counter regression. Native
stream/gate epochs may advance during Stop; no new live epoch is admitted.

Any cumulative driver silence/drop is retained as a quality failure, including
faults already present in the last live observation. Correctness collection
can remain operationally complete while quality_passed becomes false;
latency-sensitive collection refuses the fault. Missing/malformed final
evidence is a hard error in either mode. Cleanup status is not rewritten as
an audio-quality verdict. First fault evidence remains latched, including
both final QUERY observations when the first fault occurs at retirement.

No audio thread, driver, scheduling, buffering, default, counter or acceptance
threshold changes. quality_complete and qualified remain false. This does not
attribute the tail increment to a specific callback, PCM sample or cause.

## Verification

- RED: the no-check implementation passed the zero control but failed both
  tail-fault and malformed/regressed-evidence regression groups.
- Final probe suite: 627 passed, zero failed/ignored, with `vad-probe`.
- Audio feature library: 679 passed, two ignored, zero failed.
- Strict probe Clippy, workspace rustfmt check and diff whitespace check pass.
- Five new regression groups cover Playback/Microphone, both policies,
  silence/drop, absent/malformed/saturated/regressed counters, zero controls,
  cumulative fault preservation, first-fault retention, export identity and
  allowed native Stop epoch advances. Tests open no native audio clients.

Logs: `target/retirement-query-probe-final.log` and
`target/retirement-query-library-tests.log` in this worktree. An earlier
overlapping verification saw formatting during an in-progress edit; the
final complete check was rerun successfully against the formatted source.

A read-only SSH inspection of `moiren-lan` at20:53:25 returned no process
named moiren-app/e2e_latency_probe/vad_native. It is only an instantaneous
inventory, not a reservation or authorization to interrupt another owner.
No driver deployment, native playback, volume/default changes, process
termination or evidence deletion occurred.

## Remaining S1-S3 gates

Collect matched ClockRebase/no-copy evidence for the later gate transition
before another production timing fix, as required by the startup-poll receipt.
Changing-waveform continuity, sustained lower-budget delivery, matched
VAD/Process routes and calibrated physical latency remain unproven. The
current-device theoretical-minimum fallback also remains unproven; neither
a silent buffer nor an API period establishes impossibility below10ms.
