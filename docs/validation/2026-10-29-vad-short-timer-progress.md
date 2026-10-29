# VAD Short Timer Packet Progress

Date: 2026-10-29. Worktree: `D:\coding\moiren\.worktrees\minimal-io-latency`.
Branch: `feat/minimal-io-latency`. Baseline:
`2c585d5b87060916f5c9f569a89c457faf2298a7`.
Fix: `9cba24baf5cf98f4a5a5f560cffc98cdcb9e4e29` (local only).

## Reproduced Defect And Actual Policy

The production timer planner previously returned a zero-frame rebase whenever
pending elapsed frames exceeded one logical packet. Valid48/49-frame packets
with repeated1.1ms callbacks observe52/53 frames on a10MHz QPC, so every
callback could rebase, quiesce and make no progress. The existing exact1ms
fixture and explicit short-packet-jitter TODO did not close this behavior.

For packets<96 frames, `TimerDrivenPendingLimit` now admits up to one packet
plus48 native frames, capped by actual mapped frames. Packets>=96 retain their
old pending threshold and scheduling. Every callback still processes at most
one logical packet, at most three logical/mapped copy spans and at most one
PCM transfer. Original clock/PCM association is preserved under the admitted
limit; residual debt requests `Due=-1`. Once drained, the next quantum remains
on the original clock, not a drifting `now+1ms` schedule.

Above the threshold the callback still transfers zero, rebases, quiesces old
PCM and records the discontinuity. The first timer-stall diagnostic receives
the exact ring-admitted threshold; it does not infer a new deadline from the
packet size alone. This threshold uses quantized elapsed frames, not an exact
wall-clock timeout or a general hardware retention guarantee.

No request/completion batch expansion, queue/default change, packet-size
relabeling, microphone freshness relaxation or RT allocation/blocking is added.
A proposed two-packet callback was rejected: current microphone pumping has
only one pending WRITE, and the completion batch assumes one transfer.
Graph whole-native-drain/wake admission and provider/reprime behavior remain.

## Verification

All runs occur on the development host. No driver installation, native audio,
VM access or deployment occurs. Logs are in the ignored worktree directory
`target/s1-s3-20261028/`; generated packages stay in `driver/vad/build/`.

| Command | Result | Log |
| --- | --- | --- |
| `./driver/vad/test.ps1 -Test all` | all10 MSVC host suites passed with `/W4 /WX` | `short-timer-host-final.log` |
| `./driver/vad/build.ps1 -Analyze` at fix commit | WDK x64 Debug analyzed build and catalog generation passed; no compiler/analyzer warning/error found | `short-timer-wdk-final.log`, `driver/vad/build/build.log` |
| Native `InfVerif.exe /w /v` (inside build script) | INF is VALID | `driver/vad/build/infverif.log` |
| `git diff --check` | passed | direct output |

Host suites: lifecycle, audio, ownership, timer, cleanup, removal, diagnostics,
clock_diagnostics, unserved_diagnostics and interface_name. These are10 suites,
not10 total assertions. The audio suite includes its existing3.6M rational
deadline checks. Rust/app tests, remote CI and Miri are not rerun for this
driver-only source slice.

### Regression And Review Evidence

- New short-packet test first fails on the production planner's zero-progress
  behavior (`short-timer-red.log`, `short-timer-policy-red.log`). Design review
  changes the proposed two-packet test contract before implementation, retaining
  the original RED logs rather than claiming them as final green evidence.
- Final patterned PCM tests run3000 successive1.1ms primary wakes for both
  playback and microphone with48/49-frame packets. Each fixture explicitly
  services residual debt100 QPC ticks later, asserts exact elapsed-frame
  conservation, unchanged epoch, one-transfer/three-span bounds and every
  patterned byte across logical/native wraps. This timely-service fixture is
  not evidence Windows or the provider can replenish WRITEs that quickly.
- Separate3000-callback fixtures give every callback a fixed1.1ms interval,
  including redue requests. They require real progress plus counted overload
  resets, not lossless output or zero starvation under impossible throughput.
- Inclusive threshold and one-frame-over tests cover48/49/95/96/480/512/1250/
  4800 packets with2/4/8-byte alignments. A narrow page-mapping fixture verifies
  the pending limit cannot exceed mapped history. Larger packets retain their
  old threshold. Existing long-stall, epoch/stream/gate fencing and fresh-packet
  recovery tests remain green.
- Production-extracted `VadAudioTimer` tests cover short progress, residual
  service, exact first-stall deadline, one-time invalidation, no event-mode
  packet notification and same-QPC retry. Restoring the old stall predicate as
  a temporary mutation genuinely fails the new actual-callback assertion;
  restoring the fix passes (`short-callback-mutation-red.log`,
  `short-callback-restored-green.log`). The extracted test stubs transfer and
  quiescence; patterned PCM tests separately exercise the real copy helper.
- Two independent read-only source reviews find no Important/Critical issue in
  the final policy, arithmetic, mapped/frame bounds, clock and diagnostics.
  Reviewers do not build/run tests or access the VM.

### Generated Artifact Receipt

Final analyzed host build at `9cba24b` produces an unsigned development package;
catalog generation is not release signing or installation qualification.
`DriverVer=10/29/2026,9.55.16.120`.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| `driver/vad/build/bin/x64/Debug/MoirenVad.sys` |120320| `9B15FF7EAF08875B5565D361959325263B9C8326384C781A0FC2C311D3FF8D12` |
| `driver/vad/build/bin/x64/Debug/MoirenVad/MoirenVad.inf` |1804| `333D5D0833EB5AB1040B05F51BD07EC7B700A8B56E50D8D6F8BAC7E7146E7D36` |
| `driver/vad/build/bin/x64/Debug/MoirenVad/moirenvad.cat` |1178| `A854B1B0BA824144817CFD6889C16088B49E1811DE2485159AC9870FB7E5B490` |

Build timestamps and hashes differ from the earlier pre-commit run. Only this
final receipt identifies the final committed-source build. Nothing is copied
to the guest or loaded on the host.

## Remaining Gates

This closes a source-level S2 short-timer starvation defect, not S1-S3 as a
whole. A minimal due time is a request, not guaranteed scheduling. Microphone
WRITE supply/freshness failures remain honestly counted. Mapped capacity is an
upper safety ceiling, not proof that Windows retained a specific PCM interval.

Actual lower-period negotiation and stable waveform playback on suitable
hardware, source-age provenance, sustained service and matched physical IO/Bus
below10ms remain unqualified. The actual e2e VAD probe still uses the default
period; adding opt-in queried-minimum period/evidence there is another software
integration gap, separate from standalone `vad_native --minimum-period`.

Correctness0..3000ms progress grace versus latency-sensitive fail-fast remains
unchanged. Grace is not audio latency and does not enlarge production buffers.
This work stays in scoped local commits; no push or new CI trigger occurs.
Unrelated `gate.log` is preserved.
