# VAD follow-up validation: 2026-10-21

## Result

The short combined target gate remains **FAIL**. Do not start or qualify the
eight-hour idle/PCM runs from these results. No silence, waveform, freshness,
coverage, or deadline acceptance rule was relaxed. No driver was installed and
no system audio default was changed during this follow-up.

Two source-level fixes were independently reviewed and committed in separate
slices:

- `bfe916ee6b15909b4e12023600426d6c2fe97394`: apply the existing bounded
  carrier/FIR clock-drift adaptation to virtual-only boundaries too. Native
  480-frame delivery and Engine 256-frame wakes have independent phases even
  when their nominal rates agree. Deterministic production-preparation tests
  exercise packet-phase changes; owner activation still does not wait for PCM.
- `eca8906336285359321addbe9548e9f1328a8a1d`: do not re-anchor an established
  microphone submission cadence on every delayed completion collection.
  Equality/overdue successful collection issues at most one freshly encoded
  packet. Expiry and epoch reset retain their separate behavior. This is not a
  kernel-admission guarantee and is not a claim that target silent packets are
  fixed.

The carrier-derived residency remains unqualified for negotiated latency.
There is still one Engine owner and no additional PCM queue. The original
half-period constant, one pending microphone request, four allocated rotating
slots, and driver submission-age checks remain unchanged.

## Target and evidence

The verified SSH guest is `DRIVER-TEST`, Windows 11, build `10.0.26200`, at
`192.168.111.128`. Use the dedicated key and account:

```powershell
ssh -i C:\Users\cheng\.ssh\moiren-test -o BatchMode=yes wdkremoteuser@192.168.111.128
```

The supplied `.1` address did not accept this key in the preceding access check;
it is not the verified test guest. The VM display name is `Windows 11 x64`,
distinct from the guest hostname. AC/DC standby timeouts are confirmed zero;
hibernation is unavailable/disabled. No known probe process remains after the
bounded runs. Service join is observed; it does not prove final PDO absence.

Evidence directories are under `C:\MoirenVadTest` in the guest. Seven runs were
also copied into the ignored local directory
`driver/vad/build/tests/20261021-soak-followup/`. Each retains launch metadata,
stdout/stderr, final report, and failed-window counts. Trial binaries below
were built with candidate patches, not from a clean committed source tree;
their exact SHA256 prefixes identify them. Do not relabel them as clean-commit
release or soak qualification.

| Evidence suffix (prefix `20261021-`) | Trial | Result |
| --- | --- | --- |
| `9C821861-phase-short` | All-virtual phase candidate | Two 48,000-frame exact signal phases and held-RUN Stop silence passed. |
| `9C821861-phase-rehearsal2` | Same phase candidate | 15s idle passed; PCM failed at 16.317s: 782,880 matching + 480 silent frames. |
| `F12152F9-timing-diag` | Additional timing diagnostics, logger filter did not enable them | 15s idle passed; PCM failed at 14.417s: 691,680 matching + 480 silent frames. Not timing attribution evidence. |
| `F12152F9-timing-diag2` | Same binary, corrected custom-target trace filter | 15s idle passed; PCM failed at 3.737s: 179,040 matching + 480 silent frames. |
| `9579E7C1-mmcss-diag` | Mic-only MMCSS experiment plus timing trace | Registration succeeded; 15s idle passed; PCM failed at 1.168s: 55,680 matching + 480 silent frames. Experiment removed. |
| `9A7219E3-cadence-rehearsal` | Initial cadence candidate | 15s idle passed; PCM failed at 17.508s: 840,000 matching + 480 silent frames. |
| `2C9087A0-cadence-rehearsal` | Cadence candidate with equality/overdue handling | 15s idle passed; PCM failed at 1.398s: 66,720 matching + 480 silent frames. |
| `8D412F54-epoch-diag` | Additional temporary non-RT reset observation | 15s idle passed; PCM failed at 5.726s: 274,560 matching + 480 silent frames. Both internal boundary epochs advance immediately before a zero-frame microphone carrier read. |

All these failed PCM windows record zero unexpected frames and zero capture
discontinuities. That is not success: each contains a disallowed silent packet.
The failed runs joined their service with no reported cleanup error. Temporary
per-packet trace, QUERY-duration logging, and the MMCSS experiment are not
retained in production.

