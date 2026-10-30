# Shared microphone direct-WRITE diagnostic

Date:2026-10-31, Asia/Shanghai. This bounded S1 localization experiment is
separate from the exact IO -> Bus -> IO acquisition and physical latency
qualification. It is not a replacement route or a new production preset.

## Question and controlled boundary

The retained full virtual route has all-zero Shared microphone recordings,
while an Exclusive recorder acquires nonzero PCM. Those recordings also differ
in driver-side format: Shared PCM16/480 versus Exclusive Float32/480. Therefore
they do not isolate native sharing mode or prove a driver-format defect.

The next control bypasses Engine, Bus and provider carrier conversion. An
explicit `vad_native --exercise --shared-microphone --intent=microphone`
invocation creates only one owned stereo microphone, submits a known nonzero
PCM16 packet through the existing `PcmOwner`, and samples its Shared capture
client. It requires the actual QUERY shape to remain stereo PCM16/480 and
refuses a mismatch instead of selecting another format, period or mode.
Default exclusive-event and explicit exclusive-timer probes remain separate.
Help and ordinary portable tests must never contact the installed driver.

Shared capture initializes an event callback with duration/periodicity0/0,
then uses the real client buffer size for bounded storage. `GetNextPacketSize`,
`GetBuffer` and `ReleaseBuffer` stay on the same owner thread; one packet per
poll bounds collection. The probe never changes endpoint defaults, volumes,
mute state or physical-device selection. Native acquisition occurs only on
the identified target after builds exit, never on the development host.

## Evidence and interpretation

Retain the exact QUERY shape, actual encoded submitted bytes, nonzero-sample
count, peak and bounded sample prefix; successful/dropped terminal WRITE
counts and exact bytes; capture packet flags, SILENT/discontinuity/timestamp
counts, nonzero/nonfinite sample counts, peak and bounded sample prefix.
Counters survive failure in the existing diagnostic stderr receipt as well
as successful reports. The existing close-gate, drain, Stop and Removed
cleanup path remains authoritative. A no-signal result is a retained failure,
not permission to retry a different format silently.

- Nonzero submitted PCM and successful terminal WRITEs with zero Shared samples
  narrow the next investigation to the post-WRITE Shared delivery boundary;
  they do not prove audibility, native-ring integrity or sole driver cause.
- Nonzero Shared samples demonstrate that direct injection is possible in
  that configuration. Actual provider encoded payload remains the next
  unobserved boundary; the control alone does not blame Engine or source.
- Dropped/failed WRITEs or unexpected QUERY shape leave the intended control
  incomplete. A successful process exit alone is not a stability pass.

The legacy signal-pattern check permits startup silence and is a functional
threshold, not continuous waveform quality. No result from this direct,
constant-pattern control qualifies changing-marker continuity, a matched
VAD/Process comparison, physical end-to-end latency or a smaller default.

## Retained first native result

The physical target `LAPTOP-NP49RG6H` ran source `cf4aeb7` through
`frp-rug.com:14857`, with the existing driver unchanged. The release example
passed all 69 portable tests before deployment; host strict example Clippy,
formatting and diff checks passed. No build overlapped native acquisition.

`resume-vad-native-shared-direct1` used the explicit two-second Shared
microphone invocation. QUERY confirmed stereo PCM16/480 and the native client
reported a 1,056-frame buffer. Actual encoded submissions contained 200
packets, 384,000 bytes and 192,000 nonzero samples, with the retained prefix
`[2048, -5120, ...]` and absolute peak 5,120. There were 184 successful terminal
WRITEs carrying 353,280 bytes and 15 dropped completions; the final outstanding
request was retired during cleanup. QUERY also reported 15 dropped and 15
silence packets.

The Shared client collected 199 packets / 95,520 frames, all flagged SILENT,
with zero nonzero samples, zero peak, one discontinuity and no timestamp-error
packets. The probe exited 1 with `insufficient matching PCM: 0 signal frames,
95520 silent frames, need 48000`. This is a failed control, not a stability
pass. Nonzero submitted payload and successful terminal requests narrow the
next investigation to the post-WRITE delivery boundary, but neither establishes
native-ring contents nor proves a sole driver cause.

The terminal receipt confirms no timeout, no remaining owned processes, only
the original root VAD device present, unchanged boot, zero volume/default-role
writes and no physical output. No driver or speaker setting was changed.
The raw failure stays on stderr; host-derived extraction is kept separately
and is not substituted for the original log.

The build, deployment, run, scripts and final read-only audit are archived as
`native-shared-direct-20261031.zip` (24,077 bytes, 21 manifest files / 22 ZIP
entries; SHA-256
`F2DCF38B9126685CC33DB480B093E1BF59A298E50FA45D9075D79F4E09464984`).
The host copy was checked against the remote archive hash, exact entry names,
and every manifest length and SHA-256. Neither target copies nor Cargo caches
were deleted. Further native experiments are deferred while independently
reviewable, accepted non-latency changes are extracted from draft PR26.
