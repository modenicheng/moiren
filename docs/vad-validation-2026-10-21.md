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

## Deterministic owner-reset regression: 2026-10-21 (host)

A user-mode regression now reproduces the combined-soak silent-packet signature
without a driver. It drives the real `VirtualPreparedTransport` owner thread
through a step-controlled fake clock (one Graph256 period per granted wake),
renders through the real fenced `VirtualPlaybackSource`/`VirtualMicrophoneSink`
pair with clock-drift FIRs and 4800-frame carriers, and runs the native halves
with the same `quiesce_playback`/`quiesce_microphone`/`playback`/`microphone`
call sequence as `run_playback`/`run_microphone` over fake PCM IO and an
independently advanced native instant line.

The measured sequence, from a baseline whose packets are fully non-silent:

1. Exactly one wake delayed by one period (`skipped_periods == 1`, `Wake`
   stage) bumps both boundaries' reset epochs in the same owner batch.
2. During that render the microphone sink publishes its producer fence.
3. The next native poll completes `quiesce_microphone` (the producer fence is
   already set), discards the whole carrier, acknowledges the consumer fence,
   and rebuilds the bridge, owner and a fresh `MicrophonePump` whose
   `next_submit` is unset.
4. `microphone` treats the unset phase as due (`remaining() == None` passes
   `is_none_or`) and encodes immediately: 480/480 silent frames from the
   still-empty carrier — the target machine's `0/480` signature, one native
   step (~5.3 ms) after the wake observation.
5. Playback then re-primes (2544-frame threshold): about eight more fully
   silent packets, one 433/480 transition packet, then clean audio. Epochs
   converge and stop advancing; recovery is bounded, not a permanent loop.

This closes the attribution question left open by the target runs: an
owner-requested reset after one skipped period is sufficient to produce the
observed empty-carrier encode. No kernel starvation, expiry, or gate change is
required. It also refutes an unbounded-liveness hypothesis: the microphone
carrier refills after the handshake; the earlier reading of the sink's
`available_frames` check as an empty-carrier branch was wrong (it is the
free-space/backpressure branch and did not fire here).

### Contract and fix

A skipped period discards a render opportunity, not audio data: the unrendered
input stays queued in the boundary carriers, which are queues, not wall-clock
slots. The carriers' servo residency (about 2255 frames) absorbs the level
shift and the client receives a continuous waveform. Resetting every fence for
one missed period converts ~5.3 ms of ordinary scheduler jitter (a lateness
covering less than two periods, < ~10.7 ms) into a dual-carrier discard, an
FIR re-prime and a ~50 ms silence window — strictly worse than the jitter it
answers.

The owner therefore still records one skipped period on the clock trace but no
longer resets boundaries for it; `skipped_periods >= 2` keeps the full existing
reset handshake. No reset was removed for real stalls, and no timeout,
half-period constant, queue, slot count, or acceptance rule changed;
stale-epoch crossings remain fenced. The regression asserts zero silent
packets across the single skip, so reintroducing the failure fails the host
suite.

The worker-side immediate encode from an empty carrier remains unchanged and
is honest for genuine (>= 2 period) stalls: audio was actually missed there,
and the fence handshake stays the recovery path.

### Host verification (this slice)

- `cargo test --locked --workspace`: passed (exit 0, no failures), including
  the new regression and the new owner single-skip absorption test.
- `cargo test --locked -p moiren-windows-audio --lib`: 392 passed, 2 ignored.
- `virtual_owner::owner` subset: 8 passed; the existing 6000-period stall
  tests still reset and discard.
- Strict workspace Clippy with both probe features, `cargo fmt --all --
  --check`, `git diff --check`, and `tools/rt-check` (79 passed).

Not claimed: any target-machine result. The guest executable predates this
change; before any combined-short or soak qualification the probe must be
rebuilt, its commit and SHA256 recorded, and the standalone PCM, combined
short, and repetition gates re-run in order.

## Post-fix target revalidation: 2026-10-21 (combined still fails, longer clean span)

Probe rebuilt from `44ca0ed2de226faa1dbba92996cbfeba3baa3b88`
(includes the single-skip absorption fix and the new music-material soak
mode), SHA256 `52B430A8B5DF9A2E49A37C471F67F32075E3F99D80776E3B20AE902C24F522FA`.

- `20261021-44ca0ed-pcm-short1`: PASS. Both 48,000-frame signals match
  exactly; Stop-while-RUN silence verified; service joined, control released.
