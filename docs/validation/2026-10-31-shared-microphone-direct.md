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

## Explicit RAW control after the small foundation merge

Probe `3ac3f6c` adds opt-in `--raw` to the explicit Shared microphone
invocation. Only that mode sets Other/RAW, without offload, through
`IAudioClient2::SetClientProperties` before Initialize. Interface, properties
or initialization failures remain failures: no ordinary Shared, Exclusive,
format or period fallback. Both modes keep the exact owned stereo PCM16
client, 48 kHz, zero Shared duration/periodicity and QUERY PCM16/480 checks.
Production routing and policies do not change. RAW still permits endpoint
specific always-on processing; it is not proof that every processing layer
is bypassed. See Microsoft's
[AUDCLNT_STREAMOPTIONS reference](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/ne-audioclient-audclnt_streamoptions).

The CLI regression fails on the absent `--raw` argument before implementation.
All72 example tests then pass, as do host strict Clippy, formatting and diff
checks. Independent review finds no definite dispatch/shape/fallback defect,
but notes new COM failure paths lack injected integration coverage. Target
release tests/build pass with two jobs and shared caches. A local Git bundle
transfers exact committed source without triggering another CI run.

New probe SHA-256:
`734EE639C23163F4B88F8A364D652C9AD406899A96D72A9406DF71A0CB45C5B9`.
App and `e2e_latency_probe` stay at verified `0fd3fbf`; installed driver stays
`4934172`, `oem199.inf`, SYS SHA-256
`1EA1FEBCCF842DBB4904AB3BB6A897FC9E2F71680226E932BE2BDD68D686E19D`.

Two sequential two-second controls initialize with native buffer1056 and
QUERY packet480. Each submits200 packets /384000 bytes /192000 nonzero
samples, prefix `[2048, -5120, ...]`. Ordinary Shared completes179 successful
WRITEs /343680 bytes with20 drops; RAW completes177 /339840 bytes with22 drops.
Both capture199 packets /95520 frames, all SILENT, zero nonzero samples,
one discontinuity and zero timestamp errors. Both exit1 with insufficient
matching PCM. SILENT packets follow the API flag contract; zero sample counts
do not independently inspect bytes the API declares silent.

Both matching PDB/TMF decodes contain52 events, zero lost/unknown/format errors.
Ordinary first nonzero equal16-byte copy is native1/request2, QPC643533696878;
first successful notify is native0, QPC643533598200. RAW has the same ordering:
native1/request2 copy QPC643693082558; native0 notify QPC643692982003. There is
still no same-nonzero-packet notification result, query/readback or position
witness. RAW does not resolve the failure or identify a sole cause.

SSH runs in session0. A separate read-only audit finds no logged-on user,
running Audiosrv/AudioEndpointBuilder/sshd and Allow values in inspected
microphone consent stores. This is a confound, not a proven explanation.
No privacy/session policy is changed or user logged on automatically. Both
runs clean owned processes/children, preserve root PnP OK, boot, render volumes
and default roles; no physical output or cache cleanup occurs.

`raw-shared-3ac3f6c-20261031.zip`:274395 bytes,22 manifest files /23 entries,
1559153 uncompressed bytes, SHA-256
`6C5D56590EA13CE3061AB5657DB524787BDEF2009216882158B57C38035ECB68`.
Host checks remote archive identity plus every entry name, length and hash.
Host-only decodes stay outside the raw archive. Next, associate the first
nonzero packet copy with its actual notification result instead of native0.
The [ACX streaming contract](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/acx-streaming)
permits packet-vregister consumption without the capture query callback;
missing callback witnesses therefore remain unknown.

## Zero-mean AC control and explicit cleanup retention

