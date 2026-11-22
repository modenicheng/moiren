# Native-first MIC retirement, 2026-10-31

## Repair and verification

Probe commit `8b574cc` integrates independently reviewed `8a40870`. The owned
Playback -> unity Bus -> MIC collector now keeps MIC admission, service and
its gate alive until the recorder's native Stop/release and worker join finish.
Playback closes first, then both external Stop requests are broadcast before
any completion wait or join. Native completion has a separate three-second
wait; this is not a deadline for all existing teardown or sink joins. Global
admission invalidation, MIC close and host shutdown follow native completion.
Physical-route retirement retains its previous order.

Timeout, failure and panic still attempt every later cleanup stage, retaining
original errors and counters. Missing ownership does not synthesize Stop or
completion success. Admitted stream identity must match its first four stamp
components, with monotonic gate epoch; a stale close is an error, not a retry
against a replacement. Partial pre-admission cleanup retains the original
CREATE identity rather than inventing an admitted stamp. Driver, production
provider, buffers, defaults and quality rules are unchanged.

Author scheduler/order and partial-CREATE regressions retain genuine RED then
GREEN. Independent source review finds no blocker. Root runs the integrated
618-test probe suite successfully, strict example/test-profile Clippy and
formatting; release build succeeds. A fresh post-archive rerun again passes
618 tests, strict Clippy and formatting. Author Linux-target compilation is
not Linux test execution or native Windows qualification.

## Fixed native control

Target is `LAPTOP-NP49RG6H`, reached through `frp-rug.com:14857`. Direction
remains virtual speaker/Playback = Moiren Input/Source, virtual microphone/MIC
= Moiren Output/Sink. This run opens no physical render endpoint and makes no
volume, mute, default-role, privacy, trust, install or reboot change.

The deployed probe is7253504 bytes, source `8b574cc`, SHA256
`02987253E9E1C78A6F7C4B08AA33E81B7C7ECA1A279F3F87736D44C477CD940B`.
Installed driver remains diagnostic `883a5be`/`oem204.inf`/`3.1.1.610`, SYS
SHA256 `94EAE9BC2F1611C38F050977F7E740A09FE8C5B8EB19BC8780A82036FD0D32C7`.
App remains main-PR50-integrated `74f173f`, with609/608/607 rollback retained.
On-disk SYS/PnP identity is not independent in-memory-image attestation.

The original30-second indexed-v2 schedule has29 unique1024-frame markers at
amplitude0.025 and1440000 source frames. Source Playback stays Exclusive480;
only the MIC requests its queried Exclusive minimum, actual144. Engine32,
target1024/capacity4800/jitter96, declared packet/gap480, recovery cap8 and
correctness3000ms remain fixed. Progress grace is neither permitted audio
latency nor a waveform-quality exemption. This is not the previously failed
both-minimum configuration and does not admit a smaller default.

Frozen runner SHA256 is
`224F3107DCD6EFA1F40BB694434462AEF1B0DBE3D2A64E311685256F1496E56A`.
Root reruns42 period and63 provenance fixtures before acquisition. Exact local
build/native pins, runner and input copies are separately retained; a pin file
inside an archive is not its own independent provenance proof.

## Native-first-1 observation

The controller runs UTC06:19:08.0628253 through06:19:43.7545785. Probe and
controller exit0; source, collection and cleanup complete with no retained
runtime or cleanup failure. Recorder stores10110 complete144-frame packets,
1455840 frames and11646720 PCM bytes. Sink counters retain no missing or
incomplete packets, sequence gaps or nonfinite samples. Clean packet flags
and storage counters alone do not prove source identity or waveform continuity.

Active Playback and MIC retain drop0/silence0/reset0/recovery0. Both final
Removed/Closed providers retain drop0/silence0 and separate reset1/recovery1.
The preceding unchanged-driver control had final MIC silence1; this sole new
control has0. That is a retained observation of the changed retirement order,
not repeated-duration stability, exact cross-layer packet attribution or
permission to discard cleanup counters.

Control QPC100ns windows retain the following order:

| Boundary | Before | After |
| --- | ---: | ---: |
| Playback close | 1025310688060 | 1025310695608 |
| First measurement Stop broadcast | 1025310695615 | 1025310695627 |
| Native completion | 1025310695629 | 1025310747224 |
| Actual native Stop | 1025310703224 | 1025310719948 |
| Native worker join | 1025310747003 | 1025310747194 |
| MIC close | 1025310747237 | 1025310751494 |

Worker observes Stop at1025310703182. These are control/native-call windows,
not sample arrival, exact driver-increment timestamps or physical latency.
Matching610 PDB/TMF decode processes159 events with zero reported loss,
format errors or unknown formats. No no-copy/rebase/unserved/revision text is
exported; bounded/passively drained diagnostics do not prove universal absence.

