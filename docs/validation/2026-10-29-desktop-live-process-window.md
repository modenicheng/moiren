# Desktop live observed-window Process admission

Date: 2026-10-29. This is an explicitly selected extension of the existing
bounded S3 desktop configuration, not a new continuous-window guarantee.

## Intended behavior and boundaries

The production desktop already has per-wake measured selection and stopped
retained-prefix evaluation/Apply. A separate opt-in live observed-window page
uses the existing `ApplyWindowedProcessBridge { request, policy }` service
request. It must require an explicit choice for each of the five policy
fields, retain the exact source/settings/owner identity, and distinguish
request consumption, asynchronous replay selection and confirmed activation.
Neither mode nor policy may be inferred from an unrelated selected profile.

Missing, overwritten or unsupported evidence must remain a visible refusal,
without a per-wake fallback or a guessed policy. The previous owner and editable
draft remain distinct from candidate intent. A finite admitted prefix is not
continuous quality, source sample age, future-safe scheduling or calibrated
physical latency; `qualified` remains false.

The current native measured histories are capped at256 complete-drain and256
whole-wake events; the replay archive is capped at8192 packet/read events per
writer. Longer checkpointed windows and replay combined with ingress elapsed
are unsupported extensions, not silently added prerequisites to original S3.
The original requirements permit visible refusal of unsupported budgets.

## Actual test-first evidence

The separate target worktree starts at clean
`5382369973361e0e3750c769c85a7476439163a6`, with only the two new desktop test
files patched. Patch SHA-256:
`3A3A31132248ED6F05523E01115E9339263866348AB428215E3C766DAE243782`.
The actual command is:

```text
cargo test --locked -p moiren-app --lib --features gpui-test desktop_live_window -- --test-threads=1
```

Compilation succeeds. All six tests fail as expected before implementation:
one pure state regression accepts a windowed summary for a per-wake request;
five real GPUI click regressions cannot find the absent opt-in control.
Exit is101, without watchdog expiry. The retained logs are
`D:\MoirenBuild\logs\desktop-live-window-red` and
`target/physical-1921681297-20261029/desktop-live-window-red.stdout.log`.
These are actual behavioral failures, not a compilation-only RED claim.

## Verification history

The first implementation passes all6 new tests and67 measured regressions.
The first full application run exposes a real viewport acceptance failure:
the additional opt-in control moves the Expert warning below the initial
viewport. A bounded actual-scroll correction preserves the original visible,
complete-label and unsaved-draft assertions. Its exact test then passes, and
the subsequent full app command exits0. Logs retain the original failure;
it is not relabeled as a flaky timeout or suppressed assertion.

Independent read-only review identifies a separate confirmation bug: a later
`RefreshCatalog` receipt is incorrectly treated as cancellation even though
the service deliberately preserves its background window evaluation. A real
tracked-mailbox regression is added before the production fix. The final
review and verification below supersede that initial checkpoint.

That exact refresh regression compiles and fails on the original equality
guard: `a newer harmless catalog receipt does not supersede live evaluation`,
0 passed /1 failed, exit101, no watchdog expiry. It uses two actual tracked
mailbox refreshes, before and after selection, and requires exact new-owner
confirmation rather than treating either receipt as activation. Original
failure logs remain under `logs\desktop-live-window-refresh-red`.

The minimal repair keeps cumulative `consumed_request_id >= request_id`
acknowledgement and all exact policy/mode, source, settings, selection and
new-owner checks, without requiring equality with the latest receipt. The
actual refresh regression then passes. Final independent review finds no
remaining Important/Critical issue in the six-file scope. A cancelled request
whose projection is otherwise unchanged may wait for the existing30-second
deadline; that is not erroneous activation or proof of service completion.

The exact final six-file binary patch matches the host and target SHA-256:
`828423822910D3450C517A4E9C4462F56F629ABF0F2D82A0AEC3A6F0376E71A3`.
New control strings are currently English; existing locale behavior is not
changed, and complete translation of this opt-in experimental page is not
claimed by the bounded integration.

The exact reviewed patch is verified on the separate target, without native
audio acquisition or local heavy compilation:

| Check | Result |
| --- | --- |
| New desktop regressions |7 passed |
| Exact RefreshCatalog regression after repair |1 passed |
| Existing Expert viewport acceptance |1 passed |
| All measured regressions |68 passed |
| Full app with gpui-test, serial |633 passed /4 ignored;13 summaries |
| Strict app all-target Clippy with gpui-test |Passed |
| Workspace formatting and scoped diff |Passed |

All final commands exit0 without a watchdog. Final deployment and hardware
evidence remain separate from these software checks. Logs:
`D:\MoirenBuild\logs\desktop-live-window-reviewed`.

Do not interpret the fixture's software service as native audio evidence. It
preserves CallerSupplied trace origin and uses the existing cfg(test) proof
seam; production proof issuance and owner replacement are separate.
