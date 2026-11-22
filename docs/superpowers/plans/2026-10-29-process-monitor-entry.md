# Process Monitor Application Entry Implementation Plan

> Execute the already-authorized S3 application/CLI propagation requirement
> with `executing-plans` and test-first changes. Preserve the complete S1-S3
> objective; this slice does not replace its remaining acceptance gates.

**Goal:** Restore a real `moiren-app monitor --process` entry that uses the
current professional application service and its validated Process bridge
settings, rather than the retired monitor implementation or a probe command.

**Architecture:** Parse and validate intent without opening native resources.
Build only `Input -> Bus -> Output`, seed an isolated service context, and use
`ManagedDriver` for preparation, confirmation, bounded operation and cleanup.
Keep command parsing, graph/context construction and supervised execution in
separate modules. Reuse existing platform budget admission; never infer Process
delivery timing from the physical output period.

**Constraints:**
- Work only in the existing `feat/minimal-io-latency` worktree.
- Leave compatibility defaults and persistent desktop settings untouched.
- Explicit Process queue options require a complete target/capacity/jitter/
  packet/gap tuple. CLI bounds are declared assumptions, not measured evidence.
- An explicit sample-age request remains visible and is refused by the native
  Process preflight until measurable sample age is implemented.
- Stable PID plus creation time is resolved before service startup and checked
  by the existing native owner. No silent source substitution or default output.
- Use a finite requested run, startup deadline and explicit service exit/join.
  A control deadline cannot interrupt a blocked OS call or prove native teardown.
- Report requested versus confirmed active settings and operational outcome;
  never label monitor completion a latency/stability qualification.
- No audio-device exercise, VM deployment, driver installation or host kernel
  change in this software-only slice.

## Task 1: Parse the actual application command

- [x] Add regression tests for GUI/offline compatibility, monitor help and a full
  Process queue tuple; reject missing/duplicate/unknown/out-of-range options.
- [x] Implement a shared CLI parser used by `main`, with compatibility defaults
  and explicit requested native-output settings separate from bridge settings.
- [x] Verify native Process age refusal before opening the service.

## Task 2: Prepare and supervise the real application route

- [x] Add graph/context tests proving exactly Source/Bus/Sink, stable
  Process identity, pinned output, both precisions and effective settings.
- [x] Seed a service context before any polling/activation, using a narrow
  crate-local spawn API; avoid a race between separate settings requests.
- [x] Use the same ManagedDriver as desktop; record activation only after its
  matching generation/revision and active-settings confirmation.
- [x] Test successful completion, preparation refusal, owner failure, start
  deadline and cleanup, using the existing non-native fake backend.
- [x] Keep errors through exit, request cleanup on every path and join the
  service; do not invent final native counters from a last-running snapshot.

## Task 3: Verify and deliver

- [x] Run focused CLI/application tests, full relevant application library tests,
  all-target app check, strict Clippy and workspace formatting.
- [x] Review the diff against original S3 requirements; update the execution
  ledger with current evidence and remaining measured-delivery/physical gates.
- [x] Commit explicit code/test paths atomically, push promptly, and verify
  the remote hash. Commit documentation separately; preserve `gate.log`.

Implemented as `a4a21d6`. Command help initially failed in the real application
binary, then passed after entry integration. An injected native stop error
initially produced false cleanup success, then passed after retaining the
post-join Host snapshot. See the
[execution receipt](../../validation/2026-10-29-process-monitor-cli.md) for
the complete check results, earlier failed attempts and remaining S3 gaps.

The earlier turn made progress by committing and remotely verifying the test
policy implementation and its documentation. Physical acceptance stays
deferred by the user's 2026-10-29 instruction, not a reason to stop this work.
