# Desktop Process capture scope and S1-S3 stopping boundary

Date: 2026-11-02, Asia/Shanghai. Parent source: `c6b2e3d` on
`feat/minimal-io-latency`.

## Small product correction

The Applications source list now explains that capture includes the selected
process and its descendants, not an individual browser tab or audio session.
The explanation is visible for empty and populated catalogs, translated into
English and Simplified Chinese, and exposed as an accessible Label with the
same text value. Existing visual styles are reused; no control or route changes.

This completes the already recorded process-tree explanation requirement in
`docs/plans/2026-10-08-windows-integration-plan.md:174`. It matches the native
include-target-process-tree activation and the preceding
[CLI identity/scope correction](2026-11-02-process-root-identity.md). It does not
add child discovery, exact-emitter selection, session isolation or takeover.
Selection continues to bind the exact catalog PID plus creation time; display
names do not identify a process. Graph edits remain candidates until normal
service confirmation, and selecting an application does not start audio.

## Regression and isolation

Two actual behavioral RED results are retained before implementation:

- The headless Applications test fails because the empty list has no scope
  label, not because of a compilation or test-framework error.
- The locale test fails because the Chinese key falls back to English.

Both pass after the eleven-line view change and translation. The headless
test then publishes two identically named applications with different stable
identities, clicks the intended row, verifies its exact PID/creation binding,
and rejects any request other than the existing candidate source/output edits.
The observed graph remains unchanged until the service processes those edits.
It also checks scope visibility for populated catalogs. All seven locale tests
pass with explicit locale arguments; the test does not mutate global locale.
The final full application library suite with `gpui-test` passes656tests,
0failed,3ignored. Strict application/all-target Clippy with `gpui-test` and
workspace formatting both exit0. The preceding identity increment separately
records workspace2171passed/8ignored; it is not relabeled as a rerun including
these new text/tests. Independent review of the five-file change finds no
substantive issue. These are software-only checks, not native audio evidence.

The shell uses an in-memory ServiceMailbox, disables preference writes through
the existing test harness, and never constructs a native audio driver. No
application window, device, OBS recording or source is opened on the host or
target. A new native soak would not validate this text-only change better.

The independent source export excludes all eighteen preexisting dirty files;
only the five application/view/translation files are copied into the verified
`c6b2e3d` runtime export. Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Logs use `s3-capture-scope-*-1102.log`; final verification and hashes are retained
in `s3-capture-scope-final-1102.json`. The earlier identity binary does not
contain this new desktop text and is not described as a native UI validation.

## Do not expand the original software scope

A bounded source review against the original S1-S3 requirements finds no
additional necessary software integration gap in those requirements after this
small explanation. S1 observations/analyzers and exact IO/Bus collectors, S2
independent operating budgets/recovery accounting, and S3 actual desktop/CLI
configuration/admission/prepared replacement already have linked implementation
and test receipts in the [software ledger](2026-10-29-s1-s3-software-ledger.md).
This is not an overall S1-S3 qualification PASS.

The remaining evidence boundaries are real, but are not instructions to add
ever more offline models or a new render journal:

- Native quality retains delivery-gap-bound violations; zero drop/underrun
  counters do not erase them. Existing VAD revision/epoch errors remain in
  their backlog without restoring the withdrawn fail-fast experiment.
- Actual interpolation coordinates and owner-pinned native PCM do not provide
  sample-exact Engine/render/OBS epoch mapping or calibrated presentation time.
  Unknown timing remains unknown, not an inferred production route defect.
- Matched route A/B waveform quality and calibrated physical end-to-end delay
  remain unqualified. The user has no suitable simultaneous physical recording
  hardware; that limits qualification, not independent application development.

First-packet work stays deferred. Repeated stability scans, smaller-default
tuning, longer histories, cross-boundary render PCM taps and all optional policy
combinations are not silently added to the original completion criteria. Keep
Compatible defaults, quality gates and bounded recovery unchanged. Future
long-running tests should be finite, isolated on the authorized remote target
and parallel to development only when they answer a new useful question; do
not block development or repeat existing soaks merely to occupy the target.

Independent audit handoff:
`20261102-next-step-product-integration-handoff-20261101-234449281-76bc0102.md`
under the evidence root. No push or memory update occurs.
