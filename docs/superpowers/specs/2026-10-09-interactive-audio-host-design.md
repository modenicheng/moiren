# M0.6 Interactive Multi-Source Audio Host

The six acceptance requirements in the supplied milestone remain the scope:

1. A single WASAPI render session runs until stopped, with live Gain/Pan control.
2. The real render owner applies prepared plans at block boundaries; control reclaims retired resources. The output bridge and retained input backends keep their identities.
3. Physical capture and process loopback run simultaneously as graph sources.
4. Each source has Gain/Pan into a Bus; starting, stopping, failing, removing and replacing one source does not stop render or another capture.
5. Cold start, drift, stop and reconfiguration have regression tests and reproducible physical stress evidence. Investigate the existing Miri failure rather than dropping its coverage.
6. UI-independent domain APIs expose graph edits/snapshots, runtime state, accepted/applied parameter replies, desired/active plan revisions, devices/process identity and diagnostics.

Additional explicitly requested investigation: a controlled Takeover feasibility experiment covering original playback, mute/volume capture effects, parallel observers, restart and verified restoration. Only alter audio sessions owned by the experiment unless separately authorized. Report evidence and limits without equating loopback capture with redirection.

## Ownership

`AudioHost` owns the editable desired graph, stable source IDs, parameter control ports, plan snapshots/bindings and revision history on the control thread. A separately owned engine is prepared once and moved into `DemandRenderer`. WASAPI render owns COM objects for its entire lifetime. The render engine is returned to control after stopping, including on backend failure.

Capture owners each use their own stop event. Host polling joins a completed capture and records its status while the clock bridge renders a initialized silent tail. Render failure terminates the host; capture failure terminates only that source. Capture startup and all graph compilation happen on control, outside audio processing.

The default graph is `source -> gain -> pan -> bus -> optional compressor -> sink`. UI consumes app-domain handles and plain data; Engine, WASAPI interfaces and ExecutionPlan never appear in that contract. Provide explicit graph commands and structured validation errors, not an IPC protocol.

## Plan publication

Only one candidate may be pending. A candidate compiles with inert placeholders for retained input backends and a fresh placeholder sink. Explicit reuse maps stable logical identities to each plan's local processor IDs, preserving the original sink, sources, unchanged DSP processors and unchanged sends. Bus reuse is excluded whenever its prepared IO schema changes. Changed edge settings must take effect rather than inheriting stale runtime settings. Compile/publish failure leaves the current graph and output working; ownership consumed while preparing a newly added capture must be released safely on control.

Desired revision records the edited state; active revision advances only on render acknowledgement. Retired plans are polled and destroyed on control after all queued parameter requests have received terminal replies. Retain the old parameter port until those replies are drained. Accepted requests never imply applied requests. Runtime timeline telemetry is atomic and bounded.

## Verification

Run focused deterministic tests for source mixing, independent stop/failure, parameter receipts, repeated compressor insertion/removal, failed edits, queue pressure, bridge identity and no RT allocation/deallocation. Then run locked workspace tests/checks, strict Clippy, formatting and diff checks. Commit privacy-safe statistics and reproduction commands for hardware tests, with unexecuted physical scenarios explicitly recorded as unverified. No PCM data, full process paths or persistent device identifiers in shared evidence.

Preserve the user's frontend work at `crates/moiren-app/src/ui/` in the original checkout. Development takes place in `.worktrees/audio-host` on `feat/interactive-audio-host`.
