# First microphone no-copy witness

## Diagnostic slice and verification

`883a5be` integrates independent author `6b7d321`, limited to four production
driver files and the existing capture-boundary test/extractor. The first
actual MIC `SilencePackets` increment in each stream generation now retains
its original endpoint/session/generation/gate, state/open/ready/format/shape,
native sequence, existing callback clock/frequency and counter before/after.
Matching event layout and actual Notify result attach separately without
relabeling the original gate. Successful all-zero WRITE is not no-copy.

The fixed observation is144 bytes; complete capture diagnostics664 bytes.
There is no new allocation, lock, clock read, RT trace, PCM scan or public ABI.
Original silence/drop accounting, gate guards, PCM handling, periods, budgets
and defaults are unchanged. At most four new passive WPP fragments are exported
per stream generation; tracing disabled does not consume pending evidence.
Only the first increment is covered, and generation reset can erase an
undrained witness. Missing exports remain unknown. Notify success is not
proof of client acquisition or persisted PCM identity.

Author retains a real production-routine RED before GREEN. Root independently
passes all17 MSVC host suites, including54 pause-fence scenarios and8 new
no-copy scenario groups. The author additionally runs the focused real-routine
test under strict Windows GCC and actual WSL Ubuntu GCC13. Earlier shim
`Clock Clock` compilation failures remain retained; fully qualified test-only
types repair them without warning suppression. Root default/opt-in WDK analyzed
builds and native INF verification pass. Independent six-file source review
finds no blocker; root verifies every reviewed file hash after integration.

Matching PDB/TMF validation confirms all four new decodable schemas and exact
12/9/13/10 fields/descriptors with no tracepdb warnings. Existing unserved TMF
checks also pass. The unchanged native runner's period42/provenance63 fixtures
pass; its only predecessor differences are evidence roots, trace prefix and
new driver pins. The original18d1552 probe binary is deliberately reused to
isolate the driver diagnostic change, not relabeled as newly built883a5be.

## Frozen package and actual physical-target installation

Evidence root: `D:\coding\moiren-evidence\20261031-mic-no-copy-witness`.
The package-only INF is version3.1.1.610 dated10/31/2026; tracked source INF
is unchanged. Frozen manifest SHA256:
`74494541104F97AE74EF99F98139891BA3BE557FE4C4BC8A67FBEE034B5415E7`.
Signed175000-byte SYS SHA256:
`94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`.
Matching PDB SHA256:
`F5D63D11D6C06E388FA76C53CE9F3DA39AD065ADFB22ECD66F568F5C4C680EF7`.

The host retains its existing untrusted-root verification result without
changing trust. On the exact separate physical target, existing pinned trust,
both package signatures and all eight signature/catalog-member calls verify
successfully before mutation. A fresh609/oem203 identity and retained rollback
are checked. Root independently executes121 PS5.1 helper/state-machine
fixtures, with frozen deployer SHA256
`2DC78DCFB5C196AAD8935D34C82A0769AD11D3FFD490523E69762C9F5383AC74`.
Mock fixtures are not themselves native installation evidence.

Actual installation starts2026-10-31T05:44:28.7946297Z and finishes
05:44:39.1627004Z. Install and exact-root restart each return0; binding becomes
oem204.inf/version610, Running/root PnP OK/problem0 with matching installed SYS
and INF hashes. The transitional binding is already the healthy new driver.
Boot remains2026-10-30T09:50:49.6598450+08:00. No reboot, automatic rollback,
trust change, volume/default write, app replacement or package removal occurs.
The installed service/PnP/file identity is not independent in-memory attestation.

## Both native attempts retained

Both use exactly Playback/Input -> unity Bus -> MIC/Output, no physical render,
source default480/MIC minimum144, Engine32, target1024/capacity4800/jitter96,
recovery8, correctness3000ms and the original indexed29-marker source schedule.

- `no-copy-1` exits1 before native recorder Start: the selected MIC becomes
  inactive/unavailable after successful144-frame preparation. Source does not
  start and PCM/metadata are empty. Collection/quality fail; cleanup completes.
  This does not prove the new diagnostic caused endpoint invalidation.