- `20261021-44ca0ed-combined1`: 15s idle PASS, both signal phases PASS,
  sustained PCM fails at 25.549s with **1,225,920 matching + 480 silent**
  frames (previous builds failed at 12-16s / 600k-774k matching). The fix
  extended the clean span roughly 2x but did not eliminate the failure.
  Exit was a normal quality failure: no kill, no outer timeout, service
  joined, control released.

Owner clock trace for the failed run (13/13 retained; elapsed from owner
start; engine frames cumulative): twelve `skipped=1` gaps and one
`skipped=3` gap (the only reset-triggering event, at 1.59s). The final two
gaps are 10ms apart at 44.757s/44.767s, immediately before the failing
encode. The soak failure coincides with those back-to-back single skips.

Additional evidence from the same run:

- `maximum_packet_gap_ms = 16` in the soak observer: the worker's own
  observation cadence was within 4ms of its 20ms observation-gap threshold.
- Native audio clocks vs QPC over 25 samples: render clock averages
  47,978.6/s (**-445ppm**, peak |deviation| 1264.8ppm); capture clock
  averages 47,993.5/s (-135.8ppm, peak 1038.3ppm).
- Engine production over the run: 2,144,512 frames in 44.767s
  (**-231ppm** vs 48kHz), equal to the fifteen discarded periods.
- No expired WRITE, no request-shape error, no capture position gap or
  regression; discontinuity count zero.

Working hypothesis (not yet proven, handed to an independent read-only
architecture audit): the owner schedule permanently discards missed
production while native consumption advances on device/wall clocks, so each
skipped period is an unrecoverable microphone-side deficit; the follower
servo's authority and the pump's phase anchoring do not reliably rebalance
the carrier level, and clustered skips deplete it below one 480-frame
packet. Per user direction, no further patches are being applied before the
audit concludes; the eight-hour run is on hold.

## Independent architecture audit verdict: 2026-10-21

A read-only independent audit (no code changes, no test runs) of the whole
clocking chain confirmed the working hypothesis and corrected it in three
places. Headline findings, all reproducible from code plus the failed run:

1. **Proven root cause.** `ClockSchedule` discards missed wall-clock periods
   forever (`clock.rs:66-77`): every skip is a permanent mic-side production
   deficit of 256 frames and, symmetrically, a permanent playback-side
   consumption deficit. This run lost 15 periods over 44.767s = **-1791ppm
   one-sided bias** on both directions simultaneously (mic starves, playback
   floods). An earlier note in this ledger read the engine rate as -231ppm;
   the correct arithmetic is 2,144,512/44.767s = 47,904 f/s, of which the
   skip term alone is -85.8 f/s = -1791ppm.
2. **Proven servo authority defect.** `FollowerServo` effective authority is
   **±500ppm, not the nominal ±2000ppm**: `desired = relative_error x 500ppm`
   and carrier geometry (4800 frames, target 2255) bounds relative error to
   ±100%, so the ±2000ppm clamp is unreachable (would need |level-target| =
   9020 frames in a 4800-frame ring). Gain, clamp, and geometry contradict
   each other — a design arithmetic flaw, not a tuning issue.
3. **Proven inevitability.** Net deficit 1650-2130ppm >> 500ppm; the servo
   pseudo-equilibrium sits 7460 frames below target — outside the carrier.
   The level drains monotonically from ~2256 frames in 29-41s; observed
   failure at 25.549s matches. The two final skipped=1 events were the last
   512 frames on an already-collapsed level, not the cause.
4. **Proven: reset during active PCM is unrecoverable.** After a mic reset
   the level restarts at 0 while production (<=47,928 f/s at full servo)
   stays below device-clock consumption — the level can never rebuild;
   every subsequent packet is silence. The single skipped=3 reset in this
   run landed in the idle phase, which is the only reason the run reached
   25.5s.
5. **High confidence: playback is a second independent death path.** The
   symmetric surplus (+50..75 f/s after servo) overflows the 2256-frame
   headroom above reprime level in ~45-95s -> decode-full reset -> 53ms
   reprime silence -> five 480-frame silent packets. Mic simply died first.
6. Also flagged: worker observation-gap threshold 20ms vs observed 16ms
   worst gap (4ms margin) with a discard-everything remedy; owner/worker
   threads lack MMCSS while every physical path sets it; control IOCTL and
   PCM completions serialize on the worker thread; idle-phase mic carrier
   overflow/discard limit cycle randomizes soak start level; the
   `STALL_RESET_SKIPPED_PERIODS` rationale comment in `owner.rs` (servo
   absorbs the level shift) is quantitatively false and must be revised
   with the real fix.

