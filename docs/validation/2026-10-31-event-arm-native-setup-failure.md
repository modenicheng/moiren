# Event-arm diagnostic native setup failure - 2026-10-31

## Outcome and scope

**One failed native attempt; no event-arm/rebase association was acquired.**
The new diagnostic driver is installed on the declared target, but the
unchanged frozen9968505 probe fails independent microphone preparation at
`SetEventHandle: HRESULT 0x88890004`, before recorder Start. Do not replay this
attempt, count empty counters as fault-free delivery, or infer a scheduling
fix, stable floor, impossibility result or physical latency.

Host evidence: `D:\coding\moiren-evidence\20261031-event-arm-boundary`.
Target evidence: `D:\MoirenTest\20261031-event-arm-boundary` on
`LAPTOP-NP49RG6H`. Run `event-arm-boundary1`; exact task
`Moiren-EventArmBoundary-waveform1`; owned trace
`MoirenVAD-20261031-EventArmBoundary1`.

## Frozen experiment and deployment

The [preceding-arm diagnostic](2026-10-31-event-timer-arm-boundary.md), source
`48ecf086941da9ba7d6c90abe48d2cdc2c1c3bee`, is packaged as3.1.1.611.
The package-only INF version/date changes do not modify the tracked INF.
The exact frozen9968505 probe remains7437312 bytes, SHA256
`69BB890BDBEADF2F23B994EA70AFDAC225208BD47455CB63746CA83464EB0C77`.
Its revision is not48ecf08 and it does not include the later0ef5a29 final
retired-query accounting change.

Inputs and native argv are independently byte-compared with trace2:48kHz,
Playback Exclusive480 request, MIC-only Exclusive queried minimum, unity
Bus/Engine32, target1024/capacity4800/jitter96, maximum packet/gap480,
recovery8, indexed-v2 source29 identities/30s, correctness grace3000ms and
60s requested ceiling. No physical renderer is selected. MIC144 is an
intended request here, not an observed negotiated buffer or delivered period.

Frozen SHA256 pins:

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| Signed SYS | 177048 | `4B6F3A1A572EF36886A26EF5687797280F51AF6F7B29DEC2697A696AB06F5B73` |
| Packaged INF | 1799 | `8EA7802F8D0EB87D21105D59D60EEC728E4F970943F3B3CE32A306CA0DDA8A22` |
| Signed CAT | 2484 | `98ABF70C0D5E612CCD3CD05A47A7B0A4DA62B98F4D63841CCA2217ED03C625A7` |
| Matching PDB | 1380352 | `3BDB9388FC3F0E0F231BCC3013E872AFDBBAEF69559933BE3EBF21D3EF0F8647` |
| Package manifest | 1716 | `53473D1C68FF32D3E44D1DE21157C733F97C27E12EA1A35C02A5A1FC22E9E53A` |

Root verifies all package/PDB pins and the matching dispatch TMF's13
field/descriptor pairs, including signed `due_hns`. Repackaging reruns17
driver host suites, default-off and opt-in WDK `/analyze`, INF verification,
signing and TracePDB. Earlier unsigned build hashes are historical, not the
authority for this signed package.

The adapted deployment helper passes131 actual PS5.1 inert helper/state-machine
checks, including five new integral schema guards. The acquisition helpers
pass48 runner,21 WPP,8 actual Invoke/Read/Wait and54 archive checks. The
integration fixture initially fails because its body marker assumes LF;
normalizing only the fixture source's CRLF and refusing a missing marker
restores8/8. The operational wrapper retains the already-fixed explicit
script-local function capture before `GetNewClosure()`.

Target deployment runs once,13:51:34.9111308-13:51:44.3053580UTC. Existing
target trust and all eight candidate/rollback signature/member commands pass.
One install and one exact root restart return0; binding changes
610/oem204 to611/oem205, service Running, root `ROOT\MEDIA\0001` OK/problem0.
Before/after boot is unchanged at `2026-10-30T09:50:49.6598450+08:00`.
All ten owned tool handles retain explicit exit0; no automatic rollback,
retry, reboot, trust edit or package removal occurs. Rollback610 remains
preserved. Installed service-file/PnP identity is not in-memory attestation.
Six default roles and twelve render master volume/mute controls are unchanged.

## Actual failed run