- After confirming terminal retirement and no owners/children, the unchanged
  `no-copy-2` runs13:45:49.0817984 to13:46:22.5253376 local time. It exits0,
  completes source submission/collection/cleanup and retains10110 packets,
  1455840 frames/11646720 PCM bytes. Quality remains incomplete and unqualified:
  MIC active silence0 becomes final1, cleanup reset/recovery1 and unknown
  coverage remain. The earlier failed attempt is not discarded by this retry.

## Exact new driver boundary, not cross-layer packet proof

The second run exports all four fragments with matching endpoint4,
session1004990359638, stream-generation1, gate2, native-sequence10109:

- Actual no-copy increment: Present, Running, gate-open0, audio-ready1,
  Float32/stereo144 frames, counter0 -> 1.
- Existing callback QPC1005300713908, frequency10000000Hz. Raw ticks therefore
  have the same numerical scale here as the control QPC100ns domain; that
  equality must not be assumed for arbitrary frequencies.
- Event gate2, two packets/index1,1152 packet/native bytes, offset0;
  packet-start QPC1005300681210. Actual matching Notify returns success.
- Gate-close call[1005300680090,1005300697011], first measurement stop
  broadcast[1005300697021,1005300697032], worker-observed stop1005300714279,
  native Stop[1005300714288,1005300730612]. The recorded no-copy belongs to
  closed-gate/Running and its existing callback sample is after broadcast but
  before worker stop observation/native Stop. Do not treat it as the exact
  instruction timestamp of the counter increment or as proof that the
  increment precedes entry into the user-side Stop call. A callback can still
  hold stream State while Stop waits; driver Stop drains callbacks before
  completing.

This closes the previous missing original gate/state/native-event attribution.
It does not establish the retained PCM packet identity: last recorder packet
10110 has sample timestamp1005300713908 and is entirely zero, but its schema
does not carry driver native sequence/gate or an independently verified mapping.
The numerically equal timestamp and adjacent sequence numbers alone are not
cross-layer proof. The driver's event packet-start timestamp is a distinct
coordinate. Scheduled postroll and successfully copied zero WRITE are also
possible zero PCM. No tail-quality exemption or counter suppression is added.

Matching decodes process116/164 events for the two attempts, with zero reported
ETL loss, format errors or unknown formats. The second retains one
operation-unidentified `STATUS_REVISION_MISMATCH`. No rebase/active-unserved
message is exported; bounded/passively drained diagnostics do not prove those
events universally absent. Originals and negative results remain retained.

## Retention and remaining work

`no-copy-native-evidence.zip` is1313257 bytes,68 payloads plus manifest, SHA256
`3888DCE03AA2B30F7181A7A0A90FBB016A10924E7519CFD8169FC4B2EA39AD09`.
Root verifies CRCs, exact membership, all lengths/SHA256 and extracted bytes,
including deployment logs and both attempts. Independent native archive audit
finishes96 checks with zero failures. Its final JSON SHA256 is
`8F90EF2ED96DC175BF992E2672D6A2DC259DCEC671D19D8FF04BDDF9452E0B5C`.
The original91/1 audit is retained: its overly broad inventory assertion is
replaced by five explicit checks, not silently deleted. All four actual
catalog snapshots retain the same six default roles and12 preexisting render
master-volume/mute controls. The first failed run's after snapshot adds exactly
the owned source MMDevice entry, absent again before the second run; only the
second run's full render inventory is unchanged. A catalog entry is not proof
of a still-present PnP child. Single-snapshot catalog `after:null` fields are
preserved. Launch/result `ControlRunnerSha256` retains historical wrapper
metadata; the actual wrapper is separately pinned at SHA256
`42B7A0A6887146846F46873CF0151ED9983DA64A72DF672B3C9B6E83EFAD1FF4`.

Fresh05:49:18Z target audit confirms driver610 Running, root OK, unchanged boot,
no audio/build owner or virtual child, retained609 rollback and about222GiB
free onD. No cache clean occurs. Next investigate a retirement boundary that
preserves MIC service until native capture stops without reintroducing stale
gate/reset races; do not erase counters or reclassify unknowns into a pass.
S1-S3, complete waveform identity, matched physical routes, sustained stability,
calibrated sub10ms and an evidenced device-limited theoretical floor remain open.
