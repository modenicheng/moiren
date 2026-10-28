# Process Packet Publication Implementation Plan

**Goal:** Supply the actual per-packet publication envelopes missing from S1/S3
without pretending completed drains reveal packet availability or sample age.

**Architecture:** A prepared unique-writer scalar history records offered and
accepted native frames, cumulative frontiers and host-QPC brackets around the
actual SPSC publication. Capture ingress owns it; native preparation supplies
the existing prepared QPC. Ordinary untimed calls and graph-output writes stay
explicitly unknown and preserve provenance. Control observations retain the
same generation-pinned history after registry replacement/retirement.

**Tech Stack:** Rust, existing fixed atomic history pattern, rtrb SPSC and
software COM capture mocks. No new dependencies or native audio exercise.

## Constraints

- No VM, native audio device, driver build/install or deployment in this slice.
- No per-sample clock, allocation/deallocation, logging, lock or retry on RT.
- Offered supply is not admitted publication; partial/zero acceptance stays
  visible. ReleaseBuffer failure does not erase audio already published.
- Packet host-QPC publication brackets are not WASAPI sample timestamps.
- Keep current queue intent/defaults/admission; alternate-target replay still
  requires consumer envelopes and target-independent demand bounds.
- Preserve unrelated gate.log, commit explicit paths and push promptly.

## Tasks

- [x] Add pure `latency::packets` history with test-first conservation, retained
  suffix/frontiers, unknown/regressed clocks, saturation and concurrent snapshots.
  Writer records bounded scalar data; observer makes one nonblocking attempt.
- [x] Add failing bridge integration tests for exact before/after commit clocks,
  partial/zero acceptance, invalid/empty calls, provenance and no RT allocation.
  Wire actual ingress publication and include its storage in preparation budget.
- [x] Add native mock regressions proving clocked per-packet events survive
  successful release, stop/query failure and ReleaseBuffer failure. Preserve
  release failure as hard failure rather than silently healing the session.
- [x] Project packet history through actual bridge/Process observation surfaces;
  test retained identity/generation and late publication after registry clear.
  Update explicit app fixtures without introducing fake measured evidence.
- [x] Run focused/full audio, app, probe and RT checks plus strict lint/check/fmt.
  Independently review publication/provenance semantics, record remaining gates,
  atomically commit/push and verify remote SHA.

Implementation: `6b7ec12`, `680ff8e`, consumer continuation `ebc1fbd`, closure
`c544fb1` and bounded replay `1531eff`, each pushed and remote-hash verified.
Exact results, failed attempts and remaining scope:
[`../../validation/2026-10-29-process-packet-publication.md`](../../validation/2026-10-29-process-packet-publication.md).
