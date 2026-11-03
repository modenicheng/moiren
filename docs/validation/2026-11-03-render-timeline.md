# Bounded Native Render Timeline

Date: 2026-11-03, Asia/Shanghai. Baseline: `d50d443`.

## Scope And Development Decision

The existing target1280 recording is closed. Its frozen association found a
replacement-owner mismatch that cannot be explained solely as the final Stop
tail. This is not enough to specify a product audio fix. The production archive
previously contained encoded packet identities and ReleaseBuffer outcomes but
no per-packet clock/padding history. The latest-only padding observation cannot
reconstruct packet855 retrospectively.

This implements the original S1 actual-boundary observation requirement in
`docs/vad-low-latency-goals-2026-10-28.md`. It does not change DSP, SRC demand,
admission, owner pacing, queues, defaults, quality gates or error recovery.
First-packet investigation remains deferred. Missing calibrated physical
hardware does not block this independent software development.

Local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-render-timeline-20261103-1618`.

## Frozen Residual Localization

A separate offline locator reuses the original integer lag, gain/DC, exclusions
and thresholds. Calls to refitting/correlation entry points are forbidden, and
the original source ledger is asserted unchanged. No new audio recording,
sample trimming, resampling or alignment search occurs.

- Baseline: all1179residual frames are the final95941..97120native interval,
  starting at packet286offset165; associated OBS values there are zero.
- Replacement: first residual native frame287120 is exactly packet855offset0.
  Of98288remaining native frames,98282are residual, in7runs. At onset a fixed
  1024-frame block includes160exact-zero OBS frames. Later OBS remains nonzero
  but no longer agrees with the frozen sample-coordinate association.

This rules out an exclusively final-tail explanation for the replacement
association, not capture-path artifacts, clock displacement or another cause.
Successful native releases and these sample coordinates do not prove playback,
native underrun, product data loss or physical latency. No causal repair or
buffer adjustment follows from this analysis.

The six compact locator payloads total261552ZIP bytes, SHA256
`82F5C7E48DEE030F06F38614CE89FA482350B725B278D9785EF2A7200EC377DD`.
Raw audio remains remote. The inspected visualization is
`s3-target1280-20261103-1520\residual-location-compact\residual-location.png`
under the same parent evidence root. Original denominator/packet-timing results
remain in `2026-11-03-target1280-frozen-waveform.md`.

## Implementation Contract

An existing opt-in render-evidence attachment prepares a separate4096-event
SPSC ring and QPC frequency off the audio thread. The event ring payload counts
toward the existing8MiB combined frame-ring/staging preparation budget. No
timeline is installed, clock initialized or QPC read when the hook is disabled.

Each Start, Stop, Wait and Write method invocation records a scalar QPC envelope
and success/failure; Wait also records readiness/timeout. Write records requested
frames, its existing write-side GetCurrentPadding result where applicable,
explicit completed release size/HRESULT, associated completed evidence packet
and accepted-frame frontier. Zero submissions and failures before release retain
no invented packet identity. Exclusive padding, failed clocks and unavailable
outcomes remain unknown. Envelopes include method work and evidence publication;
they are not hardware presentation timestamps or isolated COM-call durations.
No extra GetCurrentPadding calls or per-sample clocks are introduced.

The existing native lease is released once. Encoded bytes are copied before
release; only owned staging/scalars are accessed afterward. A staging refusal
cannot attach a prior packet identity or gate/retry audio submission. Event ring
overflow, reader abandonment, missing/regressed clocks and counter saturation
are explicit and independent of the PCM ring. Closure neither allocates nor
reclaims storage; both endpoints retain their storage until control-side join
and retirement, including native destructor failure.

The control-side archive independently drains `native-render/timeline.jsonl`
with bounded batches. Unknown/regressed clocks are preserved, not repaired.
Sequence gaps remain visible and refuse timeline completeness. A sticky
timeline file/IO failure cannot discard or prevent closure of the independent
`samples.bin` / `packets.jsonl` archive. The existing per-owner512MiB disk cap
is shared, including reserved failed-write tails. Final receipts distinguish
audio persistence from timeline persistence and require a joined final summary
and complete bounded drains. Finalization streams both sidecars, at most4KiB
per line and512MiB per file, to reconcile every Write packet identity, size,
HRESULT and accepted frontier with the persisted PCM packet ledger. Missing,
duplicate or contradictory correspondence refuses timeline/combined completion
without discarding the independent audio archive. An empty timeline cannot
certify a nonempty PCM archive. Persistence is never a quality/latency pass.

## Verification Ledger

Tests run from a clean `git archive d50d443` source with only explicit intended
overlays, excluding the27preexisting dirty/cache pins. Native/audio/UI activity
on the development host is prohibited; these are build, pure and headless tests.

- Portable timeline RED: absent types/APIs, exit101, `timeline-red.log`.
- Native envelope RED: absent operation helper, exit101,
  `native-timeline-red.log`. The first portable GREEN attempt also records an
  unused-mut compile failure; it is retained, not hidden.
- Initial focused GREEN:46passed/1native ignored, `timeline-green-v2.log`.
- Audio library:860passed/5native ignored, `audio-lib-v3.log`.
- Independent RT harness:256passed, `rt-v3.log`, including event overflow,
  closure and PCM independence with zero allocation/deallocation assertions.
- Strict audio all-target Clippy: exit0, `audio-clippy-v4.log`.
- App archive lifecycle RED/GREEN: initial34render/51cursor tests. Independent
  review found missing cross-stream reconciliation; a second behavior RED
  demonstrates wrong packet ID and empty timeline falsely passing before the
  fix. Updated app checks39render/56cursor tests. Original subagent tool outputs
  are extracted into `app-agent-red-green-extract.json` and
  `app-cross-red-green-extract.json` rather than recounted from memory.
- Pre-review full feature integration:2456passed/10native ignored across71
  harnesses, plus strict workspace/RT Clippy and both fmt checks exit0. Release
  v4 builds but is superseded by the cross-stream repair; it is not deployed.

At16:35:43 the remote residual-final audit verifies278pin entries across253
unique paths, including17locator pins and the locator manifest hash. Original
driver/boot, app hash, user OBS/QQMusic identities, default roles and render
settings are unchanged. No audio/analysis owner or running Moiren task remains;
the original task is Ready/result0. The1884-byte audit receipt hash is
`D57AE70A9708821628B01F8EC557E7E94C43DCD403549F874DEE17EBBE9B3271`.
Only the audit script and compact receipt cross the network, with no audio rerun.

Final cross-stream-repaired integration:2461passed/10native ignored across71
harnesses (`workspace-feature-v5.log`), strict feature workspace all-target
Clippy and workspace fmt exit0. Final focused audio evidence tests49passed/
1native ignored. Independent native/RT and archive reviews find no remaining
blocking finding after the packet-correspondence repair. Native early-error
branches are control-flow-reviewed; the pure envelope/release tests do not
claim to execute every concrete WasapiStream native branch.

Clean-source release v5 exits0:34350080bytes, SHA256
`B56B0AECD297A8BC708CAC4878D97FD6D7040184D09CCD751CB151A4BDAF3E75`.
The12intended code pins match the clean-source overlays;27preexisting dirty/
cache pins and unrelated main-worktree audit files are preserved. No push.

A distinct finite remote acquisition package is prepared alongside development,
with56synthetic Python tests and29wrapper checks on each PS5/PS7. It retains
the target1280 configuration and all owner protections; its only new question
is whether two production owners yield complete, reconcilable operation/packet
timelines. Pending binary/review pins forbid launch until finalization. Any
subsequent single native run requires its own acquisition/closure receipt;
no new instrumented native run is claimed by this implementation receipt.
Original S1-S3 qualification remains incomplete; physical latency is unknown.
