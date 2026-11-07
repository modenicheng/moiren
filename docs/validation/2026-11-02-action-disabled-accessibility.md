# Disabled action accessibility metadata

Date: 2026-11-02, Asia/Shanghai. Source baseline: clean `f533869` plus
the scoped widget changes described here. No audio policy, buffer, default,
driver, shutdown or detector behavior changes.

## Defect and scope

The retained Process low-target desktop observation shows the visually disabled
`Apply measured budget and restart` action reporting UIA `IsEnabled=true`,
although it has no InvokePattern and clicking it is inert. This is a metadata
defect, not evidence that the rejected measured request can activate.

The locked `gpui-base 0.7.3` Button suppresses click/focus when disabled but
does not set AccessKit's disabled property. Its own upstream accessibility test
documents this limitation. `gpui-pre 0.3.9` has no public disabled ARIA setter
on the rendered Div. Registry sources and dependency pins remain untouched.

An app-local `ActionAccessibility<E>` delegates the existing native element's
ID, source location, layout/prepaint/paint state, role, metadata and synthetic
children, then conditionally sets disabled metadata. `ActionButton` and
`IconButton` materialize their styled base button once before decorating it.
There is no added Div, layout node, accessibility parent or nested ID. The base
button still owns interaction/focus, including its existing disabled behavior.
Toggle widgets and unrelated controls are outside this fix.

## Regression evidence

Evidence directory:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s3-disabled-action-20261102-1310`.

- `red-disabled-metadata.log`: genuine regression RED. The real materialized
  button node has its ID, label and enabled/disabled action surface, but
  `is_disabled()` is false where true is required. This is a runtime assertion,
  not a compiler or harness failure.
- `green-disabled-metadata.log`: the exact regression passes after decoration.
- Earlier `red.log`, `red-corrected-harness.log` and `red-native-node.log`
  preserve, respectively, a duplicate-render probe issue, inspection of an
  opaque component instead of its native element, and ambiguous render trait
  selection. They are not accepted target RED evidence.
- Final direct-node coverage checks actual role, ID, exact labels, Click/Focus
  action availability and disabled state for enabled/disabled/re-enabled
  instances with the same IDs. Real headless events separately cover inert
  disabled Action/Icon buttons, pointer/Enter/Space activation, unchanged
  bounds and focus behavior through repeated state changes.
- `app-ui-library.log`: 661 passed, 3 ignored, including measured settings and
  the house action family. No device or desktop window is opened.
- `workspace-tests.log`: `cargo test --locked --workspace --features
  moiren-app/gpui-test -j1` exits 0: 2307 passed, 9 ignored across 71 result
  surfaces, including all 13 UI acceptance tests. This feature-enabled count
  overlaps the separate application-library invocation; do not add them as
  unique behaviors. Hardware and native-window opt-in tests remain ignored.
- `workspace-clippy.log`: `cargo clippy --locked --workspace --all-targets
  --features moiren-app/gpui-test -j1 -- -D warnings` exits 0.
  `cargo fmt --all -- --check` also exits 0 (`fmt.log`). Linux, CI and Miri
  are not rerun by these Windows checks.
- `clean-source-verification.json`: all tracked export files match `f533869`
  except the three intended widget files; the sole new source is the adapter.
  Preserved PCM/SRC experiments are not part of this test source.

The upstream test-support `Observed` wrapper reads its own inner metadata, so
its disabled snapshot does not include this outer decoration. Those snapshots
are not used as a substitute for actual decorated AccessKit nodes or Windows
UIA. No test-only metadata is fabricated to hide that distinction.

## Acceptance boundary

Independent static review finds no concrete implementation defect. Actual
Windows UIA `IsEnabled` on this new build remains **unverified**; current
interactive desktop startup acceptance is still open. This node-level repair
does not turn that harness failure into a native desktop PASS, resolve the
retained Process/VAD waveform refusals, or qualify physical latency. First-packet
investigation stays deferred, and S1-S3 qualification remains incomplete.