Playback classification retains15 unknown live reads, final coherent evidence
after owner join, startup-prime32 frames and zero recovery-prime/short-read/
xrun/steady-delta counters. A missing versioned live read is not a zero-fault
sample and cannot be specifically attributed to contention from these fields.
Joined virtual clock has45562 periods/1457984 frames, late-wake bursts249,
maximum backlog2, clamp0/reset0, recovery cap8 and no saturation. Provider
cleanup reset/recovery1 is a distinct boundary, not this clock's reset counter.

`quality_passed=true` means no recorded collector-quality failure in this
completed operational run. Both `quality_complete` fields and `qualified` are
intentionally false in the current collector, not computed false because of
unknown15. Do not rerun merely to clear that fixed field or change it into a
qualification claim. Native recording identity analysis remains separate.

The existing exact-nominal indexed-v2 analyzer processes this verified PCM
without fitting an SRC model or lowering thresholds. The same support-based
window rule (>32 zero-frame separation,64-frame padding) gives29 unlabelled
windows; every window competes against all29 codes at0.7/0.05/0.0001, with
163714048 charged correlation samples under the256000000 cap. Author and
independent root CLI reruns produce exactly equal JSON and exit1:12 unique
identities,17 missing and no duplicate. Six missing candidates fail global
correlation;11 pass0.7 but fail section coverage. This does not prove17 lost
markers or that retirement worsens waveform quality. The previous recording's
14/29 result belongs to that separate sample, not a controlled signal-quality
comparison. Stored coverage is complete, but9830 packet intervals retain the
existing nominal-QPC diagnostic-policy refusal, not9830 lost frames. Source
origin, continuous identity and physical latency remain unknown.

Analysis output SHA256 is
`B4CE548D770D397485CA6E40BACA8066F32886BF8F787CB97AEC72075A554E79`;
the reproducible invocation/config/input receipt SHA256 is
`C5DEE117D21E14B8D68641E616B5AE843BB11881F5BB23ECA63466826AAA00E6`.
The separate synthetic FIR/affine tests do not silently extend this CLI or
replace its retained native refusal.

## Evidence retention and remaining gates

Host evidence is
`D:\coding\moiren-evidence\20261031-native-first-mic-retirement`.
`native-first-1-evidence.zip` is2120270 bytes,27 payloads plus manifest, SHA256
`B22B9E42CC48A3D7A5131927CB7E8112D8F311CA17673B3D65C1D4A3F054F123`.
Root verifies CRCs, exact member set, every length/hash and extracted bytes,
plus exact frozen-local pins/runner/helper/inputs and their result relationships.
Previously downloaded collection/report/ETL previews match archived originals
byte-for-byte; decoded ETL therefore belongs to this verified run.
Independent acquired-evidence audit passes107 checks with zero failures; the
separate draft factual check passes13. Audit JSON SHA256 is
`9E3A701F0E29C2E77073E891871C0F83F22CC96120EF4CCE72407DAF15089F49`.
The original reviewer-check RED is retained: an unanchored ERROR token matched
DEBUG `error=None`; correcting the log-level matcher does not remove any
native error. Full PCM inspection retains9861 all-zero packets, including the
last544 complete packets. Sparse source intervals and postroll exist; no zero
packet is deleted or silently exempted and support groups do not assign marker
identity. That audit verifies retained facts, not continuous-waveform quality.

The first helper review found three P2 issues: unbound provenance, unchecked
reparse ancestors and failed pre-artifact runs lacking an integrity receipt.
Corrected real RED controls and final100/100 host fixtures, including actual
PS5.1 archive-to-Python round trips, are retained. An independent13-check
review closes all three findings. A confirmed nonzero run with no collection
is integrity-verifiable as unavailable/unqualified, never accepted audio.
The helper assumes quiescent files after terminal/no-owner checks; this is
not an adversarial concurrent-filesystem or untrusted-ZIP service.

Fresh06:25:06Z read-only audit finds unchanged boot,610 Running/root OK,
matching app/probe/SYS pins, no audio/build owner or VAD child, unchanged
default roles and render controls, and239017684992 bytes free onD. No cache
cleanup or original-artifact deletion occurs. Legacy CIM `IsSigned=false`
is retained; this audit is not a new signing/trust acceptance test.

S1-S3, exact changing-waveform continuity, matched physical source paths,
sustained lower-budget stability and calibrated physical latency remain open.
Current evidence proves neither sub10ms impossibility nor a theoretical
current-device floor. The user's conditional device-limited objective remains
in force; a queue model or this single software-route run cannot establish it.
