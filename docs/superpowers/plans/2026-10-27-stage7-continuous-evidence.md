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

- [ ] Native live rendered UI acceptance slice.
- [ ] Bounded capture evidence tap and deferred native attachment.
- [ ] Bounded sink and actual physical measurement capture integration.
- [ ] Supervised load and genuine continuous evidence collection.
- [ ] Stable target candidate and calibrated same-condition physical sweep.
- [ ] Thirty-minute and independent two-hour qualification repetitions.
- [ ] Remaining complete UI state matrix and final branch verification.
