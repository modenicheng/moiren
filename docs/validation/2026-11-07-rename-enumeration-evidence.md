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
