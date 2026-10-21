# VAD follow-up validation: 2026-10-21

## Result

The short combined target gate remains **FAIL**. Do not start or qualify the
eight-hour idle/PCM runs from these results. No silence, waveform, freshness,
coverage, or deadline acceptance rule was relaxed. No driver was installed and
no system audio default was changed during this follow-up.

Three source-level fixes were independently reviewed and committed in separate
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
- `121118cf73f72a9bb016f35ad43ddec676cc7977`: a completed stale close may
  confirm the same stream is now closed. An asynchronous revision mismatch
  first needs an identical terminal result from a nonblocking completion
  check; only then can the control view issue one confirmation QUERY. The
  endpoint identity, stream generation and forward closed gate epoch must
  match. No SET_GATE is retried. Timeouts, uncertain completion, different
  errors, reopened gates and new streams retain their failure behavior.

The carrier-derived residency remains unqualified for negotiated latency.
There is still one Engine owner and no additional PCM queue. The original
half-period constant, one pending microphone request, four allocated rotating
slots, and driver submission-age checks remain unchanged.

Diagnostic commit `a6b16d1bb96b46b764eb69ccec16fc7f49525bb8` adds a fixed
64-event owner gap tail. The realtime thread only records existing clock
samples, frame positions, skipped periods and observation stage. The control
joiner emits the trace after the thread exits. A worker-side microphone
short-read log also samples QPC. Scheduling, reset conditions and PCM acceptance
remain unchanged. This is evidence collection, not a silence fix.

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
Separate passive present-device snapshots before and after these new replays
show only the healthy `ROOT\MEDIA\0000` control bus, with no present
`MOIREN\*` children. The after snapshot also records no `vad*`/`moiren*`
processes. These are momentary observations, not a 100-cycle absence or
resource-growth qualification. Both JSON snapshots are copied with the logs.

Evidence directories are under `C:\MoirenVadTest` in the guest. Follow-up runs
are also copied into the ignored local directory
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
| `eca8906-committed-pcm-short` | Committed source, matching rebuilt executable hash | PASS: two exact 48,000-frame signal phases; capture remains RUN during Stop and supplies 48,000 silent frames; service joined and control session released. |
| `eca8906-committed-rehearsal` | Same committed-source binary, fresh combined repetition | FAIL before PCM soak: 15s idle passed, but first signal phase lost active revision after a backend `GetOverlappedResultEx(VAD)` error `0x8007051A` (revision mismatch) during native startup/admission. Service joined and control session released. |
| `121118c-pcm-short1` | Committed concurrent-close fix, matching rebuilt executable hash | PASS: two exact 48,000-frame signal phases; capture remained RUN during Stop and returned 48,000 silent frames; service joined and control session released. |
| `121118c-combined1` | Same committed-source executable | FAIL: 15.068s idle and both signal phases passed; continuous PCM failed at 14.828s with 711,360 matching + 480 silent frames. User-side carrier read was 0/480; microphone driver silence count stayed at 4. Service joined and control session released without cleanup error. |
| `a6b16d1-pcm-short1` | Committed bounded clock diagnostics, matching executable hash | PASS: two exact 48,000-frame signal phases, held-RUN Stop silence, service join and control release. |
| `a6b16d1-combined1` | Same committed-source executable | FAIL: 15.120s idle and both signal phases passed; continuous PCM failed at 16.148s with 774,720 matching + 480 silent frames. Carrier read was 0/480; a preceding one-period owner gap is recorded. No cleanup error. |
| `a6b16d1-combined2` | Same committed-source executable, fresh repetition | FAIL: 15.061s idle and both signal phases passed; continuous PCM failed at 12.512s with 600,000 matching + 480 silent frames. Again 0/480 carrier read follows a recorded one-period owner gap. Service joined and control session released without cleanup error. |

The silent-packet failures record zero unexpected frames and zero capture
discontinuities. That is not success: each contains a disallowed silent packet.
The cadence-only committed repetition instead fails before the soak begins. All failed
runs joined their service with no reported cleanup error. Temporary
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
  That mechanism is source-confirmed, but the owner skip was not independently
  recorded in the earlier `8D412F54-epoch-diag` run. Changing it requires an
  explicit bounded freshness/recovery contract; removing the reset or inventing
  a timeout is not justified by these observations.
- Management QUERY and PCM submission share the sequential root queue,
  precreated passive work item, and Admin lock. Their possible admission delay
  is unmeasured here. Async WRITE return and collection timestamps do not
  locate the request's kernel queue-admission time.
