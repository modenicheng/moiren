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