The trace starts13:52:36.7126839UTC; the one Interactive/Limited task request
remains observable until delayed execution. Root observes that same task,
never launches a replacement. Probe PID5608 starts in the intended session;
its launch receipt is13:54:42.0171467UTC and final receipt
13:54:50.3654448UTC. Probe, controller/task and wrapper each exit1.
The exact owned trace stop returns0 at13:54:51.8463153UTC.

`capture/collection.json` retains the earliest native failure:

- Query default100000/minimum30000hns returns0; initial/final request30000hns.
- First Initialize returns0; no alignment retry is entered.
- SetEventHandle returns0x88890004. Actual buffer query/result stays null;
  no buffer, sample rate, packet shape or native Start is reported as observed.
- Recorder packets/wakes are0 because the stream never starts. Source outcome,
  admitted generation and Engine/native summary are unavailable, not zero loss.
- Collection and source submission are incomplete; quality is false/incomplete.
  Cleanup is complete with no cleanup failure; worker join succeeds and both
  owned endpoints reach Removed. This is distinct from trace2's failed cleanup.

The local SDK names0x88890004 `AUDCLNT_E_DEVICE_INVALIDATED`.
Microsoft's SetEventHandle contract lists endpoint/resource reconfiguration,
removal or unavailability as possible invalidation conditions; it does not
identify which caused this attempt. Official reference:
`https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-seteventhandle`.
Do not replace the retained error with cancellation or automatically retry it.

Read-only Kernel-PnP Configuration events in13:54:40-13:54:51UTC record16
events: both raw children configure to611/oem205, each has event411 with
problem status0xC00000E5, then prior MMDevice instances are deleted and new
instances configured/started. The selected MIC is
`{0.0.1.00000000}.{52756e87-c06f-401c-be2d-a6f6bc01f9cf}`.
WPP independently shows successful repeated circuit Prepare/publish within
the same run. These observations support a post-install lifecycle/rebinding
hypothesis, not proof of the precise SetEventHandle cause. There are no
per-COM QPC spans or notification-disconnect cause in the frozen probe.
Read-only System/Application/UMDF queries return no matching entries; that
is not proof of absent faults elsewhere or whole-interval log completeness.

## Archive, decode and cleanup

The42-payload archive plus manifest is89610 bytes, SHA256
`6C305869722628F60008CA3CCC9341419CC1C8C5CF43DD537821C3E48103C619`.
Manifest11323 bytes, SHA256
`8832BA871EEC4E0BA815BED707F937435F23EF375AB4399DDB0C20C5A8B5FB77`.
The73728-byte closed ETL SHA256 is
`9AC56EE0FEBE862727BBA37042264791C35471B2200EB3C68560F866D5288F53`.
Target archive pins, root trusted-helper/provenance validation and independent
post-cleanup ZIP/extracted42-payload byte/hash recheck all pass. No raw file,
failed verdict, cache, rollback or package is deleted.

Matching611 TMFs decode116 events with reported EventsLost/format errors/
unknowns all0. Raw trace-loss availability remains unknown. There is no
Clock rebase/dispatch or Capture no-copy witness; schema2 correctly refuses
`missing rebase/no-copy witnesses` and writes no successful association.
`root-boundary-decode/association-refusal.json` records that failure explicitly.
No callback timing, late rearm or scheduler cause can be derived from this run.

The exact-task cleanup helper passes50 guard and4 real extracted-flow PS5.1
fixtures. Its first read-only audit refuses a transient busy/PID guard; no
mutation occurs. A subsequent bounded process inventory is empty and a fresh
read-only audit passes. Only then does the guarded one-shot cleanup unregister
the exact unchanged terminal task. Final14:00:19.2518970UTC audit has no task,
owned trace, build/audio owner or raw child; root remains OK, driver611 still
Running, boot/six roles/twelve render controls unchanged. The prior guard's
exact transient owner is not recoverable and is not assigned to the audio fault.

## Next discriminating work

Preserve this completed failed attempt. A separately frozen future experiment
may distinguish post-install child rebinding from stable-binding native
preparation, without reinstalling/restarting the driver or changing audio
periods, buffers, epoch safety or quality gates. If preparation still fails,
capture exact setup/endpoint-lifecycle boundaries before pursuing timer cause.
The preceding-arm observation itself still needs an actual rebase/no-copy
witness. S1-S3, waveform continuity, sustained lower-budget output, matched
VAD/Process and calibrated physical acceptance all remain open.
