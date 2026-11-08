# One policy-owned publication reference

Date: 2026-11-08, Asia/Shanghai. This implements an enabling part of the
[naming boundary](2026-11-08-vad-name-identity-contract.md), not final immutable
adoption, Windows name publication or S4 acceptance. The remote signed613
package remains untouched; no driver deployment or native test occurs.

## Source change

The passive policy worker now owns one child-lifetime PolicyReference derived
from its locked slot snapshot. Audio/flow/Realtime registrations and ACX's
circuit name consume that exact storage. Audio initialization no longer hashes
the mutable name independently or retains another authority. The initializer's
declaration, definition and sole caller change together; no public ABI changes.

Policy's existing retained-registration path refuses incomplete/empty or
unterminated references. Circuit building checks successful complete policy
and a bounded reference before any ACX allocation. Release/failure invalidates
the reference; PnP serialization and the existing passive worker flush order
publication before construction. No registry/hash/lock/allocation is added to
an audio/RT callback.

The historical name hash deliberately remains unchanged. This avoids silently
rotating installed endpoint references while enabling the future shared
resolve/adopt/persist step. It does not preserve Windows identity across a
rename yet; no constant-reference switch, endpoint-history reset or new rename
IOCTL is introduced. Final supported MMDevice name publication remains open.

## Verification

Missing-reference/retained-reference regressions have RED/GREEN evidence.
Production-extracted tests cover changed live name after the locked snapshot,
all three category references, cached reference refusal, failure rollback and
actual Policy -> Prepare -> BuildCircuit composition. The composed suite is
wired into both ordinary test runners, not left as a standalone extra runner.

- Windows MSVC host22suites pass, independently rerun by the parent.
- Windows/GCC portable17suites pass in the implementer's runner.
- Ubuntu24.04/WSL portable17suites pass, independently rerun by the parent after
  the review correction to the fixture's SDK indirect-string constant0x19.
- WDK analyzed build and InfVerif pass; the17frozen613package/PDB/TMF pins remain
  unchanged. This newly built unsigned source is not the installed613image.
- Rust VAD library regressions pass275tests; workspace formatting passes.
- Independent source and final-delta review has no remaining actionable finding.

Evidence reuses `D:\coding\moiren-evidence\20261108-112100-collector-worker`
and its `s4-*.log` receipts. Wide-ABI adaptation changes only SDK/DDI fixture
types and wide-libc calls, not extracted production bodies. These software
checks do not prove native PnP races, cleanup, exact Windows names/settings,
streaming/held-client rename, waveform or sustained calibrated latency.

Next product work remains immutable machine-local identity/adoption and actual
mutable Windows name propagation, followed by unchanged S4 native cases. The
full S0-S5 goal remains active; the user still owns any needed system reboot.
