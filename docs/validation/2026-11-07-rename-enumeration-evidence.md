# Failure-time enumeration evidence and direct remote rename

Recorded November7,2026, Asia/Shanghai. This continues the full S0-S5 pipeline;
it does not qualify native naming, continuous PCM, physical latency or a new
buffer default. The standalone remote computer is used directly, without VM
startup or desktop input/unlock. Development-host work is build/pure tests only.

## Question and diagnostic scope

The preceding corrected-source run at19:01 retains Playback's replacement
MMDevice enumeration timeout. This occurs before `Stream::prepare`, not inside
WASAPI Initialize. The harness selects ACTIVE endpoints; the production Windows
name reader enumerates ALL states before exact root/stable-ID/flow matching.
The old receipt has no failure-time ALL-state snapshot, so it cannot distinguish
inactive endpoints, absent replacements or unavailable topology identity.

Commit `9e5a5e3203c748a441171afad091b0ba196bda57` adds one failure-only metadata
snapshot to the existing held-client mode. ACTIVE selection and subsequent
exact-device revalidation remain the only client admission path. No ALL-state
entry initializes a client, replaces exact FriendlyName, changes the product
policy, or authorizes an acceptance result. No new option/round-trip is added.

The report first persists the original failed checkpoint and pending metadata,
before diagnostic COM calls can stall. It then records up to512 requested-flow
MMDevices with exact ID/state, topology/PnP identity match, raw FriendlyName,
opaque adapter/connector references and typed failures. Scheduling stops after
one second within the original case deadline, with checks between metadata
stages and explicit NotAttempted values. This is not a hard COM timeout: an
individual native call/topology stage remains bounded by the original process
watchdog. Partial rows, truncation and collection-completed state stay explicit.
The original error is returned unchanged, including collection failure.

## Software evidence

Local evidence namespace:
`D:\coding\moiren-evidence\20261107-rename-enumeration-1920`.

- A compiled behavioral RED proves the old Case serialization omits the
  failure-time evidence field. The initial visibility compile error is retained
  separately and is not counted as RED.
- Final Windows example tests pass152. Coverage includes successful admission
  without diagnostics, original-error retention, publication before a simulated
  nonreturning collector, mid-row budget refusal, count/time/read truncation,
  contextual HRESULT serialization and unchanged existing acceptance tests.
- Exact eight-file source overlay hashes match in a separate Linux archive.
  Debian's existing stable1.99.1 toolchain passes130 example tests and strict
  example Clippy. The initial Ubuntu invocation has no cargo; the earlier
  Debian overlay exposes a dead-code warning in the untested Value variant.
  Both failures remain archived. Final tests exercise the real budgeted Value
  path instead of suppressing the lint or changing acceptance.
- Strict Windows workspace/all-target Clippy with gpui-test and the integration
  feature passes; formatting and whitespace checks pass. Independent final
  source review finds no actionable blocker after two publication/budget issues
  are corrected. This is not a new full-workspace test count or exact-head CI.

The clean optimized release is7892480 bytes. Its source is9e5a5e3, tree
`1e25b4014748717fbc211c0b00e40c5bbd89a00f`, and SHA-256 is:

```text
1FE017F706C1A1968366F36FFCCFC896734D9D75BAB5FDDDBBF3AEF2433CFFF8
```

## Direct remote observation

The same explicit fixture seed202611071825 and held-rename flags run once in
fresh remote root `D:\MoirenTest\20261107-rename-enumeration-1920`, retaining
the installed612 package and the frozen driver/helper/safety contracts. The
adapted worker's eight real-invocation/refusal checks pass under each of PS5/7.
Worker SHA-256:
`5E8ABF34B59C568BE2BA49EFC00495C29335D7B21568192B6438652AA9BA8DD3`.

At19:29:31..47, the native report fails acceptance after7916ms:

- Both held clients and both replacement clients initialize/release. Both
  direct old-Removed checkpoints and fresh Engine confirmations succeed.
- Neither direction experiences an enumeration failure on this run, so both
  native_enumeration_failure fields remain null. The diagnostic collector is
  not exercised natively; portable/helper tests are not substituted for that.
  This observation does not prove the earlier transient timeout is repaired
  or explain its cause. No unchanged retry is justified by its non-recurrence.
- Both exact FriendlyName gates fail, with requested base names followed by
  `(69- Moiren virtual audio endpoint)` for Playback and
  `(70- Moiren virtual audio endpoint)` for Microphone. There is no suffix
  stripping, DeviceDesc substitution or display-name acceptance.
- Native Starts remain0 and no PCM is verified. Join/control release succeed;
  cleanup_error is null. The later wrapper finds no test owner/present child,
  unchanged boot/612 driver/default roles/volumes/security and protected-client
  identities. Its sole audit error retains the failed native report; it does
  not mean join/release failed or prove uninterrupted protected-client audio.

All17 terminal files are independently verified remote/local/ZIP by lengths
and SHA-256. The42814-byte `native-fixed-seeded.zip` SHA-256 is:

```text
5F1E657161F0ED87B3C186CC87E65526D8220FE0508D6864013FBAF825688F0F
```

No driver installation, reboot, security/default/volume write, protected-client
operation, new A-to-B-to-A mode, raw-audio transfer or host native run occurs.
Exact naming, the remaining native lifecycle/default/sibling/race/repetition
matrix, ordinary desktop acceptance, sustained waveform/load and calibrated
physical p95/p99 remain required and unqualified.

## Independent PCM refusal and both-flow failure metadata

