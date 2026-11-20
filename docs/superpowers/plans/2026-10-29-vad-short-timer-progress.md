# VAD Short Timer Progress

**Goal:** Close S2's software short-packet timer starvation under modest repeated
callback lateness without clamping away elapsed time or replaying long stalls.

**Root cause:** `PlanTimerDrivenStep` rebases to zero whenever elapsed pending
frames exceed one logical packet. At48/49 frames, every1.1ms callback observes
52/53 frames, invalidates PCM and advances no samples. Existing exact1ms tests
do not cover this explicitly recorded TODO.

**Design:** Keep the existing one-packet processing budget, one PCM transfer
and three logical/native spans per callback. Only packets<96 frames permit at
most one packet plus one48-frame timer quantum of pending elapsed time, capped
by actual mapped frames. Packets>=96 retain their old threshold. Process `min(pending,packet)`
without changing the epoch; request a minimal due time while residual debt
remains. After debt drains, schedule against the original clock association.
Beyond this explicit bound preserve zero-progress rebase, quiescence and
counted discontinuity. Every callback stays bounded; the OS need not service a
minimal due time immediately. Sustained callbacks slower than available
throughput still produce counted overload, not a lossless-output claim.

Independent design review rejects a proposed two-packet callback: the provider
has one pending microphone WRITE, so a second capture packet start would have
no guaranteed supply. No completion batch expansion or packet-size relabeling
is allowed. The actual external packet shape, queue defaults and app whole-wake
admission stay unchanged. Timer first-rebase evidence must use the same pending
threshold. This is bounded jitter recovery, not VM grace or a hardware promise.

**Constraints:** No VM/native audio/driver install/deployment. Host WDK analyzed
build and native INF validation are required for this kernel-source slice.
Keep local atomic commits and batch remote updates; preserve `gate.log`.

## Planner And Segmented PCM

- [x] Reproduce48/49-frame repeated1.1ms callbacks with a failing production
  planner/PCM test in `driver/vad/tests/timer.cpp`; verify both directions,
  payload order, timely residual service, counted overload and bounded work.
- [x] Add pending-debt helper in `src/audio/Clock.h`; preserve segment admission
  and the fixed1ms timer scheduling quantum.
- [x] Preserve exact threshold/one-frame-over stall and post-stall recovery,
  duplicate timestamp idempotence, all format widths and ring wraps.

## Production Callback And Diagnostics

- [x] Add failing extracted callback regression for short-packet progress,
  one-packet work bound, no false invalidation and real over-threshold stall.
- [x] Keep `src/Driver.h`, `src/Ownership.h`, `src/Pcm.cpp` and callback span
  bound unchanged; retain existing request completion/ownership contracts.
- [x] Update `src/audio/ClockDiagnostic.h` threshold using the planner helper;
  test actual extracted callbacks and same-QPC retry without catch-up replay.

## Verification And Local Preservation

- [x] Run all10 portable MSVC driver suites, WDK `build.ps1 -Analyze`, INF
  verifier, independent review and diff checks; fix reproduced findings.
- [x] Record exact bounds, tests, package hashes and unqualified target gates
  in validation ledger and canonical driver status; make scoped local commits.
- [x] Leave this batch local pending a meaningful integration push. Do not
  install any generated package or infer target stability from host suites.

Full S1-S3 and stable physical IO/Bus below10ms remain unproven.
