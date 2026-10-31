# Playback READ copy boundary - 2026-10-31

## Verdict and direction

The physical target now runs the explicitly enabled Playback-copy diagnostic
driver `74f11c6`, `oem202.inf` / `3.1.1.608`, and has the new application
`af70aa9` deployed side by side. These are diagnostic/deployment checkpoints,
not S1-S3, nonzero waveform, continuity or physical-latency acceptance.

The direction contract is unchanged and already merged in PR44:

| Windows virtual device | Moiren role | Other applications |
| --- | --- | --- |
| Speaker / Playback | Input / Source | Output / render |
| Microphone / MIC | Output / Sink | Input / capture |

Root `main` is freshly fetched and equals `origin/main` at `6b4ffe6`.
PR43/44/45/47 are merged foundations; the large experimental PR26 is not merged.

## Bounded implementation

Atomic source commits are `af70aa9` (isolated opt-in builds), `d9eca71`
(READ-copy diagnostic), `a56b743` (enabled/disabled host suites), `05f0beb`
(scan coverage) and `74f11c6` (portable source-controls DTO test).

`MoirenVadPlaybackCopyDiagnostic` defaults to 0 and accepts only 0/1.
`build.ps1 -PlaybackCopyDiagnostic 1 -Analyze` uses a separate output,
intermediate, scratch and log tree. Default builds compile out the added
state, scans, QPC calls and exporter. No public ABI, routing, admission,
packet status, queue policy or schedule is changed.

The enabled per-slot record is 440 bytes. The first matched Playback READ
binds its historical endpoint/session/stream/gate/format/shape; it scans at
most 480000 frames, 4800 frames per packet and 8 bytes per frame. Actual
volatile source/destination readbacks happen AFTER the original memcpy.
The source is the transfer source, not the native ring ingress and not a
before-copy witness. Source memory can change concurrently.

Counters cover scanned frames/bytes, nonzero frames/bytes on both sides,
different bytes, accepted-copy count, sequence-gap events and saturation.
First/nonzero/mismatch paired prefixes are at most 16 bytes. First/last
actual scan sequence/callback QPC/readback brackets remain available.
`all_interval_coverage=0`: these observations do not establish every interval
or source-sample alignment. The maximum added per-callback work is bounded
by 76800 volatile byte reads; actual overhead is not qualified.

Passive QUERY/gate-close export occurs outside the shared lock. At most one
interim and one sealed revision are emitted, 15 fragments per revision.
Missing fragments, abrupt teardown or slot reuse can leave unknown evidence;
there is no recovery framework or hot-path WPP.

## Host checks and CI repair

- Final MSVC run passes all 15 suites, including 54 pause-fence scenarios,
  10 enabled copy groups and the actual default-off READ path.
- Five behavioral MSBuild tests pass, including invalid-value rejection and
  actual default/opt-in compiler definitions. Default and enabled final WDK
  analysis plus INF validation succeed.
- App tests pass 551 tests with 3 ignored. The probe example passes 544 tests;
  strict example Clippy and formatting pass.
- Current remote CI for the preceding `19bef8d`, run `41149508218`, finishes
  with 19 successful jobs and one Ubuntu test failure. It is NOT green.

The Ubuntu failure is a real test-harness dead-code error: portable example
tests did not construct `Preparation` and `SourceControls`. `74f11c6` adds
an actual DTO serialization regression, without lint allowances. The Windows
example tests pass. Linux-target metadata checking with `--profile test`
and a verified rustc `--test` invocation passes. A Linux cross-link attempt
on Windows failed because the available linker was not a Linux linker; no
local Linux execution is claimed. Ordinary `cargo check --tests` was not
evidence that this example's test harness compiled.

An earlier app build hit stale shared-cache audio API artifacts. One scoped
`cargo clean -p moiren-windows-audio --profile dev` repaired that cache; the
subsequent full app test run passes. No full/repeated cache clean occurs.

## Target deployment

Target is `LAPTOP-NP49RG6H` via the pinned SSH host alias and forwarded port.
Before mutation the target is idle, old driver `6d9e32f` / `oem201.inf` /
`3.1.1.607` is Running/root PnP OK, and rollback package hashes match.
Independent review finds no blocker in the exact signed package/deployer.