The existing fixed-identity PCM fixture runs independently at19:34:42..35:05
with9e5a5e3, without a rename or fixture-seed modifier. The report fails after
15141ms with `owned MMDevices did not arrive; topology errors: []`, before
the probe initializes/starts its native client pair. Its4002ms idle observation
retains679 forward Engine timeline advances, including162 after the former
three-second boundary, with zero timeline epoch changes. Starts0 does not mean
the service/Engine has done no native work. PCM remains unverified, identity is
null and all recorded sample counts are zero. Cleanup_error is null and service
join/control release succeed; safety and terminal idle audits pass. Its only
wrapper audit failure is the failed native report.

This original timeout lacks failure-time PCM enumeration metadata. Commit
`831830805dd85dea4391350417aca82481d4c6f1` extends the existing failure observer
to both directions of PCM Pair preparation. The ordinary Pair::prepare wrapper
retains its original streaming semantics. Both diagnostic flows share one
one-second scheduling budget inside the existing deadline. Original failure and
both pending records are published before COM reads; each completed flow is
then published separately, so a second-flow stall cannot erase the first flow's
metadata. ACTIVE-only admission, exact root/stable-ID/flow matching and all PCM
gates remain unchanged. There is no new command-line option or automatic
enablement, identity rotation, suffix stripping or migration acceptance.

Final-source Windows example tests pass154 and Linux stable tests pass132.
Strict Windows workspace/all-target Clippy with both opt-in features, Linux
example Clippy and formatting checks pass. The final Windows example test
run is freshly repeated before the documentation checkpoint and again passes
154 with zero failures. Coverage includes explicit JSON key presence and
retention of first-flow metadata during a simulated second-flow collector stall.
Independent final source review finds no actionable blocker. These are software
checks, not native audio acceptance.

The clean8318308 optimized release is7915520 bytes, tree
`69511e21ad4b0ecc2c1ed63be006455e60b7a179`, SHA-256:

```text
18F49ED3F458288E6865FF486C5E6435F31B2E5F3EE83F9514580AB874D898A9
```

Its bounded remote run at19:43:02..26 retains the same fixed fixture,612 driver
and existing `--exercise --pcm --timeout-seconds=90` arguments. The worker's
eight invocation/refusal checks pass on each of PS5/7. Worker SHA-256 is
`B955C92AAAD96CC0CB772A0F829AAED471366BC6F44633F2985CA98E92150B34`.
It fails after15539ms with the same mapping error. The4002ms idle observation
retains695 forward advances,172 after three seconds and zero epoch changes;
probe Starts remain0, no PCM is verified and cleanup/join/release succeed.
Safety and no-owner/no-present-child audits pass; report acceptance still fails.

The failure collector now runs natively, unlike the earlier held-rename run:

| Failure snapshot | Playback | Microphone |
| --- | --- | --- |
| ALL-state rows / elapsed | 97 / 320ms | 53 / 34ms |
| Completed / truncated / collection error | true / false / null | true / false / null |
| Exact root/stable-ID/flow matches | 1 | 1 |
| Exact matched MMDevice state | 2, DEVICE_STATE_DISABLED | 1, DEVICE_STATE_ACTIVE |
| Provider checkpoint MMDevice matches | Yes | Yes |
| Matched raw FriendlyName | Speakers in the target's Chinese locale, with `(8- Moiren virtual audio endpoint)` | `Moiren integration microphone (9- Moiren virtual audio endpoint)` |

Playback's matched MMDevice is
`{0.0.0.00000000}.{83b34e0c-fb74-435d-b573-18f1f549d0a6}`; Microphone's is
`{0.0.1.00000000}.{0a1d19f5-d83c-4009-a6b9-247c6348c5a5}`. Both match the
preceding live provider checkpoint, parent `ROOT\MEDIA\0001`, their exact
fixture identity and direction. Playback's connected topology already reports
OUTPUT_UNDEFINED and its interface FriendlyName is `Moiren integration playback`.
The many historical state4/failed-topology rows are not substituted for either
exact match.

Microsoft defines state2 as DISABLED and permits new client streams only on
ACTIVE endpoints. This explains why the matched Playback row cannot enter
the ACTIVE-only pair at the failure snapshot; it does not prove the endpoint's
state throughout the preceding wait, who disabled it, whether creation policy
was consumed, or that the earlier seeded replacement timeout has the same cause.
No lock-state diagnosis follows. EnableEndpointByDefault is documented as a
creation policy, not evidence that this persisted fixture's state was migrated.
Read-only exact-interface EP0 Association/EnableEndpointByDefault correlation is
the next discriminator; correct staging alone would still not prove consumption
or disablement origin. No enablement or historical-device deletion is performed.

Both terminal archives independently verify17 entries by length and SHA-256:

| Archive | Bytes | SHA-256 |
| --- | ---: | --- |
| native-pcm.zip | 28585 | B3FFA22315CD6933B2102858FBC72AEA9657F19D246EE5F2FDEF64E70345988A |
| native-pcm-observed.zip | 223417 | DA93A92788F59B316A5631CA140239CE0EC67F0DB72592DDF4FA02787F8ACC4F |

No raw audio is present or transferred. Neither refusal is a PCM, migration,
exact-name, default-role, desktop, sustained-audio or physical-latency pass.

## Published CI boundary

The prior published headc890e50 completes run42367038940 onNovember7 at19:45
with all20 jobs successful; its existing watcher exits0. This is not exact-head
CI for the later9e5a5e3/8318308 diagnostic changes or this documentation update.
The original29 draft/cache pins are reverified unchanged before these receipts.

## Official interpretation references

- Microsoft Learn, DEVICE_STATE_XXX Constants:
  https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-state-xxx-constants
- Microsoft Learn, PKEY_AudioDevice_EnableEndpointByDefault:
  https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/pkey-audiodevice-enableendpointbydefault