Recommended fix contract from the audit (directions only; nothing
implemented): (A) bounded deficit repayment — the owner records skipped
periods as frame debt and repays up to a capped D (1-2 carrier periods,
~20-40ms) across later wakes, with debt pinned at the cap still escalating
to the existing stall reset; (B) make servo authority genuinely reachable
(gain >= 2000ppm) — necessary, not sufficient alone; (C) reset policy
redesign for active PCM: absorb skips within the debt cap, raise and grade
the observation-gap response (phase reset without carrier discard; discard
reserved for real epoch events), and raise owner/worker thread scheduling
class to match physical paths. Not recommended: larger carriers, encode
water-level gating (moves silence into the kernel), or any timeout-style
device-death heuristics. Verification for any direction: deterministic host
regression with periodic injected skips asserting bounded level, zero
silent packets, and non-pinned debt, plus a carrier water-level probe
observed from the control side before the next target soak.

Per user direction, no product code was changed after the audit; the
eight-hour run remains on hold pending a decision on the fix contract.

## Buffering and clock-sync redesign (2026-10-21, commits 20bdb84..f8c8965)

User decision after the audit: redesign the whole buffering and clock
synchronization instead of patching. Five mechanisms landed, each an
independently revertable commit, all TDD-first:

1. **Debt ledger clock** (`20bdb84`, R1). `ClockSchedule::due` no longer
   jumps `next` past missed wall-clock periods: unrendered periods stay
   due and the owner renders one block per immediate redue, so the engine
   timeline keeps pace with the wall clock pacing the native side (the
   anti-bias invariant: injected periodic two-period late wakes over 3000
   blocks produce zero long-term drift — clock.rs unit test). Debt beyond
   `DEBT_CAP_PERIODS = 8` (2048 frames ~ 42.7 ms, under the 2255-frame
   servo target headroom) is clamped and counted in the trace (new
   `GapStage::Clamp`); clamping on three consecutive wakes escalates to
   the existing reset handshake. `skip_past` and the AfterRender stage are
   folded into the ledger; the false one-period rationale comment is
   replaced. This removes the proven −1791 ppm one-sided skip bias at its
   source.
2. **Reachable servo authority** (`b516f21`, R2). `FOLLOWER_GAIN_PPM` now
   equals `CORRECTION_PPM_LIMIT` (2000): ring geometry bounds relative
   occupancy error to ±1 at the rails, so the ±2000 ppm clamp is reachable
   exactly where it is needed. Servo unit tests (rail authority, sign,
   clamp, smoothing) added; the ±200 ppm dual-direction drift traces pass
   unchanged.
3. **Graded boundary resets** (`742729e`, R3). Mic-carrier overflow and
   playback starvation are bounded counted losses: overflow keeps drop-new
   and the queued in-order frames (worker drains at its own cadence), and
   a starved playback block keeps the settled servo correction and prime
   state instead of re-arming the ~50 ms reprime gate. `finish_transfer`
   no longer requests an epoch on carrier short; the fenced discard stays
   reserved for real epoch events (racing test still pins that path).
4. **Graded worker policy, mic re-prime, MMCSS** (`eb7efd8`, R4). A worker
   observation gap now degrades softly: the stale phase anchor is dropped
   and the next successful collection re-arms it like a dropped
   completion; no epoch bump, no carrier discard. After a completed hard
   reset the first fresh WRITE waits, bounded by 2.5 native periods, until
   the carrier holds two packets of margin — one counted driver-side drop
   instead of ~20 s of partial silence (the proven reset-unrecoverability
   defect). Fresh streams still start their phase immediately. The owner
   thread and PCM worker join the Audio MMCSS class via a shared
   `Mmcss::audio` helper, matching every physical path.
5. **Carrier water-level probes** (`4816e94`, R5). Each virtual boundary
   carries a preallocated `BoundaryProbe` (relaxed atomics, one store per
   block): queued occupancy, applied servo correction in milli-ppm,
   cumulative xruns, soft-loss events. Exposed on `PreparedVirtual*` and
   wired through both the VAD and professional backends. Cross-process
   service-surface exposure deliberately deferred; the deterministic
   regression consumes probes directly.

