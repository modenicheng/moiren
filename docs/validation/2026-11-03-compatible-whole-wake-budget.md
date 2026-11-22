# Compatible physical-master whole-wake budget

Date: 2026-11-03, Asia/Shanghai. Baseline:
`bbe4785dfdaac68e1f7af12e648bbdb34e244ab3`.
This repairs an original S2 preparation contract; it does not qualify native
waveform continuity, lower settings or physical latency.

## Reproduced defect

With default Compatible virtual buffering, `ProfessionalBackend` prepared
virtual resources before the physical master negotiated its actual demand.
The provider received the provisional Engine block times recovery-cap demand.
Later preparation calculated the whole native wake, including rate conversion,
FIR allowance and Engine rounding, and published that newer budget.

This was not merely a stale display. `WindowsVadProvider::prepare_microphone`
retains `request.prepared_budget()` in `WorkerConfig::Microphone`; reset reprime
therefore used the earlier budget even though diagnostics advertised the later
one. The existing explicit-target path already deferred preparation.

At Engine48k/block64, native48k/render512 and default cap8:

```text
provisional provider demand: 64 * 8 = 512 Engine frames
negotiated whole wake:       512 + 64 + 2 * 64 = 704 Engine frames
MIC packet480/carrier4800:
old modeled reprime threshold: 1026 native frames
correct modeled threshold:    1218 native frames
```

The modeled bounded wait also differs. Neither threshold nor a nominal wait is
measured transport residence or end-to-end latency. This mismatch is not a
demonstrated cause of a previous native discontinuity, format refusal or
`0x8007051A` cleanup result. In particular, the preceding pure-virtual
[Shared source attempt](2026-11-03-vad-source-mode.md) has no physical master
and is not repaired or reclassified by this change.

## Narrow fix and lifecycle

Default virtual resources retain early preparation only for the pure-virtual
clock. Every physical-master route now prepares virtual resources after the
negotiated whole wake is available, using the existing deferred path. Provider
and published budgets consequently derive from the same demand.

No native-demand formula, queue capacity/target, servo, reprime formula,
recovery cap, default setting, driver code or RT callback changes. Pure-virtual
Compatible and explicit-target preparation order stays unchanged.

Physical-master preparation failures now occur before opening virtual exports.
The previous test expecting two virtual sessions on such a failure is updated
to require zero requests, sessions or activation. If later virtual preparation
fails or is cancelled, the already parked physical owner must still wake,
join and release before preparation returns. This is deliberately tested,
not inferred from a successful output observation.

## Regression coverage

The new F32/F64 tests call real `prepare_with_native` with existing fake native
and virtual owners; no host audio device or desktop UI opens. Each precision
covers the following Engine/native rate and block/period matrix:

| Engine rate | Block | Native rate | Native period | Whole wake | Provisional |
| --- | --- | --- | --- | --- | --- |
| 48000 | 64 | 48000 | 512 | 704 | 512 |
| 48000 | 256 | 48000 | 128 | 704 | 2048 |
| 44100 | 64 | 96000 | 128 | 251 | 512 |
| 96000 | 64 | 48000 | 512 | 1216 | 512 |

Assertions compare actual provider requests, prepared budgets, microphone
reprime and published observations. Compatible remains the default options
with no explicit input queue. Fake activation and Stop/join verify ownership;
there is no virtual clock on the physical-master route.

Separate F32/F64 cleanup tests inject microphone preparation failure, a short
playback carrier and cancellation after playback preparation. Each joins the
one successful virtual session and drops the parked physical worker without
native Start/write. Existing pure-virtual failure tests remain unchanged.

## Verification evidence