The target validates existing signer trust, SYS/CAT signatures, catalog
membership, exact lengths/hashes, root instance and old binding before install.
Both install and root-device restart exit 0. The new binding is Running/PnP OK,
problem code 0, `oem202.inf` / `3.1.1.608`:

```text
SYS DFB8CC3B69EC66BA0C5CEB4982508D33504557C36C665226A07E2E956DC54C28
INF 3620B66618B628797000E7F39615B448487F687A69DAD1F88F8F32D6A84441A9
CAT 82AC4051D11C35641DFA16CADF4426F505C5B7649744C6621655D2B445AF4DC6
```

Boot remains `2026-10-30T09:50:49.659845+08:00`. No trust import, system reboot,
driver uninstall, volume/default/privacy change or rollback-package removal.
On-disk hash plus PnP binding is not independent in-memory-image attestation.

## Acquisition and retained failed attempt

The unchanged frozen source worker is `b6b26b7` (7064576 bytes,
`CE4EEDF7FD57FFB88D4B796F46CCB253312F35C26F1F3223B4680A16AF38B9D4`).
The unchanged holder is `b0a0223` (7421952 bytes,
`3D66F7518BCF57924505D9B80BF592CFF9BCE64B3D106F4AB182C558792F7ABD`).
Frozen runner SHA is
`E43481217937122F07C194A7B4C1933FCDA7707F71DE5CCC835D1C271AA87F33`.
The WPP wrapper passes 64 PS5.1 fixtures; the inherited runner passes 142.
Only an owned successfully started trace session is stopped.

The first wrapper invocation discovers that the archived predecessor runner
is absent. No trace/holder/source starts. The exact frozen runner is restored
and hash verified, with the first failed receipt preserved.

Actual run `copyboundary1` then fails before any source starts. Playback
changes from gate-open/audio-ready to gate-closed/audio-not-ready with gate
epoch 2; the holder rejects that identity/readiness change and exits 1.
Its provider capture is empty. The ETL has no diagnostic fragments, so this
attempt says nothing about copied PCM. The symptom is consistent with the
existing readiness-loss path, but an update-related or sole root cause is
not established. No timeout/readiness/identity guard is relaxed.

Fresh run `copyboundary2` uses the same binaries/fixture and finishes with
collector, holder and source exit 0, no retained errors or forced owners.
The fixture is still `Playback -> Gain0.5 -> MIC`, NOT exact S1 and not DAWN.
The source's 20-second changing schedule remains active at Stop; its observed
Start/Stop lifetime is 12382.8517 ms. No target build or physical render runs.

## Actual run2 boundary

Provider identity is stable ID `MoirenAppVadPlay`, native stamp
`[3,827427392878,3,5,13]`, reset 0, admission ticket 2, active revision 2.
PCM16 stereo has 480 frames / 1920 bytes per packet.

The source's actual Float32 destination prefix equals encoded
`CD CC CC BC` repeated four times before successful ReleaseBuffer, flags 0,
return QPC `827479967031`. Exact source controls after that Release return
HRESULT 0, session Active, session/endpoint master 1.0 and both unmuted.
That one observation is not an effective-gain measurement.

Exact matching PDB/TMF decoding retains 15 interim fragments at revision 1
and all 15 sealed fragments at revision 1000. Tracefmt reports 34 total
events, 0 EventsLost, 0 format errors and 0 unknown formats; 30 events are
the diagnostic messages. This is a decoder-reported loss count, not a
continuity or latency qualification.

The strict offline decoder requires the exact identity/revision and all
15 unique fragments, with count/seal/coverage and LE-prefix consistency
checks. Independent review reproduces two impossible inputs accepted by
the first decoder: contradictory overlapping prefixes and a nonzero-frame
imbalance exceeding differing-byte count. Both receive RED/GREEN regressions
and are rejected in the final 17-test version. Final decoder SHA is
`C1825BCC4371AAD32A6097E83D40AAD60B48FBB8AB479B2D848D176032D8FED5`.
Its exit 0 means a structurally valid sealed snapshot only; it retains
`qualified=false` and `trace_loss=unknown`. The tracefmt counters above are
separate evidence. Root and independent final decoder checks agree; a
separate 52-check review correlates actual source/provider/kernel artifacts
without promoting them to acceptance.