- The cadence-only committed repetition exposes a same-file concurrent-close
  race: lifecycle closes microphone stream 1/gate 1 to gate 2, then transport
  receives revision mismatch on its stale close. That was an explicit native
  completion error, not normal zero transfer. The narrow confirmation fix
  above covers this race with production-used completion-state regressions.
  Its standalone and combined replays pass startup and both signal phases.
  The combined replay still fails quality later, so it does not qualify
  repeatable end-to-end stability.
- In `a6b16d1-combined1`, the owner's complete retained trace has 11 events.
  A `Wake` observation at QPC 536807677760 records one skipped period before
  requesting boundary resets. The microphone's post-short-read sample is QPC
  536807713944, 3.6184 ms later at the recorded 10 MHz counter frequency.
  The next owner gap is after this sample. The microphone driver silence
  count remains at its startup value of 6, and no expired WRITE is recorded.
  This supports the owner-reset/empty-carrier mechanism; it does not prove
  which reset emptied this read. Owner timestamps precede reset increments,
  and the microphone timestamp follows its read; preemption can intervene.
  `Wake` identifies the pre-render observation point, not a measured timer
  delay. These records do not distinguish scheduling delay from other CPU
  work, or attribute an exact boundary epoch to its requester.
- The fresh `a6b16d1-combined2` repetition retains all 9 owner events. The
  nearest preceding one-period `Wake` observation is QPC 538208597695; the
  post-short-read sample is 538208656281, 5.8586 ms later. The next owner gap
  follows this sample. No expired WRITE is logged. The microphone silence
  counter stays at 5 before the failure and is later observed at 6 alongside
  a paused stream and gate epoch 3 during teardown; that later observation
  cannot establish an initiating kernel starvation event. Both repetitions
  fail the unchanged quality gate, with the same timing-interpretation limits.

Next investigation should correlate the owner tail with bounded reset-requester
and epoch evidence plus short scheduler evidence, using independently advanced
native-tick tests. Do not try another phase constant, increase queues, weaken
freshness, or excuse failed samples as VM noise. Any later separation of PCM
dispatch from management requires its own cancellation/rundown review and
target-driver validation, not an incidental pacing patch.

## Host verification

- `cargo test --locked --workspace`: passed on the phase-fix tree and on the
  final cadence tree. The later concurrent-close and clock diagnostics edits
  are covered by the focused checks below, not retroactively by that run.
- Current library suite: 390 passed, 2 intentionally ignored. The concurrent
  close slice has 106 passing VAD tests, including terminal completion,
  pending-buffer retention, conflicting identity and reopened-gate cases.
  The PCM-worker subset has 31 passing tests.
- The final committed cadence release executable rebuilds to SHA256
  `2C9087A00D360B6F00AA690476D3ED5ADA52C95164247CF3E7A4E93B51F417BC`.
  This matches the cadence trial's executable bytes; it does not retroactively
  change that trial's recorded build provenance or failed result.
- The committed concurrent-close executable has SHA256
  `8589F5D562F61DD494636A711480C8E15C9415FC14AA993D172D9B1E4AE96BC9`.
- The committed clock-diagnostic executable has SHA256
  `70FFCA82BFC3AE1B139FEA590F0D83EE7ACA8D9995DE97E478F685516F5165A5`.
- `cargo check --locked --workspace --all-targets`, strict workspace Clippy,
  formatting, and diff checks passed on the cadence candidate.
- Strict workspace Clippy with both probe features, formatting and diff
  checks also pass on the committed concurrent-close/clock-diagnostic source.
- `virtual_boundary` integration tests: 2 passed, including allocation and
  deallocation guards. Integration-probe host tests: 48 passed.
- `tools/rt-check`: 79 passed.
- Clock-owner tests: 7 passed, including exact wake/post-render gap stamps,
  unchanged reset behavior and bounded ordered retention after ring wrap.
- `driver/vad/test.ps1`: all six host suites passed, including production-source
  extraction for cleanup and REMOVE retry/failure propagation.
- `scripts/tests/vad-soak.tests.ps1`: report, cancellation, power failure,
  cleanup failure, and duration regressions passed without opening a driver.

These checks do not close eight-hour duration/resource-growth, repeatable
native matrices, shared/software-volume behavior, power transitions, Verifier,
100 lifecycle/crash rounds, installation/rollback, or production signing gates.
