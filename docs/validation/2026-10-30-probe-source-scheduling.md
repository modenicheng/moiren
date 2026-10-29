# Native probe source scheduling

The example-local external playback source is shared by VAD-exclusive
source generation and the standalone Shared Process source worker. Other
capture/render/provider audio owners already register an Audio MMCSS task;
this source owner previously did not.

The [simultaneous DAWN source/app recording](2026-10-30-dawn-native-process.md)
retains a matching approximately2.11-second marker interval on both sides
of the graph. Source acknowledgement also falls behind nominal48 kHz
progress despite zero100ms event timeouts. The graph is not the origin of
that particular upstream gap; starvation of the normal-priority source
worker is a hypothesis requiring a matched native follow-up.

The repair registers Audio on the actual source owner thread before
publishing ready/starting the native client. Its guard remains alive through
explicit Stop and reverts on the same owner thread. `PlaybackInfo` now
retains `mmcss_registered` and `mmcss_hresult`, independently of whether
Start/Stop succeeds. As with the other owners, registration failure is
best-effort and retains its HRESULT rather than inventing successful
scheduling. No queue, period, gain, production driver or default changes.

The actual serialized-info regression first fails because the registration
field is absent (`null` versus expected`false`). After repair the full probe
suite passes519 tests; strict example Clippy passes. Regressions cover
registration reporting independently of Start, failure retention and guard
retirement. Those software receipts do not prove native registration,
elimination of source gaps, continuous output or physical latency.

## Bounded native follow-up

Source81eb641 is deployed beside the original probe; app3ac8484 and
recorder9313364 remain unchanged. Both ready and final receipts confirm the
actual Audio registration succeeds. Host and remote release example suites
pass519 tests, and independent review finds no issues.

The same source/app dual recording after registration finds35 markers in
each34.98 nominal seconds of retained PCM. All source intervals are48,000
frames; graph intervals are47,984..48,058. Neither recorder reports artifact
loss, discontinuity or timestamp errors; Process position-gap detection is
inapplicable. The seconds-long interval from the original paired run is
not reproduced in this finite window. See the linked DAWN receipt for exact
durations, hashes, serial-comparison limits and normal retirement. This
supports keeping the source repair; it does not prove a hard scheduling
guarantee, qualify lower buffers or supply a physical calibration.

Repeating the two smaller VAD budgets with the repaired source likewise
retains zero in-run playback xruns/short reads and zero provider sequence
holes in both35-second windows, compared with15/8 xruns and72/56 missing
slots before. The144-frame source case still has one cumulative driver
drop appearing at the first post-start sample; its in-run driver-drop delta
is zero. Preserve that startup fault. Neither diagnostic acquires PCM or
passes the full waveform/physical gates, and defaults remain unchanged.