- Sealed identity: endpoint 3, session 827427392878, stream generation 5,
  gate 13; format 1, channels 2, packet frames 480, align 4.
- Exactly 480000 frames / 1920000 bytes are scanned; source and destination
  nonzero-byte/frame counts and different-byte count are all 0.
- Accepted-copy count 1000; first/last sequences 1/1000, sequence-gap events 0,
  saturation false, seal reason frame cap.
- First callback/readback QPC: `827480163727`,
  `827480163769..827480164183`; last: `827580067712`,
  `827580067745..827580068073`, frequency 10 MHz.
- Provider retains 1000 published packets, 480000 frames, all scalar samples
  zero, with matching first/last transfer stamps and no stale/epoch exclusions.
  All callbacks follow witnessed Release; 992 are also strictly within the
  conservative source-active observer window. No sample-origin alignment follows.

Raw provider SHA is
`124617C1F65E92D3BC895FBD869E4BB16A30754B198F59E6E973949B9AAA1B01`;
metadata SHA is
`AA056B3EC523284924604B3D0D5433029A1450C5147666E7AAFFB16039FD6E5E`.
ETL SHA is
`42E8370BC494EADF0E197BD7A6259111EB0EA3738B41B7D47D52977092E39B8B`.

The paired AFTER-copy observations do not show a nonzero transfer source
becoming zero in the READ destination. They do not establish what the source
contained before memcpy or what Windows wrote to the native ring. The
next distinct boundary is the actual consumed ring span -> timer Scratch,
immediately after copying and before ring clearing, with real packet layout
and event/timer selection recorded. No ring-layout defect is established.

## Separate app deployment and cleanup

After acquisition, the release app is separately deployed to
`D:\MoirenTest\20261031-af70aa9\bin\moiren-app.exe`, 33693184 bytes,
SHA `48DE8A4B65C74A7D8068B402BEF2732E0B112D8A617884352AA3A0A298013A4A`.
Its production source is unchanged through `74f11c6`. The manifest's mistaken
short revision `af70aa9d` is corrected to `af70aa9` before upload; its full
revision was already correct. Help and offline `--gain 0.25` smoke exit 0.
No GUI/native session starts; no shortcut or saved settings are changed.
The previous app is preserved. GUI/audio acceptance is not claimed.

Holder run2 completes its 60000 ms idle request in 60002 ms, 59 verified
windows, zero recorded idle timeline glitches/epochs. Idle progression with
zero PCM does not qualify nonzero continuity. Service join/control release
are true; public endpoint-absence confirmation remains false. Independent
final PnP audit finds no VAD children, no owned build/audio processes and no
owned trace session. Driver/boot remain unchanged after the app smoke.
D: has about 225 GiB free; no cleanup or cache eviction is needed.

## Retention and open gates

Host evidence root:
`D:\coding\moiren-evidence\20261031-playback-copy-boundary`.
Target diagnostic root:
`D:\MoirenTest\20261031-playback-copy-boundary`.
Original native runs stay under the frozen runner's `runs\copyboundary1/2`.

The target-generated manifest covers 105 payload members / 4840662 bytes;
the host verifies every length/hash against downloaded loose originals.
The archive contains those exact members plus the identical manifest:

```text
playback-copy-target-20261031.zip: 276876 bytes
ZIP SHA E80F76147928BC30ED631838BABC0E444E50DDCA39F7ABC5A5BF44E2CDD11ED4
manifest SHA A2C53D3C67BD64759E1C7D6436FE3BDDA37CE40D0DFFE525AE1AFE427F62CBD5
```

The native archive does not claim to contain the signed package, executables,
host source/test logs, PDB/TMF or later decoder work; those remain separately
retained. A second reviewer independently verifies every archive payload
against both the target manifest and its loose original, including the exact
member set and CRC. No target original or rollback artifact is deleted.

S1-S3, nonzero native waveform delivery, calibrated physical p95 <= 10 ms,
lower-buffer defaults and matched VAD/Process latency comparison remain open.
Correctness versus latency-sensitive policies are not weakened by this run.
