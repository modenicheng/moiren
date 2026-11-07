# Native rename contract and desktop notice

Date: 2026-11-02, Asia/Shanghai. Base: `3ece138` on
`feat/minimal-io-latency`. This is a small desktop correctness increment and
reconciliation of existing naming evidence, not a native rename repair or an
S1-S3 waveform/physical qualification PASS.

## Do not repeat the old rename experiment unchanged

The [2026-10-28 current-driver receipt](2026-10-26-minimal-io-latency.md#2026-10-28-bounded-rename-rerun-on-the-133d31d-driver-vm-authorized)
already establishes a concrete unresolved result: a same-stable-ID name change
creates a different Windows MMDevice GUID; microphone names recompose with the
INF device suffix, while playback still displays a localized generic speaker
name. Provider CREATE names are not exact Windows endpoint-name success. The
historical strict-held expectation also fails independently because holding an
initialized WASAPI client does not guarantee that the old PDO remains present.
Neither failure is erased or rerun here.

Current source still uses the requested name to derive the ACX circuit/interface
reference in `driver/vad/src/EndpointReference.h`, and the playback bridge pin
uses `KSNODETYPE_SPEAKER` in `driver/vad/src/audio/Circuit.cpp`. Current service
and desktop distinguish requested, provider-created and observed Windows names.
No native result for the current target is inferred from the older VM receipt.

Two documented platform constraints bound the next product decision:

- [Audio Endpoint Builder Algorithm](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-endpoint-builder-algorithm)
  specifies a hardcoded localized Speakers name for `KSNODETYPE_SPEAKER` and
  says neither a driver nor a third-party application can alter that speaker
  name. Changing interface staging alone is not a justified exact-name fix.
- [Default Audio Endpoint Selection](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/default-audio-endpoint-selection)
  identifies reference strings and pin IDs as inputs to endpoint identity and
  documents their stability requirement to preserve associated user settings.
  A new MMDevice GUID on every new requested name has a real reselection/settings
  consequence; a stable Moiren project ID does not make Windows identity stable.

These sources do not establish that no supported design could ever meet the
user requirement. They do mean the present Speaker-category/recreation contract
must not be qualified by retries, substring/name normalization, a lower gate,
speculative registry/property writes, or silent endpoint-category changes.
Any subsequent native solution must explicitly reconcile category, identity,
permissions, external-client behavior and exact observed name. No driver,
property store, category, reference, defaults or audio policy changes here.

## Small user-visible correction

Each actual Rename control, for both export directions, now has an accessible
English/Simplified-Chinese notice: native recreation can interrupt audio or
require other applications to reselect the endpoint, and requested text is not
proof that Windows exposes it. The existing observed-name and exact-match rows
remain authoritative. No success state, threshold or matching rule changes.

The control continues to dispatch only the existing candidate `SetExports`
edit. It does not start/apply audio, acquire a native owner or write Windows
properties. Stable project identity, binding, sibling export and the observed
graph remain unchanged until normal service processing. Existing house styles
and Label accessibility semantics are reused.

## Regression and isolation

Tests precede implementation. The actual headless UI RED fails because neither
Rename control has the notice; the independent locale RED fails because the
Chinese key falls back to English. Both are behavioral failures, not compilation
errors. The fixed notice passes on both directions at 1440x1600 and 1024x720;
scrolling must fully reveal it and keep its bounds within the device page. The
same test clicks Rename and verifies the exact intent-only request and unchanged
sibling/identity/binding, without starting audio. All eight explicit-locale tests
pass without mutating process-global locale.

Final-source full application `cargo test --locked -p moiren-app --features
gpui-test` passes 721 tests with 4 ignored across 13 summaries. The focused
two-viewport test also passes independently. Application/all-target strict
Clippy with `gpui-test`, workspace rustfmt and diff checks exit 0. The full
application run includes the existing export controls, native-name presentation
and 13 headless acceptance cases; it is not a workspace, Linux, CI or Miri rerun.
The earlier full run remains separate from the final run after adding compact
viewport assertions. No timeout, threshold or ignored-test gate is changed.
Independent read-only review finds no issue in the six-file source/document
slice and confirms the receipts and remaining native qualification boundary.

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s4-rename-contract-20261102-112216`.
An independent `3ece138` archive receives only the five intended application,
test and locale files. Uncommitted changes in the eighteen preserved paths are
excluded; tracked versions remain at their commit content. No host application
window/native device, remote process, OBS recording, driver deployment or memory
update is performed. S4 native cases and S5 lower-default qualification remain
open, separately from the completed software integration checkpoint. First-packet
work stays deferred. No repeated soak is justified by this notice-only change.
