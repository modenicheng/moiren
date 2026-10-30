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
