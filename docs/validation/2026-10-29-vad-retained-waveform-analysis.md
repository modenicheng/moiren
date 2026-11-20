# Retained native VAD waveform and packet analysis

Date: 2026-10-29 on the development host. Target:
`LAPTOP-NP49RG6H` / `192.168.12.97`. The preceding receipt records source
repair `f72e03c`, verified deployment, software tests and a completed bounded
native retest. This follow-up analyzes that retained PCM; collection exit 0
does not establish waveform continuity or physical latency.

No source, binary, driver, audio setting or default changes. No compilation,
audio acquisition or cache cleanup. The physical target's unchanged wall
clock reports 2026-10-25; it is not subtracted from the host clock for latency.

## Input identity and exploratory signal finding

Input is the original `vad-engine-health2-default-b128-t1024` software route:
VAD playback -> Bus -> VAD microphone, 48 kHz, native packet 480, Engine block 128,
target 1024/capacity 4800. PCM has 255,360 interleaved stereo Float32 frames /
2,042,880 bytes, SHA-256
`0CCC192CDC6E64AEE082479D18C50764034F46045F59B0BCF696B730843BC0D6`.
The original source plan has three 1024-frame bipolar markers, amplitude 0.1,
at source-frame starts 24,000/72,000/120,000. Source and recorder clocks have
no independently verified frame mapping.

Host exploratory whole-recording matched filtering finds:

| Stored recorder-frame peak | Normalized correlation, channel 0 |
| --- | --- |
| 32,813 | 0.8325606161 |
| 80,763 | 0.8515049456 |
| 128,717 | 0.7147921559 |

This search uses an explicitly exploratory 0.5 peak floor and 24,000-frame
minimum separation, not qualification thresholds. It is only used to choose
bounded observation windows. All PCM samples are finite, absolute peak is
0.1897583008 and RMS is 0.01026860119 on each channel. Channels are sample-for-
sample identical. Those two channels cannot be an independent pre-route
reference; zero interchannel offset would not measure route latency.

Neither peak-minus-source-start arithmetic nor the changing peak spacing is
reported as delay, calibrated drift, source age, loss, or hardware clock skew.

## Analyzer refuses all three windows under chosen diagnostic settings

The unchanged deployed probe runs `analyze-pcm` twice, once on PCM
alone and once with the original packet metadata. Explicit capture-coordinate
window starts are 32,301/80,251/128,205, reference search 1024 frames and processed
search 64 frames. These are post-acquisition exploratory windows, not a retained
common-reference acquisition plan.

Both calls use minimum correlation 0.98, peak margin 0.15, RMS 0.0001 and a bounded
17-hypothesis linear-template rate grid from -4000 to 4000 ppm in 500 ppm steps.
These explicit thresholds were selected for this diagnostic run, using existing
test-fixture values as guidance. They are not an established production
qualification standard for this native route. Explicit work/window/input/packet
caps are checked before analysis. Analyzer code is unchanged.

Both commands exit 1 without watchdog expiry. All three windows refuse the
reference-template confidence test, with no accepted reference/processed
alignment, no delay value and no distribution. A segmented trend is requested
but cannot fit missing accepted observations. The PCM-only schema 5 and
packet-validated schema 6 reports retain these failures. This establishes
refusal under the chosen signal model and settings, not waveform corruption
or failure of a previously qualified production threshold.

Production virtual boundaries use drift servos and prepared windowed-sinc
resamplers; `src/resample.rs` has 32 taps and 256 phases, while the existing
offline grid models a zero-extended linear template. Filtered/variable-rate
waveform mismatch is therefore a candidate explanation for the confidence
refusal, not a confirmed root cause or proof that the samples are correct.
This evidence does not justify bypassing per-section confidence checks or
lowering thresholds. An independently declared measurement signal/model and
known-distortion validation are needed before changing that experiment.

## Additional packet-policy refusal, distinct from raw faults

The 532 metadata records cover stored PCM exactly once, without missing or
reordered packet sequences. Only packet 1 has a native flag: flags 1, device
position 480, stored start 0. Later flags are 0; every later device position is
adjacent to the preceding packet. No native timestamp-error flags occur.

The analyzer additionally rejects 487 later QPC intervals under its existing
nominal schedule-rate diagnostic policy: one-frame quantization plus 1000 ppm
tolerance. Observed intervals span 95,231..105,183 units of 100 ns, median 100,009,
for 480-frame packets whose nominal interval is 100,000. The resulting invalid
packet spans overlap all three marker windows. Coverage remains complete and
all raw fields remain unchanged.

These 487 policy violations are not 487 raw discontinuity flags, timestamp-error
flags, independently established PCM gaps, or measured physical delay. The
policy remains strict and refuses the windows; no replacement hardware epoch,
timestamp, source clock or synthetic zero-fault view is introduced.

## Startup evidence remains unresolved

The retained native first-packet flag does not establish how many frames, if
any, were lost before recording. Later adjacent device positions do not prove
an intact startup prefix. No startup flag is cleared, ignored or converted
into an exact loss count; the original strict continuity verdict stays failed.
This receipt makes no upstream API exemption claim.

## Fresh deployment and physical setup audit

Read-only catalog acquisition `s1-waveform-measurement-setup` exits 0, reports no
catalog errors and starts no capture. The selected Realtek render endpoint
`{0.0.0.00000000}.{5657d4dc-1974-4bb6-9649-f3dd08c4e7ba}` remains muted true,
volume 0, 48 kHz stereo Float32. Default roles are unchanged. No independently
captured common physical reference or calibration has been supplied.

Remote main remains `497ac7c`, also the current origin/main. No application
source has changed after deployed app `e195bfe`; the latest driver source commit
remains `9cba24b` and its installed SYS hash stays unchanged. Rebuilding/reinstalling
them solely to refresh timestamps would add no missing evidence. Probe `f72e03c`
is deployed with its existing verified SHA-256. No build/audio process remains
active. Shared target retains 915,203,047 bytes; D free 271,478,222,848 bytes at
the archive checkpoint. Cache retention remains the default.

## Evidence and next boundary

Archive: `evidence-vad-waveform-analysis-f72e03c.zip`, 177,442 bytes,
SHA-256 `42FA87A008ECD6708DDC3A1053CD1DF751276D859F23B26942E21F5C8FB0A691`.
All 28 manifest-listed files / 29 entries including the manifest, exact lengths
and SHA-256 hashes are verified on the host. The archive retains original
PCM/metadata/report/source, both
diagnostic analysis outputs and commands, explicit windows/limits, exploratory
script/summary, current catalog, binary/state identities and cache policy.

The completed recording cannot supply an accepted result under the chosen
marker-confidence settings and existing timestamp diagnostic policy. Their
applicability to filtered variable-rate native audio needs independent
validation. This receipt does not propose a new marker/clock feature, repeat
the buffer sweep or select thresholds to manufacture a passing result.

S1 remains incomplete pending a usable intended output and independent common
reference/calibration, plus accepted waveform evidence. S2's native software
matrix and S3's API/CLI/desktop software coverage do not close that physical
measurement requirement. Native desktop audio interaction remains distinct
from CLI/service acceptance. No path winner, stable lower default or sub-10ms
physical output is established. The full S1-S3 goal remains incomplete.
