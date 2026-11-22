# Exclusive capability query and configuration scope

Recorded 2026-11-03, Asia/Shanghai. Source: `881a5ba`.

## Question and bounded operation

Does the same DAWN PRO2 render endpoint expose an alternative native period
worth considering after the current Shared configuration's whole-wake budget
refuses smaller Process targets? This is a capability query, not another audio
run, a parameter scan, or a change to the admitted delivery bound.

One remote query finishes at 07:08:31 +08:00 on `LAPTOP-NP49RG6H`, using endpoint
`{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}`. It activates a client
only to query mix format, device periods and format support. It does not call
`Initialize` or `Start`, select a new default device, change endpoint controls,
open a recorder, or launch an audio source/application. All COM objects and
format allocations are released. No unchanged native test is repeated.

## Observed capability, not negotiated operation

The query reports 48 kHz stereo, with an extensible Float32 Shared mix format.

| Observation | Value |
| --- | --- |
| Shared fundamental / minimum / maximum period | 1 / 336 / 480 frames |
| Shared default / current period | 480 / 480 frames |
| Device default period | 100000 units of 100 ns |
| Minimum Exclusive period | 30000 units of 100 ns; nominally 144 frames at 48 kHz |
| Exclusive exact Float32 mix-format support | HRESULT `0x88890008` (`-2004287480`), refused |
| Exclusive extensible PCM16 stereo / 48 kHz support | HRESULT `0`, supported |
| Stream initialized / started | false / false |

These observations do not establish that a 144-frame stream initializes,
aligns to 144, renders continuously, or admits a lower Process queue target.
Format support is separate from stream ownership and buffer negotiation. No
physical lower bound, theoretical optimum, sub-10-ms result or path winner
follows from this query.

## Scope correction

The initial suggestion that Exclusive exact-period entry is a missing original
S3 feature is withdrawn after checking the accepted settings design and current
tests. The [professional settings specification](../superpowers/specs/2026-10-18-audio-settings-design.md)
explicitly offers seven buffer presets. The later exact-period extension is
Shared-only. Exclusive's current preset restriction is deliberate, not a
reproduced production defect or an unfinished promised S3 control.

At this revision:

- `StreamSettings::validate` accepts exact Shared periods but validates Exclusive
  against `BUFFER_SIZES`; Engine blocks and ASIO retain their own preset guards.
- The desktop exact-period editor is Shared-only. Exclusive settings and device
  overrides retain the existing choices.
- The native backend already has a separate `vad-probe`-gated, output-only
  Exclusive144 exception. Ordinary app configuration and capture still reject
  it. Adding a duplicate diagnostic is not necessary.
- Native minimum, explicit alignment equality, format support and Exclusive
  full-packet rendering remain unchanged. Shared render-target policy cannot
  simply be carried into a different native mode.

A future separately approved product slice could expose explicit exact WASAPI
output periods without widening capture, ASIO, Engine blocks or defaults. It is
not implemented or added as a prerequisite here. No production code, settings,
timing, admission formula or quality threshold changes in this increment.

Exclusive initialization is not attempted on the occupied endpoint. Microsoft
documents that device policy can allow Exclusive acquisition to preempt current
Shared streams. Process identity checks cannot prove that such streams were
uninterrupted; querying support is not permission to displace protected audio.
Reference: Microsoft Learn,
[*Exclusive-Mode Streams*](https://learn.microsoft.com/en-us/windows/win32/coreaudio/exclusive-mode-streams),
paragraphs on device policy and preemption, checked 2026-11-03.

## Evidence and final safety check

Remote evidence root: `D:\MoirenTest\20261103-s3-exclusive-capability-0710`.
Host evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320\s1-s3-exclusive-capability-20261103-0710`.

The exact query script is 8729 bytes, SHA256
`4AAA95479E3BE6BCFB6728073B9BE16B5C12ED7570D5058B96E1AE0D0063811A`.
The closed remote result is 1536 bytes, SHA256
`B2BFE9720C59BEB8ED0C4AB9D4383FEA99F64B401CD802B0C24B8C5B245E082F`.
Its raw host copy, `remote-result.json`, is length/hash verified; the earlier
stdout rendering has different whitespace and is not substituted for it.

The independent read-only audit at 07:14:31 +08:00 finds no test audio owner.
Original OBS12648 and QQMusic15096 creation identities remain unchanged. Driver
state remains Running, on-disk SYS SHA256
`4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73`,
boot FileTime134377986540731770. No stream interruption test is claimed.
All27 original dirty/cache file hashes match their preserved baseline; unrelated
main-worktree audit files remain untouched. No host process is stopped.

Documentation validation checks167relative links with no broken target and
passes scoped `git diff --check`. Production paths have no committed difference
from the previously integrated `003fc3c` revision. No new Cargo run or fresh
full-workspace result is claimed for this documentation-only increment.

No native test or background soak is left running. The earlier clean-source
software integration and its six gap-bound violations/one render timeout retain
their separate [checkpoint](2026-11-03-s1-s3-development-checkpoint.md); this
query neither reruns nor supersedes them. Waveform and physical qualification
remain open, while independent development may proceed under the user's
[parallel-test scheduling instruction](../vad-low-latency-goals-2026-10-28.md#2026-11-03-development-and-test-scheduling).
