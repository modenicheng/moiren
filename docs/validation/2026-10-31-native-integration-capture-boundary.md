# Main integration, deployment and capture-boundary evidence

Date: 2026-10-31, Asia/Shanghai. App/probe source: `0fd3fbf`.
Initial diagnostic driver source: `c985fc3`; packet/notify follow-up: `4934172`.
Target: physical `LAPTOP-NP49RG6H`
through the pinned existing SSH host key at `frp-rug.com:14857`.

This is bounded native evidence, not completion of S1-S3, a new default,
continuous-waveform acceptance, or calibrated physical sub-10-ms output.

## Directions and main integration

Directions always use Moiren's perspective:

| Windows endpoint | Moiren graph | Other applications |
| --- | --- | --- |
| Virtual speaker / VAD Playback | Input / Source | Output / render |
| Virtual microphone / VAD MIC | Output / Sink | Input / capture |

The route is other app -> virtual speaker -> Moiren -> virtual microphone ->
other app. `ExportDirection::Playback` maps to Source and Microphone to Sink.
The explicit comments and regression tests retain that distinction.

The independently extracted, accepted foundations are merged as PR43
(`08da70f`, exact MMDevice ownership), PR44 (`2940017`, app-side directions and
safe binding), and PR45 (`a9a59db`, IOCTL operation/phase attribution). The
latency branch merges the resulting main history without replacing its live
native transport with an unavailable-activation gate. A fresh fetch confirms
`a9a59db` remains an ancestor of the deployed source. Draft PR26 is not merged
wholesale. This receipt does not relabel earlier failed CI attempts as passes.

## Build and deployment

Target release tests pass: Windows audio library 633 / 2 ignored, changing
waveform probe 532, app library 498 / 2 ignored, and `vad_native` 69. App and
both probes build successfully. The original native-example invocation omitted
the required `vad-probe` feature and exited101; its failure files remain.
Only the two native-example stages are rerun with the correct explicit feature,
without source, threshold or production-policy changes. No build overlaps
native audio acquisition. Existing shared Cargo caches and jobs2 are retained.

Exact binaries are deployed side by side under
`D:\MoirenTest\20261031-c985fc3\bin`:

| Binary | Bytes | SHA-256 |
| --- | ---: | --- |
| `moiren-app.exe` | 33552896 | `BA3618ABA2D87DB27160D9D941ED69E5D22DAF9BC1C2C97D69E944ADBC53F79C` |
| `e2e_latency_probe.exe` | 7038464 | `651B0596700791E3C0F3C68E35C52D2D9F0432196F50618C5E9FD8E445D1B94E` |
| `vad_native.exe` | 551424 | `FED8891CFC344D0EC2ECA657992B35CC13F5E05B007A0C29A86F9828FF9C63F0` |

The diagnostic driver passes all11 MSVC host suites, analyzed WDK build, INF
validation and catalog generation. The existing WDK test certificate signs it;
no private key is exported. Host Authenticode verification initially reports an
untrusted root; that failure is retained, not fixed by adding host trust. The
target already trusts the pinned certificate and verifies SYS/CAT signatures,
catalog membership and all package hashes before installation.

`pnputil /add-driver ... /install` and a root-device restart both return0.
`ROOT\MEDIA\0001` is PnP OK, service Running, `oem184.inf`, version
`2.15.56.622`. The service image resolves to the new DriverStore package and
has SHA-256 `14BA469913DC7853791D95FF53B54C1AFCBA78A71AC577C4A4C15CC9D432505F`.
Old `oem147.inf`/SYS and rollback binaries are retained. The actual new capture
diagnostic messages below additionally demonstrate execution of the new code;
a service-path hash alone is not a memory-image attestation.

Boot remains `2026-10-30T09:50:49.659845+08:00`. No reboot, speaker-volume,
mute or default-role change is requested. Audible Process runs use only the
authorized pinned DAWN headphones.

## Shared microphone control: actual native copy observed

The first new-driver attempt fails before native admission with
`0x88890004`; no payload is submitted. Remote `tracefmt` then attempts to find
the host-only PDB and interrupts the runner's final receipt. The original ETL,
stderr and running receipt survive. A separate read-only recovery audit confirms
no owned process/child, unchanged boot, render settings and defaults. The failed
runner is not treated as a successful control or silently overwritten.

A separately named second run repeats the same strict stereo PCM16/480 Shared
control for two seconds, with no format/mode fallback. A unique owned WPP
session starts and stops successfully. Host decoding with matching symbols/TMF
reports51events, zero lost, unknown or formatting-error events.

- Actual encoded submissions:200 packets,384000bytes,192000nonzero samples,
  peak5120, prefix `[2048,-5120,...]`.
