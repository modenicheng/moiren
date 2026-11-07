# Production VAD app output on DAWN

The physical target now retains separate source and production-app PCM for
`VAD playback -> Input -> unity Bus -> DAWN PRO2`. This closes the missing
VAD app-output acquisition in the [DAWN receipt](2026-10-30-dawn-native-process.md).
It does not establish physical latency, complete waveform continuity, a
VAD/Process winner, lower-buffer qualification or S1-S3 completion.

## Identity and configuration

- Target: `LAPTOP-NP49RG6H`, SSH `192.168.12.97`, interactive session1.
- Production app3ac8484, SHA-256
  `7F4EA0148D09DFCFBC316A683A54ACC2C6B5EC4F70FB3B9B296DE63BBB211513`.
  Driver and app are unchanged by this acquisition.
- Source81eb641 with confirmed Audio MMCSS; recorder9313364. The source is
  a sibling of the app, so recording the app tree excludes the source render
  stream. Recording the source tree independently retains its submitted PCM.
- Project version2; exactly three nodes, unity Bus/edges, 48kHz stereo F32,
  Engine block256. One enabled playback export binds source node0; there is
  no microphone export. No measured Process profile or smaller VAD policy
  is applied. This is not matched to Process's earlier4096-frame target.
- Physical sink is explicitly pinned to
  `{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}` / DAWN PRO2.
  The source renders only to the owned VAD playback endpoint.
- Project stable bytes `7b86fca1b41549c9aaa61871896516a2` resolve to PnP
  suffix `A21665897118A6AAC94915B4A1FC867B`, raw adapter
  `MOIREN\VAD_RAW\1&14041d6&0&A21665897118A6AAC94915B4A1FC867B`, parent
  `ROOT\MEDIA\0001`, and render MMDevice
  `{0.0.0.00000000}.{F6B1BC91-2D61-49C1-ACF7-0A990ADF330C}`.
  Identity/direction/ancestry are checked; the generic FriendlyName is not
  proof that the requested export name propagated.

The marker uses the existing1024-frame xorshift32 template at amplitude0.025,
one marker per48,000 source frames, identical stereo channels. The300-second
schedule is deliberately stopped after the bounded acquisition; this run
does not submit the entire schedule. Correctness/3000ms scheduling grace
does not enlarge the audio buffer or represent latency.

## Retained attempt and completed acquisition

`desktop-dawn-vad-pcm1` waits for an export before pressing Start and times
out without a source or recording. `apply()` updates export-service
availability; the harness had assumed idle publication too early. Its
WM_CLOSE fallback does not exit the tray app and the exact owned app is
forcibly terminated after the deadline. This remains a failed attempt, with
unchanged output state; it is not a native playback failure or a clean exit.

`desktop-dawn-vad-pcm2` invokes Start before resolving the owned endpoint,
then starts the source and records both trees for35 seconds. There is no
build during acquisition. A read-only PnP inventory overlaps this window;
this is not a controlled zero-load experiment. Hidden UIA snapshots verify
actions only and are not screenshots or visual acceptance evidence.

| Recording | Native elapsed seconds | Stored PCM | Marker result |
| --- | --- | --- | --- |
| Source tree, VAD render | 35.0008991 | 1,679,040 frames / 13,432,320 bytes | 35 markers; all34 intervals48,000 frames |
| App tree, DAWN render | 35.00141 | 1,679,040 frames / 13,432,320 bytes | 35 markers; intervals47,989..47,997 frames |

Both capture owners Stop/join normally and report zero artifact dropped
frames, overflow, discontinuity and timestamp-error packets. Each retains
one timeout wake. Process position-gap detection is inapplicable. The
deliberately unconsumed compatibility bridge drops its copy of every frame;
these counters are separate from the persisted artifact and do not describe
loss in the app's VAD bridge.

The detector uses absolute normalized correlation >=0.5 with a24,000-frame
minimum peak distance. Its exact source/app PCM SHA-256 values are:

- Source: `D27AB7166800D6EA306392C3EE4E982324CB3AC60D5DA8BA65B6112F834B1371`.
- App: `EA83D66DD34C82FD4EB8FE3C8F3A8072DC41BDA02176892B9C2F0C18F4C0E8DE`.

No whole marker is absent in this window. The app's first-to-last marker
span is213 frames shorter across34 intervals; its peak0.04750000685 and
maximum channel difference0.00008793175 also differ from the source. These
retained waveform differences are not silently relabeled as bit-exact
continuity or assigned a cause. Independent recorder origins and repeated
identical markers cannot establish absolute source-to-output delay.

The source acknowledges1,728,480 scheduled frames in36.0104073seconds,
with1056 native preroll frames, zero event timeouts/failure, successful
native Stop and joined worker. This is ReleaseBuffer submission evidence.
The app's source is stopped and reaped before native Stop; Engine owner1
cleanup/reap and Settings/Diagnostics/Quit complete with app exit0. Both
recorders exit1 as designed: acquisition succeeds, qualification remainsfalse.
This recorder does not acquire the production VAD provider's full quality
counters, so it cannot certify zero graph xruns, resets or recoveries.

## Retention and final state

Both attempts, exact scripts, native reports, project/preferences and binary
manifests are archived as `dawn-vad-app-pcm.zip`: 521,769bytes, SHA-256
`12EB23CD986F4280617F704E09F01C15CB8BEC6018DE4F28430290D7E90BFC10`.
All53 manifest files /54 entries are downloaded and verified by exact file
set, length and SHA-256 under
`target/physical-1921681297-20261029/dawn-vad-app-pcm`.
Source/app analysis summaries and the final audit are retained alongside it.

At04:08:18 on2026-10-30, both exact terminal tasks are unregistered and no
Cargo/rustc/link/app/probe process remains. MoirenVad is Running. All output
volume/mute states and default roles match the pre-run catalog. Realtek is
still muted at volume0; DAWN is unmuted at volume1. There are zero volume,
mute, default-role or clock-setting writes. No unknown process is stopped.
Reusable Cargo caches remain; D has254,991,351,808bytes free. No cargo clean,
driver install, reboot or security change occurs.

DAWN supplies an authorized physical render destination. The headphones and
software loopback still supply no independent common-clock physical reference
or calibrated capture boundary. Physical p95 <=10ms remains unmeasured.