## What the diagnostics establish

- In `9C821861-phase-rehearsal2`, microphone `SilencePackets` increases from 6
  to 7 during the failed soak, with no corresponding expiry or carrier-short
  event. This supports an empty native request queue for that incident.
- In `F12152F9-timing-diag2`, a completion is collected at local worker elapsed
  8,624,933 us. The phase deadline is 8,629,933 us; the next WRITE starts at
  8,636,126 us, 6,192 us late. Its async submission call takes 26 us. This
  measures user-side lateness, not kernel dispatch latency. Synchronous trace
  output may itself perturb this run; sleep, ready-thread delay, and logging
  are not yet separated.
- `2C9087A0-cadence-rehearsal` has a different signature: a microphone carrier
  read transfers **zero of 480 requested frames** during the soak, without an
  increment of the driver's silence counter. That means user-space encoded
  missing content; it does not prove the same kernel-queue fault. Stream/gate
  changes observed during subsequent client teardown are not evidence that
  they initiated the failure.
- The microphone sink prepares a free-space servo but has no playback-style
  content-reservoir priming step. This is a structural observation, not a
  proven explanation of the last failure. Full carrier reads can also carry
  upstream zeros, so absence of a short-read event is not signal proof.
- `8D412F54-epoch-diag` observes playback requested/observed epochs 8/7 and
  microphone 146/145 immediately before the carrier-short event, without a
  preceding native gate/stream change. This establishes an internal reset
  alongside the failure, not its initiator. The temporary log can repeat while
  a handshake waits; it is removed rather than retained as a production tracer.
- VirtualOwner invalidates every boundary when even one Graph period is
  skipped at a wake or after rendering. With 256 frames at 48 kHz, a period is
  5.333 ms. The playback reset clears 4800-frame carrier headroom and requires
  2544 frames to prime again (six 480-frame packet publications from empty).
  A small deadline miss can therefore amplify into a much longer silence.
  That mechanism is source-confirmed, but the owner skip is not independently
  recorded in this target run. Changing it requires an explicit bounded
  freshness/recovery contract; removing the reset or inventing a timeout is
  not justified by these observations.
- Management QUERY and PCM submission share the sequential root queue,
  precreated passive work item, and Admin lock. Their possible admission delay
  is unmeasured here. Async WRITE return and collection timestamps do not
  locate the request's kernel queue-admission time.

Next investigation should separate these boundaries using a bounded non-RT
timing recorder and short scheduler evidence, with independently advanced
native-tick tests. Do not try another phase constant, increase queues, weaken
freshness, or excuse failed samples as VM noise. Any later separation of PCM
dispatch from management requires its own cancellation/rundown review and
target-driver validation, not an incidental pacing patch.

## Host verification

- `cargo test --locked --workspace`: passed on the phase-fix tree; the later
  cadence edit was not in that command's compiled test image.
- Final cadence library suite: 377 passed, 2 intentionally ignored; PCM-worker
  subset: 31 passed, including observed RED-to-GREEN scheduling regressions.
- The final committed cadence release executable rebuilds to SHA256
  `2C9087A00D360B6F00AA690476D3ED5ADA52C95164247CF3E7A4E93B51F417BC`.
  This matches the cadence trial's executable bytes; it does not retroactively
  change that trial's recorded build provenance or failed result.
- `cargo check --locked --workspace --all-targets`, strict workspace Clippy,
  formatting, and diff checks passed on the cadence candidate.
- `virtual_boundary` integration tests: 2 passed, including allocation and
  deallocation guards. Integration-probe host tests: 48 passed.
- `driver/vad/test.ps1`: all six host suites passed, including production-source
  extraction for cleanup and REMOVE retry/failure propagation.
- `scripts/tests/vad-soak.tests.ps1`: report, cancellation, power failure,
  cleanup failure, and duration regressions passed without opening a driver.

These checks do not close eight-hour duration/resource-growth, repeatable
native matrices, shared/software-volume behavior, power transitions, Verifier,
100 lifecycle/crash rounds, installation/rollback, or production signing gates.