Probe commits `a26d0d8`, `2b9bd4f` and `cdbb836` add an opt-in AC energy
diagnostic and retain cleanup failures without changing production routing.
`--ac-diagnostic` requires the explicit Shared stereo microphone mode; it
keeps exact 48 kHz PCM16/480 and zero Shared duration/periodicity. Ordinary
and explicit Other/RAW remain separate modes, with no fallback. The payload
has zero-mean 1 kHz left / 1.5 kHz right cycles in each 480-frame packet,
quantized peaks 2048/5120. It is phase-continuous repeating energy, not a
unique marker or a continuity detector. Every AC result remains unqualified,
including a hypothetical nonzero result; the constant-pattern matcher is
not used to validate it.

Independent review found expected diagnostic failure could obscure revoke,
drain and Stop failures. The two fixes retain original error context and
each cleanup error, and explicitly Stop on every AC exit, including owner
creation and pre-start setup failures. Normal modes retain their existing
cleanup behavior. CLI, cleanup and setup tests each reproduce RED before
the corresponding fix. Final host tests pass 78/78, strict Clippy, fmt and
diff checks pass, target release tests/build pass, and final independent
review closes the cleanup finding. Intermediate `2b9bd4f` was deployed but
not used for a native run.

Final `cdbb836` probe SHA-256 is
`EB71E0B3FFD530100933A1FE727C3AB3C8F8BF6022BA5D013AA9315C969E0C03`.
Driver stays `1513274`, `oem200.inf`, SYS SHA-256
`E450E9589F3775254241D851116EF145B4D00DE007195DC6924BC6623E7F610F`.
App and e2e probe stay verified `0fd3fbf`. Two sequential two-second final
controls each submit 200 packets / 384000 bytes / 182000 nonzero samples.
Ordinary completes 179 successful WRITEs / 343680 bytes with 20 drops; RAW
completes 174 / 334080 bytes with 25 drops. Each collects 199 packets /
95520 frames, all SILENT, zero nonzero samples, one discontinuity and zero
timestamp errors. Neither final failure contains a cleanup error. Both exit1
with the explicitly unqualified energy-only diagnostic, not a signal pass.
SILENT packet data is not read contrary to the API flag contract.

Both matching WPP decodes have 53 events, zero lost/unknown/format errors.
Ordinary native1/request2/index1 in generation1/gate1 has nonzero equal copy
witness QPC664548726899 and successful notify entry664548727067 /
return664548727092. RAW independently has native1/request2/index1 in
generation1/gate1, copy664595514445, entry664595514685 /
return664595514745, also STATUS_SUCCESS. Successful same-packet notification
does not demonstrate Shared consumption, mapping correctness or whole-payload
integrity. AC remaining silent rules out only the old DC shape as a sufficient
explanation, not every processing, session or driver cause.

The preceding `a26d0d8` ordinary/RAW controls are also all SILENT: respectively
188/179 successful WRITEs, 11/20 dropped, with the same 199 packets / 95520
captured frames and zero nonzero samples. Its archive
`ac-control-a26d0d8-20261031.zip` is 276654 bytes, 22 manifest files / 23
entries, 1432670 uncompressed bytes, SHA-256
`34E97CC77DAEF70056DEB0A6E3C74C86FD23476C765D39448FAB770AC88AE0EC`.
Final archive `ac-final-cdbb836-20261031.zip` is 277956 bytes, 22 manifest
files / 23 entries, 1439366 uncompressed bytes, SHA-256
`112110B351618D437EB1A44D8E054DADD2D80C30478328C8A6BB3CD601AF818E`.
Host verification checks exact entry sets, every length/hash, archive receipt
and totals. Raw target copies and caches remain; no cleanup is justified by
242281246720 free D: bytes. Both final runs remove owned processes/children,
preserve root PnP OK and boot, and change no volume, mute, role or privacy
setting. No physical output is used.

Session0 with no logged-on user remains an unproven confound. Do not repeat
equivalent headless variants or add more same-packet driver instrumentation.
The useful next comparison is the same binary/config in session0 after a real
console login, then in that interactive session. No automatic login or policy
change is authorized. If both remain silent, observe an independent client/KS
mapping boundary instead of inferring consumption from internal trace success.
