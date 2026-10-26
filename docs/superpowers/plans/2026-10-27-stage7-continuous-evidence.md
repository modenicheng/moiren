# Stage 7 continuous evidence and remaining acceptance

This is a continuation of `2026-10-26-vad-live-qualification.md`, not a new
success definition. Worktree: `minimal-io-latency`; branch:
`feat/minimal-io-latency`; starting commit: `3872621`.

## Unchanged completion gate

Complete the remaining product acceptance and collect two independent
two-hour controlled-load runs of the same proposed default. Both require
calibrated physical `Input -> Bus -> Output` p95 plus uncertainty at most
10 ms, continuous waveform/marker evidence and zero quality failures. Keep
Compatible until those gates pass. VM results never qualify physical E2E.

## Current evidence and next actions

The previous goal turn made progress: reviewed software repairs, completed
combined host verification, and produced new VM evidence. The current target
smoke has four boundary xruns and two recovery/epoch resets, so it is not a
viable candidate for a two-hour qualification. No calibrated physical setup
exists. These limitations do not prevent the following software work.

1. Add real rendered acceptance for native packet/gap/age/padding, unknown
   values and lifecycle fencing, using explicit expected labels rather than
   regenerating the test oracle from the presentation function. Exercise
   scrolling at compact window size. Retain remaining qualified-evidence and
   service-to-native rename acceptance gaps rather than manufacturing DTO
   success or physical results.
2. Add a bounded native capture evidence tap. Preallocate stereo/mono Float32
   packet storage off RT, use a single producer/consumer with finite capacity,
   never wait or allocate/free on RT, and expose packet/position/sequence loss
   explicitly. Attach it through deferred preparation without changing
   default buffers or whole-run legacy artifacts. The native report retains
   producer allocations until control-side join, including errors/unwind.
3. Integrate a non-RT bounded sink and physical measurement capture with
   explicit byte/frame/duration caps. Preserve exact PCM, timestamp/packet
   metadata, observed continuity and failed artifacts. Native cancellation
   must not wait for a stalled sink; no detached writer. Source markers,
   measurement input, graph/output identities and calibration remain distinct.
4. Add supervised controlled-load collection and source-neutral manifests;
   verify disk and memory bounds, finite cleanup and failure propagation.
5. Repeat short native tests, preserving the target's 64 GB storage policy.
   Fix actual target quality failures before longer candidate tests. Then run
   the same-condition physical sweep, 30-minute candidates and two two-hour
   repetitions only when measurement and quality preconditions exist.

## Verification and delivery

- Use focused allocation/deallocation, FIFO, overflow, invalid-packet,
  continuity, error/unwind and UI acceptance regressions first.
- Independently review each responsibility-sized slice. Commit only its
  paths, push promptly, and preserve concurrent changes.
- Run combined build/check/tests/fmt/strict Clippy and full gpui acceptance
  after integration. Do not use narrow checks as physical evidence.
- Build on the host. Archive every completed/failed guest run with verified
  lengths and SHA-256 before deletion; keep one current probe, no build trees.

## Progress

- [x] Native live rendered UI acceptance slice (`b6fbbd2`, independent review
  PASS). Three 1024x720 headless layout/accessibility tests cover explicit
  labels, missing/stale generations, fresh replacement and vertical scroll.
  Physical-result states and full native/service rename acceptance stay open.
- [x] Bounded capture evidence tap and deferred native attachment (`e16ed25`,
  independent spec/quality review PASS; crate 694 passed/5 ignored, strict
  all-target Clippy and format PASS). This is not a sink or qualification.
- [ ] Bounded sink and actual physical measurement capture integration.
  Standalone native recorder plus bounded disk sink implemented in `ab48a9a`,
  independent spec/quality review PASS; example 52 tests and strict test Clippy
  pass. Host CABLE two-second diagnostic has zero tap loss but one native
  discontinuity (retained failure). Source/graph/load supervision, calibrated
  physical setup and true same-condition capture remain open, so this combined
  integration gate is intentionally unchecked.
- [ ] Supervised load and genuine continuous evidence collection.
- [ ] Stable target candidate and calibrated same-condition physical sweep.
- [ ] Thirty-minute and independent two-hour qualification repetitions.
- [ ] Remaining complete UI state matrix and final branch verification.
  Rendered held-name rename/refusal/fresh-replacement coverage for Playback
  and Microphone is committed in `4b0deba`, with independent spec/quality
  review PASS. It uses injected snapshots, not a real service/native lifecycle.
  Integrated workspace tests pass 1,621/8 ignored across 71 result suites;
  full feature-enabled app tests pass 489/4 ignored across 20 suites. Physical
  import/qualified states and native rename acceptance remain outstanding.

### 2026-10-27 short target matrix and retention

The host-built native probe at source pin `f6f2718` produced all eight timer
case reports with unchanged defaults and zero reported native discontinuities
or microphone write drops. The event run failed in Microphone Mono Pcm16:
asynchronous `GetOverlappedResult(VAD PCM)` returned `0x8007051A` after 36,960
signal frames and two dropped writes. This narrows the current failure to PCM
completion rather than control gate admission; root cause is not established.
Timer observations contain silence too, so eight completed cases are not the
zero-silence physical/stability gate. The wrapper failed to retain process exit
codes (null); do not substitute report presence for a verified exit status.

Six guest files and their manifest (12,577 bytes total including the manifest)
are archived at `target/vad-target-archive/20261027-native-f6f2718-matrix`.
Every payload length/SHA-256 was checked on the host and rechecked on the guest
before deleting the run, temporary native probe and runner. Guest storage now
holds only `e2e_latency_probe-2c61452.exe` (3,359,744 bytes), no dynamic endpoints
or probe processes, and 30.12 GiB free. No driver/system files were removed.