- Terminal WRITEs:176 successful /337920bytes and23 dropped; the last request
  is retired during cleanup. Native capture:199 packets /95520frames, all
  SILENT, zero nonzero samples, one discontinuity, zero timestamp errors.
- Both first-copy and first-nonzero diagnostic fragments report the same
  generation1/gate1/native1/request2, packetCount2, packetBytes1920,
  nativeBytes1920, ringOffset0 and packetOffset0.
- The actual post-copy16-byte readback reports source_nonzero1,
  destination_nonzero1 and equal1. It proves this sampled native-buffer prefix
  is nonzero after successful copying, not the whole ring or downstream output.
- No capture-position witness is emitted. This means not observed under its
  same-generation/stream/first-copy-gate condition, not a zero position or proof
  that ACX never called the callback.

The process exits1 for insufficient matching PCM, without timeout. All owned
children retire, only the root remains, boot/render settings/defaults are
unchanged. The next unobserved boundary is the actual returned capture packet
and notification result; this evidence does not establish an APO/privacy,
format, indexing or sole-driver root cause. It also does not make the constant
pattern control an exact IO/Bus or physical latency measurement.

## Exact software route and lower VAD residency

Both runs use only VAD speaker input -> unity Bus -> VAD microphone output at
48kHz, Engine32, capacity4800, jitter96 and declared packet/gap480. The source
client actually negotiates exclusive Float32/144; the recorder is exclusive
Float32/480. Correctness mode retains faults rather than stopping at the first
one, with3000ms diagnostic jitter tolerance. Changing-marker source submission
and bounded PCM recording run for about30seconds and clean up fully.

| Candidate | Modeled minimum | Native MIC drop/silence | Clock clamp/reset | Recorded frames | Exploratory peaks |
| --- | ---: | --- | --- | ---: | --- |
| target1024 / recovery8 | 865 | 1 / 2 | 0 / 0 | 1456320 | 19 / 29 |
| target768 / recovery4 | 737 | 0 / 0 | 0 / 0 | 1455840 | 23 / 29 |

The lower candidate is actually admitted with low-water161, prime/servo768
and the unchanged capacity4800; recovery4 bounds a128-frame complete Engine
burst. Both provider snapshots retain zero playback drops/silence. The lower
run reports `quality_passed:true` but `quality_complete:false`; neither its
limited counter result nor exit0 establishes continuous waveform quality.
The two settings differ in target and recovery policy, so this is feasibility
evidence for complete configurations, not an isolated causal target comparison.
The lower run's microphone maximum completion interval is106527 in100ns units,
longer than its declared480-frame gap; that assumption is not qualified.

Recorded samples are finite, nonzero and duplicated stereo, not independent
reference channels. Retrospective single-channel template matching preserves
the previous exploratory0.7 threshold and reports only19/23global peaks out of
29expected. These are not uniquely identified source markers or continuity
acceptance. The old analysis script assumed its first above-threshold peak was
the first source marker and raised an empty-window error; a separate host-only
copy retains out-of-range windows explicitly without lowering the threshold.
No software or physical delay is inferred from these onsets.

### Bounded marker audit and one-minute repeat

A separate host-only audit preserves all four original inputs and the0.7 raw
template threshold. It reproduces23global peaks, but finds29nonzero clusters
and one cluster in each of29inferred schedule intervals. Assigning the first
nonzero frame32140 to the first scheduled marker is an explicit assumption,
not an independently identified source origin. Under that alignment the first
strong peak belongs to inferred slot5, explaining the previous five-interval
window-label shift. Original reports are retained unchanged.

The six weak intervals have raw correlations0.5733-0.6922. A bounded linear
rate/fractional-phase model search yields0.9454-0.9522; this supports template
rate/phase mismatch as an explanation, not its sole cause or a waveform-damage
exclusion. All3033 packet flags are zero and packet sequence, stored offsets
and device positions are contiguous. Neither that metadata nor repeated-template
matching establishes waveform continuity, physical latency or qualification.
Derived audit/replay outputs and input hashes are retained separately in
`vad-software-target768-cap4/host-marker-audit`; raw captures are not rewritten.

The next complete-config repeat retains target768, capacity4800, recovery4,
Engine32 and the3000ms correctness-mode jitter tolerance. Its source schedule
contains2880000frames and59markers; source completion ends the observation
after60.2225232seconds, rather than running the90-second watchdog budget.
The owned process exits0 after61.4260384seconds. The recorder retains2896320
frames in6034complete packets, no missing/incomplete packets, sequence gaps or
nonfinite samples. Collection and cleanup complete; `quality_passed:true`
still accompanies `quality_complete:false` and `qualified:false`.

Both pre-cleanup provider snapshots retain zero dropped/silent packets,
sequence gaps and epoch resets. Engine clock clamp/reset counters are zero.
The microphone maximum completion interval is106390 in100ns units, again
above the declared480-frame gap; the delivery envelope is not qualified.
Cleanup snapshots separately record the expected gate/Removed transition
with one epoch reset/recovery each; they are not presented as zero throughout
the full lifecycle. No marker audit or continuity acceptance is claimed for
this new one-minute capture.

