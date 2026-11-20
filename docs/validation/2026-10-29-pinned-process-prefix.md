# Managed-session pinned Process prefix evidence

2026-10-29 (Asia/Shanghai). Host-only S1/S3 continuation. No native audio,
VM/deployment, driver build/install or production buffer/default change.

## Atomic receipts

Each implementation commit was pushed immediately and its full SHA compared
with `git ls-remote origin refs/heads/feat/minimal-io-latency`:

| Commit | Responsibility |
| --- | --- |
| `c02a51b309a5709adbc82923c3836e1d24049049` | Opaque pinned-source native prefix collector, cheap refusal/identity accessors, actual bridge regressions and RT shadow compatibility |
| `a4e37764bf3a5336d5721669d9d85d7febd4133f` | Managed-session polling, post-join evidence, settled origin fencing, read-only evidence and desktop diagnostics |
| `975f3b3fb4fdbd30aebe43fb8b779c7ee8224d65` | Unwind-safe refusal fixture and deterministic live no-archive-clone counter for mutation verification |

Unrelated untracked `gate.log` is untouched. Plan:
`docs/superpowers/plans/2026-10-29-pinned-process-prefix.md`.

## Collection and provenance

`NativeObservationReadHandle::process_trace_collector` checks exact node,
Process PID/creation-time and native generation against its retained private
source. It copies immutable rates/whole-wake demand from that source and polls
immediately, without a full public snapshot read first. Later registry clear,
replacement or native recovery cannot redirect its observers. The collector
retains observation storage only, not audio resources or unique writers.

The control-side archive still caps each writer at 8192 events; native history
rings remain 256 events. Polling preserves sequence one across overlapping
suffixes. Missed/conflicting/capped prefixes permanently refuse replay but
continue collecting final counters/closure; no audio cancellation, buffer
enlargement or invented later empty starting state occurs. Cheap live refusal
access avoids cloning the archive to discover a failure.

Only the opaque collector can return a RetainedNativeOwner trace. Public
artifacts/imported JSON remain CallerSupplied, even if originally exported
from this collector. Writer closure is required for structural replay, but is
not itself clean teardown, waveform continuity, source age or latency proof.

## Actual managed lifecycle

The actual driver automatically starts at most one original-source collector
per managed host when exactly one Process source is settled. Multiple Process
sources explicitly refuse this bounded collector rather than silently choosing
one or allocating unbounded archives; audio continues normally.

Initial admission checks pending Apply/debounce/edits/parameters, current and
confirmed settings, application generation/document, candidate/active/pending
graph revisions, host desired/active revisions and the executing host source
selector. Native construction uses the running backend-owned pinned registry,
not editable AppSnapshot DTO labels. A delayed initial poll can still miss
sequence one; this is visible refusal, not a claim of guaranteed collection.

During collection the original app/native identity, active graph/settings and
document are fenced. Changes to requested settings permanently refuse the
collection without stopping audio. Candidate-only edits do not become active
evidence: the original active-source origin remains explicit until execution
changes, which refuses the collection. A new native owner is never spliced
into the original archive.

Retirement takes the collector before clearing the live registry. On the
cleanup worker, host Drop joins preparation/native/cleanup workers; only then
does final polling read late writes and writer closure. Final native generation
and host desired revision catch recovery/edit changes after the last live poll.
Closed host, absent cleanup error and absent fault are independently required.
Failure retains the partial archive with a refusal and no usable trace.

One Arc-backed predecessor evidence object is retained with `last_cleanup`.
Ordinary UI snapshots share it instead of cloning full event arrays; replacing
the predecessor releases service ownership of the old evidence. Public getters
are read-only and cannot replace the privately retained validated trace.
Live diagnostics expose irreversible refusal immediately; stopped diagnostics
show original session/document/graph/native/Process identity, retained counts
and either refusal or explicitly unqualified closed-prefix availability.

## Fresh verification

