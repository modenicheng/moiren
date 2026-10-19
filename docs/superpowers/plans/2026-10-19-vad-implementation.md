# VAD implementation ledger

## Scope and baseline

Implement the approved VAD design in ordered, independently verified stages.
Keep Graph, DSP, SRC, routing, persistence and recovery policy in user space.
The driver is an optional, separately built ACX 1.1/KMDF component; normal Cargo
builds must not require WDK or install a driver.

The initial behavioral baseline was `c97730d`. The user explicitly approved
preserving the concurrent refactors through `819e8de` and continuing on
`feat/vad-foundation`. Preserve the existing `driver/moiren/` experiment and the
unrelated repository-remediation draft.

## P0: native owner semantics

Implemented:

- Native followers start after preparation and activation, without waiting for
  master content. Their existing pump emits silence and never renders Graph.
- Remove the three-second no-transfer watchdog. Zero reads/writes, empty/full
  queues, silence and wait timeouts do not authorize resource retirement.
- Keep explicit native failures, malformed transfer lengths, cancellation,
  panic containment and stop/release/drop recovery authorization.
- Keep the follower queue target for drift correction and capture content
  priming; it is not a condition for starting a native stream.

Regression coverage uses fake native streams and bounded iteration counts.
One dedicated test crosses the former wall-clock deadline for master and
follower concurrently; no owner clock remains to replace with a fake clock.
Restoring the old watchdog in an isolated source copy makes that test fail.
This is a regression check, not an eight-hour hardware qualification.

Verification:

- `cargo check --locked --workspace --all-targets`: passed.
- `cargo test --locked --workspace`: passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- Native-owner RT harness: 75 passed, including seven new idle/transition tests;
  focused code review approved the change.
- Miri's Linux emulation target passes the idle-to-PCM ownership transition
  using Stacked Borrows. The Windows target reports an allocator-header borrow
  violation also reproduced by the existing standalone `System` allocator
  example, without any Moiren dependency.
- Separate harness formatting has a pre-existing issue in
  `professional/asio_rt_tests.rs`; do not mix that cleanup into P0.

## Remaining stages and admission gates

| Stage | Deliverable | Gate |
| --- | --- | --- |
| P1 | Independent ACX root, bounded dynamic raw PDOs, control-session cleanup, endpoint policy before exposure | Real audio endpoints, standard-user control, default qualification, safe deletion/recreation while clients hold streams |
| P2 | Four-slot overlapped Direct I/O per direction, cancellation and generation-fenced PCM | PCM16/Float32, mono/stereo, bounds and stale-completion tests |
| P3 | Explicit virtual resources, channel-aware boundaries, one Engine owner and virtual clock | Virtual-only and mixed physical/virtual operation, no stale catch-up |
| P4 | Project/preferences v2 migration, export management and activation gates | Local default qualification isolation, activation/switch failure behavior |
| P5 | Target-machine stress, Verifier, power, installation and release qualification | Eight-hour runs, 100 lifecycle rounds, bounded resources, rollback, signing |

The P1 control foundation has no audio admission until actual ACX circuit and
stream capabilities exist. Runtime PDO tokens are not persistent
`VirtualEndpointId` values. Do not bind these tokens into projects or expose a
global `AudioBackend::Vad`.

Later stages must not bypass P1's active-stream removal and pre-exposure policy
gates. Creating a raw PDO or building an unsigned `.sys`/`.cat` does not satisfy
those gates. Test signing, target installation and formal release qualification
are separate from source/build validation.