The raw repeat archive `vad-target768-cap4-repeat2-20261031.zip` is720614bytes,
16manifest files /17ZIP entries,38162466uncompressed bytes, SHA-256
`F7D0F7901760E983F86C14DA844F6AFD001B46ED0388590E2C1DD0FC4B440765`.
The host verifies exact names, every file length and hash before extraction.
The first archive attempt fails before ZIP creation because Windows PowerShell
has not loaded `System.IO.Compression`; its original error and manifest are
retained, then a versioned manifest retry loads the assembly explicitly. No
remote raw files or caches are deleted. The read-only target audit finds no
owned build/audio processes and the same boot; archive completion leaves
242294849536bytes free on D:. No new audio run or setting write is performed
during this archive/audit step.

## Actual app Process replacement and safe refusal

The real `moiren-app monitor` route is Process Loopback -> Bus -> pinned DAWN.
Engine256, Shared480-frame native output period/capacity1056, capacity16384
and jitter96 stay fixed. The one-second baseline measures actual source
delivery and requests a replacement owner. Baseline complete consumer demand
is1632frames, not the256-frame Engine block.

| Target | Measured max gap | Confirmed generation | Running ms | Later gap violations |
| --- | ---: | ---: | ---: | ---: |
| 4096 | 545 | 2 | 3002 | 1 |
| 2048 | retained failed admission | none | 0 | not applicable |
| 2560 | 538 | 2 | 10002 | 9 |

The2048 request exits1 with `input target cannot cover render demand, capture
gap, jitter and lookahead`, without silently raising the target or falling back.
The2560 request confirms exactly prime/servo2560 with independent capacity16384
and modeled minimum2268. Both successful runs finish their requested duration
and cleanup, leaving source/monitor exit0 and no owned process.

Latest live bridge snapshots retain zero dropped/silent/underrun frames,
discontinuities and resets, but the later delivery-envelope violations remain.
For2560, the retained live snapshot has433920captured/written frames and
431360output frames; it is not a final joined-owner tally. No continuous PCM
recording or independent physical reference is collected. This verifies current
CLI/service replacement after main integration, not new visual/desktop
acceptance or a Process/VAD latency winner.

### Smaller Engine block follow-up

A separate actual `monitor --engine-block-frames32` request refuses before
opening an audio owner: the app currently permits64/128/256/512/1024/2048/4096
blocks. The probe's Engine32 capability must not be substituted for this
production-entry constraint. Its error and cleanup receipt are retained; no
validator is relaxed to admit it.

The next request uses the accepted Engine64 with a fresh three-second Compatible
baseline, keeping Shared output/capacity1056, capacity16384 and jitter96 fixed.
Measured max packet480/gap559 and actual complete demand1248 admit target2048
with low-water1250 and modeled minimum1905. Generation2 confirms exactly
prime/servo2048, runs10001ms and cleans up, source/monitor exit0. The latest
live snapshot retains433920captured/written frames,431488output frames and
zero dropped/silent/underrun frames, discontinuities or resets, but two later
gap violations and maximum116866 in100ns units. This is an actual lower
configuration, not a qualified delivery envelope, physical latency measurement
or isolated target comparison. No native-period/format fallback experiment is
added to this checkpoint.

The follow-up raw archive `process-engine64-20261031.zip` is67500bytes,
35manifest files /36ZIP entries, SHA-256
`D71F0BC966D8E01D079479C73D8A832F99545AA3E8160CDA5EAD3268A6C930E5`.
The host verifies exact names, all lengths and hashes. Boot and new driver hash
stay unchanged, only the root is present and no owned process remains;
242333863936bytes remain free on D:. No remote files/cache are removed.

## Archive and next boundary

Raw build/deployment/run evidence is retained on target and copied into
`native-integration-20261031.zip`:15989545bytes,160manifest files /161ZIP
entries,96057724uncompressed manifest bytes, SHA-256
`30930C7AC2B39AB673659A77ECB03F18AEADEB40A38533AC633AD8E532CA483E`.
The host verifies the archive hash, exact entry names, every file length and
SHA-256 before extraction. Host-derived analysis remains outside the raw ZIP.
The archive script fails only while summing OrderedDictionary sizes after ZIP
creation; a separately named recovery receipt records the existing archive
without recreating or overwriting it.

The final target audit retains the unchanged boot, clean exact source, new SYS
Running, root PnP OK and no dynamic children or owned build/audio process.
D: has242448465920bytes free. No remote evidence or Cargo cache is deleted.

