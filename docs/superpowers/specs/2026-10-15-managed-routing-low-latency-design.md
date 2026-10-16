# Managed desktop routing through low latency

The user approved implementing the backend sequence through low latency on
2026-10-15. This work adopts the existing EngineHost, enables multiple native
inputs/outputs, and exposes validated latency profiles. Existing Exclusive
experiment changes are outside this delivery and must be preserved.

## Architecture

EngineHost remains the only compiler/runtime lifecycle owner. A managed service
driver translates the existing application candidate graph into EngineProject,
submits edits, projects confirmed revisions, and routes node/send parameters.
Preparation, shutdown, subscription cleanup and host destruction run off the UI
and audio threads. Keep the legacy backend driver for its public integrations and
focused tests, but the real Windows desktop starts the managed driver.

Native routing uses one explicit master render clock. Further physical render
endpoints are followers with bounded preallocated queues and continuous clock
adaptation; they never independently execute the main Engine. Failure stops the
linked native owners, all of which must acknowledge Start before Running.
Every active sink consumer must remain protected through compatible plan swaps.
Reject duplicate endpoint ownership and invalid master assignments explicitly.
Changing the IO topology may replace the transport and advance its epoch;
compatible DSP/edge edits must retain the owner and timeline.

Each graph Source and Sink has its own saved resource binding. The existing
session input/output selection remains a convenient initial/default binding,
not an alternative source of routing truth. Multiple IO projects must run rather
than merely serialize. Candidate/active identity remains distinct on errors.

## Low latency

Provide Compatible, Balanced and Low latency choices with explicit next-apply
semantics and persistence. Derive native periods and queue admission from actual
device timing using the existing prepare_latency_budget contracts. Unsupported
periods and impossible input/render budgets produce visible diagnostics, never
silent clamping or fabricated latency values. Preserve Compatible as the safe
default; smaller queues are configurations rather than hardware certification.

Expose immutable native preparation observations (rate, actual period, queue
targets/capacity and initialization fallback) and bounded live queue/clock
statistics from the managed backend. Service snapshots tag observations with
their owner identity so stopped/stale sessions cannot appear active. Actual
hardware physical input-to-output latency remains unknown until physically
measured; CPU and XRUN are not inferred from queue fill or silence.

## Acceptance

- Compatible graph edits retain owner/epoch, preserve state and only become
  active after EngineHost/RT confirmation; invalid candidates retain playback.
- Multiple inputs mix through the real graph; multiple outputs consume their
  own routing with bounded follower queues and drift adaptation.
- Parameters including sends target stable NodeId/EdgeId and cannot cross loaded
  project identity boundaries.
- Rejected latency requests retain explicit reasons. Persisted old preferences
  remain readable with Compatible as their default.
- Run portable regression tests and RT allocation tests, workspace check/test,
  gpui headless acceptance, fmt and strict Clippy. Keep native hardware tests
  opt-in, muted where possible, and record actual executions separately.

The sequence does not include Takeover, Exclusive production output, ASIO,
plugins, virtual drivers, new DSP algorithms or professional SRC quality.