Immutable local evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s2-compatible-whole-wake-20261103-0256`.
The source export starts from the baseline archive and excludes all27 original
dirty/cache files; only the four intended Rust files are overlaid for tests.

The actual behavioral RED has both precision tests fail with provider512
versus expected704. The initial full-library run subsequently exposes the
intentional ordering change through the old two-session expectation; that
failure remains archived rather than concealed. An expanded test first races
Stop against fake Start because observations publish at attach. It is repaired
by waiting for actual fake Start, not by weakening its lifecycle assertion.
These fixture failures are distinct from the original production-budget RED.

Final focused selection passes5tests, including four new budget/cleanup tests.
The full Windows audio library passes800tests/4ignored both default and with
`vad-probe`. A formatting-only correction follows the first rustfmt check;
the original formatting output is retained.

The exact exported Windows workspace passes2356tests/9ignored/0failed across
71harnesses with `moiren-app/gpui-test,moiren-windows-audio/vad-probe`.
Feature-on all-target workspace check and strict Clippy pass, as does workspace
formatting. The release probe build and its715tests/1ignored feature-on suite
pass. All commands and exits are retained separately. Independent read-only
code review finds no blocker and confirms that physical capture feeding only
a virtual MIC remains virtual-clocked with unchanged preparation ordering.
No Linux, Miri or remote CI result is implied by local Windows tests.

## Native boundary

A read-only remote preflight at03:06:47 +08:00 reaches `LAPTOP-NP49RG6H`,
finds no candidate test owner or running Moiren task, and confirms unchanged
protected OBS/QQMusic creation identities. No host native audio/UI operation
or process manipulation occurs. No unchanged soak is started for this fix.

One new-question native smoke then uses
`D:\MoirenTest\20261103-compatible-whole-wake-0315`, with the exact exported
release probe SHA256
`98526F5CC84343736401AFC70B86B5760684D0313784164263448EC77E2105C3`.
The route is owned Shared VAD Playback -> unity Bus -> fixed DAWN PRO2 physical
Shared output, Engine48k/block64. Input buffering remains Compatible; no MIC,
target-input override, smaller default or recorder is introduced. Physical
render requests period336, queue512, wake64 and callback64. The existing
six-second indexed amplitude0.025 source runs within an eight-second interval.

The Limited Interactive session1 task retains the tested held child with a90s
deadline and240s outer task bound, isolated TEMP/app-data and bounded logs.
Independent runner review finds an optional `Case` parameter could bypass the
selfcheck on omission. A metadata regression fails before both entrypoints
make it mandatory; final pins are regenerated and verified before any task
starts. PS5 syntax and eight closure cases pass. The dry-run selfcheck task
ends0 with matching intent and no audio acquisition; the single native task
then ends0. Audit completion is distinct from the probe's expected unqualified
exit1 and from audio-quality acceptance.

The actual probe runs03:19:14-03:19:28 +08:00,14.189s including preparation and
teardown. Shared VAD source opens1056frames/F32, acknowledges all288000schedule
frames and stops/joins normally. The physical master negotiates48k, actual
period336/capacity738 through Client3 without fallback, target512; published
virtual whole-wake demand is704. The joined master reports386912written frames,
1152wait wakes, zero native wait timeouts and zero reported drop/silence counters.
Host cleanup is Closed with no fault/cleanup error, final owner generation1
matches, the export reaches Removed, and no test owner remains.

These observations verify native activation/write/normal join only. There is
no waveform recorder, physical calibration or MIC reset in this route; the
private provider budget is not independently observable from the native report.
Its numeric equality is established by the portable regression, not invented
from the published native observation. Running provider counters have800
completed packets, zero drop/silence/recovery/epoch-reset counts, and the
running boundary retains32initial-prime muted frames with zero short-read
callbacks. The joined snapshot separately retains two cleanup resets/recoveries
and a discontinuous SRC checkpoint with `xruns=1`; these are not erased or
promoted into a whole-waveform/zero-fault claim. First-packet work is not reopened.

All43compact artifacts are individually length/SHA256 verified after archive
and download, without transferring raw PCM. The403526-byte archive SHA256 is
`5203E3E3510951B65C8BB6137A52AB22854D4C7B58139BAA6F4484A6084875FA`.
Protected OBS/QQMusic identities, loaded driver/boot and default roles remain
unchanged. Both tasks are Ready/0; nothing is left running as a soak.

First-packet investigation remains deferred. Native waveform/quality,
matched-path comparison, lower-target qualification and calibrated physical
latency remain separate open gates; lack of physical capture equipment does
not block independent implementation. No buffer promotion, weakened gate,
path winner, sub-10-ms result, push or S1-S3 completion follows from this slice.