Next work is bounded GetCapturePacket/Notify observation, not speculative audio
policy changes. S1 independent calibrated reference and waveform acceptance,
S2 sustained lower-budget delivery, and complete S1-S3 acceptance remain open.
No default is promoted and draft PR26 remains unmerged.

### Independent bounded-name foundation PR

The delegated extraction worktree produces PR47 from `a9a59db`, independently
reviewed at final head `792a3e2`:7driver/test files,+232/-5. It extracts only
the bounded UTF-16 `StageInterface` name-size contract and invalid-name
rejection, not typed publication/readback, name-derived endpoint identity,
diagnostic experiments or latency policies. Baseline and empty-name mutation
both fail the new regression as expected. All7 MSVC/portable suites pass;
the parent separately passes analyzed WDK/InfVerif. Actual Ubuntu portable
CI passes, then all20 final-head checks complete successfully. PR47 merges
as `6b4ffe6` at2026-10-31T03:24:36+08:00, and clean local main fast-forwards.

The experiment branch merges that main history in `31b6f4c`. The expanded
native interface-policy fixture is retained unchanged except its generated
include name and moved to `interface_name_native.cpp`; the portable bounds
fixture stays distinct and extracts the real merged `StageInterface`. Its
typed-property boundary is explicitly stubbed, while the native fixture still
checks actual typed-publication bodies. All12 MSVC suites,7portable suites,
analyzed WDK and InfVerif pass; disabling empty-name rejection reproduces a
portable exit1, then restored head passes. No full-policy/native diagnostic
coverage is dropped to resolve the merge. Test-only adaptations do not confer
native name propagation or hardware acceptance on the experimental policies.

## Packet/notification follow-up deployment

The next diagnostic slice `4934172` retains fixed first successful/nonzero/refused
capture queries and actual successful/failed notification status. Readback stays
bounded to16bytes; the record is480bytes/endpoint, capped at512. Existing locks
protect detached witnesses and only passive QUERY emits trace output. No public
ABI, sequence progression, notification order, audio policy or default changes.
Parent independently repeats all11 MSVC host suites and WDK `-Analyze`/InfVerif
successfully. Host tests stub DDIs and do not prove hardware delivery.

The package is signed with the same pinned certificate; the physical target
verifies SYS/CAT signatures, both SYS and INF catalog membership, and package
hashes before installation. Root installation/restart return0 without reboot.
`oem199.inf`, version `3.1.1.605`, service Running/root PnP OK and SYS SHA-256
`1EA1FEBCCF842DBB4904AB3BB6A897FC9E2F71680226E932BE2BDD68D686E19D`
are verified. The prior packages remain available for rollback. App/probes stay
at the verified `0fd3fbf` binaries; no Rust rebuild or app replacement is needed
for this driver-only observation. The initial long encoded SSH invocation is
rejected before execution by the command-length limit; the unchanged script is
then uploaded and invoked by file. Both original error and successful receipt
are retained.

Two independently named strict Shared microphone controls run with the same
`--exercise --shared-microphone --intent=microphone --seconds=2` request. The
first exits1 with0x88890004 before buffer/submission; its trace decodes64events.
The next reaches PCM16 stereo480 with native capacity1056, submits200packets /
384000bytes /192000nonzero samples and completes181successful WRITEs /
347520bytes with18dropped completions. Shared capture again returns199packets /
95520frames, all SILENT, zero nonzero samples, one discontinuity and no timestamp
errors. This remains an actual failure, not a tolerated quality pass.

New notification evidence reports `STATUS_SUCCESS` for generation1/gate1,
native packet0, completion QPC625695804969. The first nonzero equal16-byte
source/destination copy is native1/request2 at QPC625695902474 in the same
generation/gate. The first successful notify therefore precedes the nonzero
copy; it does not prove notification of that payload. No query/readback,
position or failed-notify witness is observed. Missing callback evidence is
unknown, not proof of absent ACX delivery. The bounded trace cannot assign a
sole driver, client, privacy or downstream-policy cause.

Matching host PDB/TMF decode reports52events, zero lost/unknown/format errors
for the second run. The actual new notify message additionally demonstrates
execution of the new diagnostic code. Both runs finish owned-process/child
cleanup, retain root PnP OK and unchanged boot/render-volume/default settings.
No physical output, speaker setting change, cache cleanup or reboot occurs.

The raw target deployment and both runs are archived in
`packet-notify-4934172-20261031.zip`:89568bytes,34manifest files /35ZIP entries,
1146045uncompressed bytes, SHA-256
`8C01923C8959DCCFD8B0A8D212D6A4E0A76156E0A09DECF05DC36AC4972AB350`.
Host verification checks exact names, lengths and hashes before extraction;
host-only decoded traces stay outside the raw archive. No remote evidence is
deleted; D: retains242293538816free bytes. S1-S3 and physical latency gates
remain open. Non-latency PR extraction continues separately from this experiment.