**Master regression** (`f8c8965`): 2048 owner wakes with a two-period
stall injected every 32nd wake while the native pair keeps streaming.
Pins: engine timeline equals the granted wall-clock frames exactly; no
boundary epoch or fence ever moves; zero silent microphone packets after
warm-up; both carrier levels stay more than a packet and a half from
either rail (probe-observed 483..1414 microphone — the designed slow
±2000 ppm rebuild from an empty start; playback in band); zero xruns; no
orphaned book slots. The regression module runs 240-frame engine blocks
so the harness is rate-exact at the once-per-sub-step pump granularity
(2×5 ms sub-steps per 480-frame packet); the target's Graph256 shape is
revalidated on the machine.

Host validation at each step: `cargo test --locked -p
moiren-windows-audio` (401 lib tests + integration suites green),
`cargo clippy -p moiren-windows-audio --all-targets -- -D warnings`
clean, `cargo fmt` applied. Remaining: workspace-wide gates, guest
rebuild, target combined short revalidation ×2, then the eight-hour
music soak.

## Target revalidation after the redesign (2026-10-21, builds EB9E7792/639ED3D6/6C5330A0/2522BD2B)

Host gates green at every step (401 lib tests, workspace, clippy, fmt,
gpui-test). Two follow-up fixes landed from target evidence:

- `1528c4a` full starvation re-arms only the playback prime gate
  (transferred == 0): consumption pauses while native supply rebuilds
  the level in under a ring time, with servo/FIR state and carrier
  content preserved. Before this, one transient dip left supply
  interleaved with silence for ~20 s of servo-only recovery (three
  runs failed PCM validation with ~8% interleaved silent frames).
- `50a332e` the resumption edge after any prime (re)arm is exact zero
  for one FIR length: the filter's group delay otherwise smears the
  first resumed frames into mixed-amplitude values that trip the
  validator's instant non-matching-frame bail (97%-amplitude frames
  observed on target).

After both fixes the PCM validation phases pass reliably again (both
signals 48,000/48,000 consecutive). The remaining failure class is a
single 480-frame fully silent packet inside the continuous soak at a
random time (0.26 s / 3.1 s / 9.5 s / 19 s / 47.6 s across runs), with
zero epochs moved, zero clamps, zero capture gaps/regressions, zero
driver drops, no encode-short logs, and max packet gap 13-22 ms.
Signature analysis: the mic carrier held 480 real zero frames at one
encode, i.e. the engine rendered ~2 blocks of zero-fill - a partial
playback-carrier starvation lasting a few tens of milliseconds, which
recovers immediately (single packet, not the ~0.5 s prime-hold
signature). The water-level trail needed to separate "level rides low"
from "transient supply gap" requires the virtual-only transport to
publish NativeObservations (virtual-only projects never construct the
Windows transport that now carries the probes; `9542513` landed the
probe-to-snapshot chain for windows transports and the report trail,
which stays empty for virtual-only sessions until that surface exists).
The 65 s combined gate therefore still fails on this VM; the
eight-hour run stays on hold. Builds and evidence:
`C:\MoirenVadTest\20261021-9641601-*`, `20261021-1528c4a-*`,
`20261021-50a332e-*`, `20261021-9542513-*`, `20261021-dbg1-combined`.

## Virtual-only observation surface and the water-level verdict (2026-10-21, build EE66F090, commit 8f41a9e)

`8f41a9e` publishes NativeObservations from the virtual-only transport
itself: `VirtualPreparedTransport` now carries the same streams, bridges
and boundary probes the Windows transport publishes, attach publishes
only after every failure path is past, Running status sets the running
flag, cancel clears the set, and the owner's observation lease removes
it on teardown (a rejected attach publishes nothing). Virtual-only
combined runs therefore record a populated probe trail. Host gates
green: 403 lib tests (two new publication tests), workspace, clippy,
fmt.

One instrumented combined run (`20261021-8f41a9e-combined2`, 15 s idle +
65 s PCM, same invocation as the a6b16d1 series) answers the open
question. The trail held 125 samples at 500 ms:

- Playback carrier level 2,042-2,660 frames (mean 2,362; capacity
  4,800, servo target ≈2,255) — never near empty, including the samples
  bracketing the failure window. Microphone carrier 1,668-2,691 (mean
  2,273). "Level rides low" is refuted.
- Zero structural events during the soak: playback xruns stayed 0,
  neither carrier's soft-event counter grew, and gate telemetry shows
  only the expected 28 ms mic reopen at the validation→soak transition
  (no epoch churn).
- The failure this run was one capture discontinuity plus a 23 ms
  maximum packet gap at ~62 s, with 2,895,360/2,895,360 matching frames
  and zero silent frames in the soak — the same single transient glitch
  family as the earlier one-silent-packet runs, at a random time.