| Command | Final result |
| --- | --- |
| `cargo test --locked -p moiren-windows-audio --lib` | 547 passed, 2 ignored; includes four pinned collector tests |
| `cargo test --locked -p moiren-app --lib --features gpui-test` | 484 passed, 3 ignored; includes eight managed-prefix regressions |
| App CLI and transport-settings integrations | 2 and 6 passed |
| `cargo test --locked -p moiren-app --test ui_acceptance --features gpui-test` | Final 13 passed |
| `cargo test --locked -p moiren-windows-audio --example e2e_latency_probe --features vad-probe` | Final 298 passed |
| `cargo test --locked --manifest-path tools/rt-check/Cargo.toml` | Final 146 passed; includes shadow collector tests, not 146 separate allocation tests |
| Strict all-target audio Clippy with `vad-probe`, app Clippy with `gpui-test`, RT harness Clippy | Exit 0 |
| RT all-target Linux cross-check | Exit 0, warning-free; not Linux execution |
| Root/scoped RT fmt and diff checks | Exit 0 |

Actual bridge regressions cover 601 events across registry replacement and
late writes, live-writer refusal, retained-native versus imported provenance,
identity/node/generation mismatch, missed polls, 8192 cap and immediate
constructor polling before a second 200-event burst overwrites the prefix.

Managed lifecycle tests run the real software host and Drop/join path with a
private software bridge collection seam. They verify late final writes,
cleanup failure, Arc snapshot reuse, retained stopped diagnostics, restart,
origin changes, late recovery/revision fencing and no archive clone for live
refusal. The same production admission helper is exercised with a real
started Process-configured software host: pending/settings/document/revision
faults, active source mismatch and missing/multiple Process sources. These
fixtures remain CallerSupplied; they do not claim an actual native end-to-end
run or fake a public native registry to obtain trusted provenance.

## Failed attempts and review

- Native and app tests first fail on missing APIs/modules. The first native
  test imports a nonexistent source type; corrected imports isolate the missing
  API RED before implementation. This is compile RED, not a behavior claim.
- A genuine late-final recovery regression fails before the final native
  generation/revision fence, then passes after the fix.
- Review finds incomplete settled/settings admission and hidden live prefix
  refusal. The shared production helper, private native metadata access and
  cheap sticky refusal propagation close both. The additional admission tests
  initially fail on missing helper/trait API, not on a claimed native run.
- Deliberately removing construction/live refusal propagation initially aborts
  the test process with STATUS_STACK_BUFFER_OVERRUN instead of an ordinary
  assertion report: the panic-on-artifact fixture is unsafe during failure
  cleanup. A `finally` restores original production bytes, then eight focused
  tests and strict app Clippy pass. This abort is not accepted as a clean RED.
  Replacing the fixture panic with an archive-access counter and benign final
  refusal makes the same mutation fail normally at missing live refusal.
  Restoring production bytes again yields eight passes and strict Clippy exit0.
  The final counter still proves zero full-archive reads on the live path.
- Test construction visibility, nonexistent GraphNodeState.kind and an unused
  evidence alias are corrected; final strict checks are clean.
- Initial RT shadow compilation warns on a new control-only reexport. A scoped
  shadow-module allow matches existing dead-code policy; final RT tests,
  strict Clippy and Linux cross-check are warning-free. Production warnings
  are not suppressed.
- Two independent final read-only reviews find no remaining Important/Critical
  issue in their scope. Reviewers inspect source/test definitions, not rerun
  cargo or native verification. No remote CI or Miri run is claimed.

Logs: `target/s1-s3-20261028/s3-pinned-*`. Failed/corrected attempts remain;
final named logs, not an earlier partial run, support the counts above.

## Remaining gates

This closes managed-session prefix loss at retirement, not the whole desktop
finite-window evaluate/select/apply workflow. The desktop can inspect retained
diagnostics; it does not yet run an explicit candidate/window policy against
that evidence and commit an evaluated replacement. Whole-wake admission stays
independent of segmented replay. The bounded prefix is not sustained future
service or automatically promotable production admission.

Source-age provenance, actual lower native periods, matched physical route
measurements and stable sub-10-ms output remain open/deferred. VM timing and
music stability are not retried. Correctness keeps recording timing/quality
faults with configurable 0..3000 ms extra progress grace; latency-sensitive
mode stays fail-fast. Hard identity/schema/resource/native IO/storage/cleanup
failures remain hard. Full S1-S3 stays active; independent software work proceeds.
