# Native Process replacement and recorder follow-up

Date: 2026-10-29 on the development host. Physical target:
`LAPTOP-NP49RG6H` / `192.168.12.97`. Its unchanged wall clock reports
2026-10-25; cross-machine wall-clock subtraction is not a latency measurement.

The installed driver, deployed app `e195bfe` and probe `5382369` are unchanged.
This follow-up uses those binaries without compiling, cleaning caches,
changing default roles, changing volume/mute, or installing another driver.
The current [cache-retention policy](2026-10-29-remote-build-cache-policy.md)
applies; an empty target is a historical state, not a completion requirement.

## Standalone recorder finding

Three independent `record-measurement` runs capture only the existing Intel
microphone, without a Process graph or generated source. Each records 3 seconds,
300 packets and 144,000 frames at 48 kHz, stereo Float32. Native Start/Stop and
the sink finish; PCM is 1,152,000 bytes per run.

Each has exactly one raw discontinuity flag: packet sequence 1, flags 1,
device position 480, stored start 0. Subsequent packet positions have no
observed adjacency mismatch. Dropped frames, position gaps, overflow,
timestamp errors, silent frames, missing/incomplete sink packets and nonfinite
samples are zero. The same first-packet pattern appears in the previously
retained Process4 and VAD software4 recorder metadata.

This isolates a reproducible recorder startup observation without requiring
the Process graph. It does not prove the startup prefix was captured intact,
establish an environmental cause, identify exactly how many frames were lost,
or prove waveform continuity. Raw counters and the existing strict quality
failure remain unchanged. All three standalone commands exit 1 and remain
unqualified; there is no first-packet exemption or new PASS classification.

## Explicit native-period refusals

Three Process5 cases request output period 128 with Engine block 256 and the
same Realtek output. Exclusive Compatible and Exclusive target1024 both refuse
the requested rate/sample format before render Start. Shared target1024
refuses period128 against observed bounds480..480 and fundamental480.

All exit1 without watchdog expiry, have no live capture and no recorder PCM,
and retain collection/cleanup incomplete in their reports. The owned source
children join with exit0 without forced termination. Later idle inspection
finds no owned audio/build process; that does not relabel the incomplete
collection cleanup receipt as successful. No period rounding, Shared fallback
or larger target is substituted. These exact refusals do not establish a
physical latency floor or that all other device modes/formats are unsupported.

## App parameter replacement on the actual target

The installed application runs its real `monitor` service against an owned,
pinned Process source. Graph: `Process Loopback -> Bus -> physical render`,
48 kHz, Engine block256, Shared output with omitted native-period override.
The native Realtek buffer is1056; its actual period is480, distinct from the
input ring capacity and operating target. There is no Gain node.

The following diagnostic requests use capacity16384, target4096 and jitter96,
not a lower-latency preset or replacement for the Compatible default. Every
request is explicit; target2048's earlier refusal is left intact.

| Case | Actual result |
| --- | --- |
| `monitor5-measured4096` | One-second native baseline; per-wake selection activates in new session generation2 from generation1; exact request confirmed; Running3002ms; exit0 and cleanup complete. |
| `monitor5-window4096` | Explicit five-field policy4096/4096/96/2/10000; finite replay returns `ObservedViolation`; exit1, no active measured selection, cleanup complete. The generic receipt does not identify which individual violation occurred. |
| `monitor6-window4096-startup16384` | A separate run explicitly changes only the initial-unserved policy allowance to16384. Policy16384/4096/96/2/10000 and exact request activate in generation2; Running3000ms; exit0 and cleanup complete. |

The latter live-window selection retains99 packet events and192 read events,
with the exact policy and original source identity. Different runs have
different observed timings; the changed allowance alone is not proof of the
first run's specific failure cause. It remains a diagnostic startup policy,
not permission for steady-state loss, 3000ms audio latency or a default change.

Both successful functional runs retain144,000 captured/written frames and
zero bridge underrun, drop, discontinuity, timestamp-error and reset counters.
They also retain5632 priming frames and224 initial prime-discarded frames.
The measured delivery envelope is exceeded later: gap-bound violations are
10 in the per-wake run and3 in the window run. Prepared whole-wake demand is
1632. A baseline envelope and a finite replay do not guarantee future service.
Exit0 here means requested activation/run/cleanup completed, not zero-fault
output or qualified latency. `qualified` remains false.

Final diagnostic histories overwrite44 producer events and45/46 consumer
events during the later three-second runs. These are retained bounded-history
limits, not PCM drops or proof that the original sequence-one prefix was
overwritten before admission. No suffix is reclassified as complete history.
All owned sources stop and join with exit0; no watchdog expires.

These are native CLI/service results, not a native desktop-click acceptance
test. The existing separate GPUI desktop regressions remain software evidence;
no physical quality claim is inferred from either surface.

## Current physical-measurement blocker

A fresh read-only catalog inspection confirms the selected Realtek output is
still muted with volume scalar0. The Intel recorder is not a demonstrated
common pre-route reference plus processed-output capture. No independently
established physical boundary, calibration offset/uncertainty or synchronized
reference mapping exists. Default-role IDs match the earlier retained catalog.

To finish S1's physical acquisition, the user must identify the intended
physical output and arrange/confirm a measurement path that records the
pre-route reference and processed physical output on a common capture clock,
with calibration of channel/path offset and uncertainty. A second copy of the
same post-route stereo signal, source submission index, or unrelated microphone
does not supply that reference. The user must also authorize/perform the safe
audible-output setup if the currently muted Realtek device is intended.

Do not unmute, alter routing or invent calibration to turn these diagnostics
into a PASS. Preserve the first-packet recorder fault and earlier VAD provider
faults. Acquired valid waveform, matched route comparison and calibrated
p50/p95/p99/max remain incomplete. No VAD/Process winner or stable sub-10-ms
configuration is established; the full S1-S3 goal remains open.

## Evidence and deployment verification

Both archives are downloaded and every member is checked by exact length and
SHA-256 on the host before extraction:

| Archive | Bytes | Files / entries | SHA-256 |
| --- | ---: | ---: | --- |
| `evidence-recorder-period-diagnostics-5382369.zip` |2851553|74 /75|`27E1B6FB0C62ACD3E52F67244ABDDB0E71E4B8997E0D027534F9E24727F544CC`|
| `evidence-process-monitor-followup-e195bfe.zip` |240719|53 /54|`1662CF7E2182073379BF4291E546BAAC35963E408FA4F4DB0AEA71FDDFCE441F`|

Artifacts and executable evidence assertions are under
`target/physical-1921681297-20261029/`, including
`recorder-period-host-verification.json` and
`monitor-followup-host-verification.json`. Original PCM, packet metadata,
commands, source receipts, failures, requested/active selections and manifests
are retained. No new Rust source changes require a rebuild for this receipt.

Fresh deployed app SHA-256:
`09A0053326179928FB20E6F90DD1472501EF4A7F2D10907BEE0E9C550566F93A`;
probe: `B10E0EB92F6B7DBC4E82450F42F0E851C2F03601E7D2328BCAA386E8880C7B0F`.
The Running driver SYS remains
`CCC0EC4FE4DDB51FB22B90A483E58FF3874D87FA8634D62C93BB8AA52FFFE471`.
Remote main and detached source remain clean at497ac7c ande195bfe respectively.
Final state has no active owned build/acquisition, target bytes0 inherited
from the previous clean, C free5,198,819,328 and D free272,445,739,008 bytes.
The caches are not cleaned again and evidence/rollback/source remain intact.