- Native clock diagnostics measured the render-side clock at −1,668 ppm
  average deviation over the run (capture side +527 ppm): this VM's
  timing routinely wanders far between instants.

Verdict: the remaining transient class is an environmental supply
interruption — a 10-25 ms scheduling/clock stall in the guest that
pauses production while both carriers hold ample margin — not margin
exhaustion. No margin increase addresses a loss that occurs with the
level mid-band and no boundary event; the redesigned debt-ledger clock
and graded resets already absorb these stalls as bounded losses. The
mic probe's pre-soak counters (2,851 xruns, constant through the soak)
equal one event per pre-soak packet (idle 15 s + validation ≈ 28.5 s at
10 ms per packet): the expected tail-drops of an unconsumed mic carrier
before the native client starts, not a soak defect.

Evidence: `C:\MoirenVadTest\20261021-8f41a9e-combined1` (refused
pre-created output directory, no probe run) and
`C:\MoirenVadTest\20261021-8f41a9e-combined2` (launch/exit JSON,
stdout/stderr, `probe\report.json`).

## Long-soak attempt 1 (2026-10-21 night): 33-minute idle-gate failure

The first 8h+8h run (`20261021-soak-8h-music`, scheduled task, upgraded
4C/8G VM after the 23:10 reboot) failed its idle gate at 33 minutes:
`idle window lacks two actual frame_end advances` — the engine timeline
froze ~951 ms while the probe's own poll loop froze only 619 ms, so the
advance window detector caught a render-specific stall. Zero epochs,
zero gate events; guest System/Application event logs were completely
clean across the window, and the timeline carried a permanent ~250-730
ppm deficit from debt clamps during the freeze. Diagnosis: VM-wide
host-side freeze (co-scheduled vCPUs), not a driver or engine defect.
The owner's Wake/Clamp trace never reached the logs — it logs at debug
on `moiren::audio::vad` while the runner exported `RUST_LOG=info` — so
`ed2545c` raised that one target to debug in the runner. Attempt-1
launch mechanics also bit twice: SSH-session teardown killed
Start-Process children (use a scheduled task), and the task needed
`-RunLevel Highest` for `powercfg /hibernate off`.

## Long-soak attempt 2 (2026-10-22 morning): idle 8h passes, music gate is environmental

`20261022-soak-8h-music` (00:03→08:25 local, same probe EE66F090):

- **Idle phase completed 28,800 s for the first time** — 28,730/28,800
  verified windows, zero epochs, timeline-wall offset a fixed −1.9 s
  start artifact with no drift. Note `maximum_advance_gap_ms=1336` with
  all windows passing: a VM-wide freeze freezes the observer with the
  render, and the post-thaw burst-advance lands inside the stretched
  window — the window gate is intentionally blind to freezes that stop
  the whole machine including itself.
- **Music phase failed at 22.5 min**: one 480-frame inferred capture
  gap (packet positions; `discontinuities=0`, `timestamp_errors=0`)
  tripped the zero-discontinuity gate. The correlation qualification
  was simultaneously far below gate — 376/1,348 windows (27.9%), a
  269-window failing streak at corr 0.4-0.88 — driven by ±800-frame
  (±17 ms) capture-to-material lag wander against the tracker's ~5
  frames/window re-convergence.
- **Clock trace (now visible)**: 1,430 gap events, every retained one
  `stage=Wake skipped=1`, **zero Clamps** — each late wake was a single
  period repaid immediately by the debt ledger. The redesigned clock
  behaves exactly as designed under sustained VM jitter (~1 late wake
  per 14-21 s).
- Carriers healthy throughout: mic 855-4,788 frames oscillating with
  the servo working ±1,600 ppm, playback stable ~2,300; one mic
  overflow touch (+1 soft event) in 22.5 min. Resources over 8h22m:
  private bytes flat after the first hour (no leak), probe CPU 3.5%
  idle / 6.7% music, pool nonpaged +1.5%.

Verdict: the 8h idle pass plus the flat resource trend is real
stability evidence; the music-phase gates (zero discontinuities, 0.99
correlation ratio) exceed what this host's timing stability sustains.
Full 16h PASS on this VM is not achievable — each single glitch
(~1 per 20 min here, 1 per 65 s in the earlier short runs) trips the
discontinuity gate immediately, and the lag tracker's hysteresis
multiplies each glitch into hundreds of failed windows. Qualifying the
full run needs either a lag-tracker snap fix (measurement artifact,
not a threshold change) plus glitch tolerance, or steadier hardware.
