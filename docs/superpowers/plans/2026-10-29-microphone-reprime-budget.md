# Packet-aware microphone reset reprime implementation plan

**Goal:** Close S2's reset-reprime margin mismatch without native audio or VM
timing work, while keeping the full S1-S3 objective active.

**Architecture:** Prepare a bounded microphone reprime requirement from the
existing admitted virtual Engine/queue budget and the later actual packet
shape. Pass that immutable requirement to the actual provider worker; a
reset waits for the required inventory or its one fixed deadline. Do not
change operating target, ring capacity, steady native phase or test policy.

**Tech Stack:** Rust preparation arithmetic, existing SPSC carrier and fake
clock/PCM tests. No driver/package edits, new dependency or RT allocation.

## Evidence and scope

The current pump waits for two packets with a deadline of 2.5 native periods.
For admitted packet48 / Engine256 at48kHz, that deadline is2.5ms while one
Engine period is5.33ms. A reset immediately after Engine publication can thus
submit before any fresh graph output exists. Existing matrix tests use
prefilled carriers, and the combined worker harness fixes packet480/block240.

Do not use Process inventory endpoints to pretend alternate-target replay is
complete. Its target-dependent servo, censored retirement and unknown packet
availability still require stronger evidence; that separate S3 gap stays open.

## Tasks

- [x] Add a pure `PreparedMicrophoneReprime` and preparation method on
  `PreparedVirtualBudget`, with failing tests before implementation. Required
  inventory covers the explicit admitted target, reachable two-packet margin
  and a complete native Engine burst plus packet/FIR/jitter reserve. Compatible
  capacity limitation is explicit; Target cannot silently cap the requirement.
  Revalidate public budget evidence against preparation. Max wait covers
  nominal refill/SRC startup, one complete admitted
  Engine wake, explicit jitter allowance and legacy 2.5-native-period floor.
  Keep checked arithmetic and actual packet/capacity refusal. This bound is
  not a physical scheduling guarantee or sample-age limit.
- [x] Add fake-clock actual-pump regressions for packet48/block256, first
  publication phases, exact-deadline fallback, unchanged first startup,
  cancellation/epoch fences and no renewed deadline. Observe failures first.
- [x] Pass the actual preparation budget through the provider microphone
  worker and rebuild its packet-aware policy on each native negotiation.
  Preserve the two-way reset handshake and honest fallback silence counters.
- [x] Exercise actual SRC graph publication into the worker after empty reset,
  not just prefilled synthetic inventory. Run affected pure/library/probe/RT/
  app checks, strict lint/check/fmt and independent review.
- [x] Record exact evidence/open gates; atomically commit, push and verify
  remote SHA. Preserve unrelated gate.log and keep hardware gates deferred.
