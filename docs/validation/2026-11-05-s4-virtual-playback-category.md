# S4 virtual playback terminal category candidate

Recorded 2026-11-05 (Asia/Shanghai). This is a source/host-test/WDK receipt,
not Windows displayed-name, native audio, default-role, or latency qualification.
S4 remains required and open. No requirement or existing acceptance gate changes.

## Isolation and scope

- Base: `1b33b687a527fcbd1ae1ce9b17b4537771c6e390`.
- Branch: `feat/vad-virtual-playback-category-20261105`.
- Worktree: `D:\coding\moiren\.worktrees\s4-virtual-playback-category`.
- Both the path and branch were absent before creation; the new worktree was
  clean. The source-pinned integration worktree was not modified.
- All changes remain unstaged/uncommitted for parent review. Nothing was pushed.
- Evidence: `D:\coding\moiren-evidence\20261104-pipeline-resume\s4-category`.
  The November 4 directory name is the parent pipeline namespace; these runs
  occurred November 5. It contains logs, a focused patch, source/package hashes,
  preserved-contract checks, and an unsigned candidate SYS/INF/CAT package.
- No native audio, UI, VM, SSH, driver installation, device enumeration, signing,
  or remote mutation was performed. The encrypted VM requires user unlock before
  separately authorized target work; it is not bypassed by host installation.

## Requirement and rationale

`docs/vad-low-latency-goals-2026-10-28.md` requires an editable displayed name
and preserved stable project identity, not an immutable MMDevice GUID. This
candidate retains the deliberate name-derived `EndpointReference` introduced
by `257d752` to obtain fresh endpoint-name composition after safe recreation.
It does not reverse that lifecycle decision or introduce an in-place rename IOCTL.

Moiren playback terminates in a software bridge, not a physical speaker. The
standard `KSNODETYPE_OUTPUT_UNDEFINED` classification is used rather than
pretending to be headphones/line-out or inventing an unnecessary custom GUID.
Microsoft documents a hardcoded naming exception for Speaker endpoints and
maps OUTPUT_UNDEFINED to UnknownFormFactor. This makes removing the Speaker
classification a justified candidate, not proof that the resulting composed
Windows FriendlyName will equal the requested text.

## Production change

`driver/vad/src/TerminalPolicy.h` supplies one immutable terminal definition to
both the ACX circuit and policy staging. The Association string is formatted at
compile time from that definition's category GUID, avoiding two independently
maintained identifiers. Nothing executes this formatting on the audio thread.

| Surface | Playback | Microphone |
| --- | --- | --- |
| Bridge category / Association | OUTPUT_UNDEFINED, `{DFF21CE0-F70F-11D0-B917-00A0C9223196}` | Existing MICROPHONE, `{DFF21BE1-F70F-11D0-B917-00A0C9223196}` |
| EnableEndpointByDefault | REG_DWORD `0x101` (render plus enable) | No new write |
| NeverSetAsDefaultEndpoint | Existing `0x307` when AllowDefault=false; zero for explicit local opt-in | Unchanged same policy |
| Placement | Association, exclusion and enablement together in each owned disabled interface's EP\0 | Existing Association/exclusion placement |

Enablement makes an UnknownFormFactor endpoint usable; it does not select the
system default. Failed enablement propagates the original failure, closes keys,
disables/frees the new interface and prevents later policy publication.
The existing interface name, typed exact readback, event-driven property and
endpoint name staging remain in place.

The INF and its fixed `ChildDeviceDescription` are byte-identical to the base.
No static AddInterface, custom MediaCategories, global category override, form
factor override, or speculative property is added. Dynamic interface policy
continues to be staged by the existing runtime path before ACX publication.
The existing bridge-pin name callback remains the naming source.

## Genuine RED before production edits

The existing MSVC `interface_name_native` suite and all 12 pre-change portable
groups passed first (`baseline-interface-name-native.log`, `baseline-portable.log`).
Then only tests/extraction/runner wiring changed; production sources were still
unchanged when these two behavioral regressions ran:

| Command | Result and evidence |
| --- | --- |
| `pwsh -NoProfile -File driver/vad/test.ps1 -Test interface_name_native` | Exit 1: `virtual playback Association is OUTPUT_UNDEFINED; microphone is unchanged` (`red-interface_name_native.log`) |
| `pwsh -NoProfile -File driver/vad/test.ps1 -Test terminal_policy` | Exit 1: `ACX virtual playback publishes OUTPUT_UNDEFINED, not SPEAKER; MIC is unchanged` (`red-terminal_policy.log`) |

The first executes the extracted real SnapshotPolicy/PreparePolicy/staging and
records actual registry payloads. The second executes the entire extracted real
VadAudioBuildCircuit and records the config passed to AcxPinCreate. Only external
DDI boundaries are stubbed; neither test scans for source text or simulates
EndpointBuilder. An initial new-test compile error (missing initializer_list
include) was corrected before its genuine RED; the failed scaffold log is kept
separately as `test-scaffold-compile-error.log` and is not counted as RED evidence.

## GREEN and build evidence

| Command | Fresh candidate result |
| --- | --- |
| `pwsh -NoProfile -File driver/vad/test.ps1 -Test all` | Exit 0, all 18 host groups (`green-all-host-groups.log`) |
| `bash driver/vad/test-portable.sh` | Exit 0, all 13 portable groups (`green-all-portable-groups.log`) |
| `pwsh -NoProfile -File driver/vad/build.ps1 -Analyze` | Exit 0, x64 Debug, code analysis enabled, 0 warnings / 0 errors (`wdk-analyze-console.log`, `wdk-build.log`) |
| Explicit InfVerif `/w /v` in build script | `INF is VALID`, one INF checked (`infverif.log`) |
| `git diff --check` | Exit 0 |

Host toolchains: MSVC 14.51.36231 / VS 18 Community; Windows Kits
10.0.28000.0; portable runner uses MSYS2 UCRT64 C++ and Python with
`PATH=/ucrt64/bin:/usr/bin:$PATH`. The full MSVC suite was rerun after the final
test-only registry-placement strengthening. No production source changed after
the analyzed WDK build or portable run.

Coverage includes both directions and both local AllowDefault values, exact
Association bytes including NUL, render-only enablement `0x101`, exclusion
`0x307`/zero, and their shared owned EP\0 placement. All 30 playback and 27
microphone registry-operation failure positions are injected, retaining rollback,
ownership, worker/lock and typed-name readback checks. The actual circuit test
retains host category, pin IDs, dataflow/communication, name callback, format
admission and two-pin publication assertions.

Unchanged-file blob comparisons are in `preserved-contracts.json`: public ABI,
INF, PDO identity, component identity, name hash, circuit initialization, format
and stream/PCM sources, and strict native rename harness. The harness still
requires exact raw FriendlyName equality and keeps historical strict-held failure
semantics; no suffix stripping or DeviceDesc substitution was introduced.

Unsigned candidate package SHA-256 (also `package-sha256.json`):

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| MoirenVad.sys | 145920 | `A2B0E398567090C2E3CA355DB79DDB1D131545BE53F36859A761078290190233` |
| MoirenVad.inf | 1803 | `092F7A125594A6B951FB30113815452960059E431E73739DB572FF059CDAF6E1` |
| moirenvad.cat | 1350 | `AD6B422BD2942D75A8EA9D0ACDF5EFD4D74C0EB3F838CB85F10A4DBE25E1DA6A` |

The build stamps the package INF's DriverVer; the source INF is unchanged.
Package generation/validation is not signing, installation, or loaded-driver
verification. The existing build explicitly uses SignMode=Off.

## Migration and unqualified behavior

### Integration Import And Independent Verification

The candidate is imported without modification into the separate
`minimal-latency-integration` worktree on base `b471975`. An independent read-only
review finds no actionable code issue; all eleven imported file lengths and
hashes match the candidate manifest. The integration copy freshly passes all
18 MSVC host groups and all 13 portable groups. Its analyzed WDK build completes
at 2026-11-05 00:52:43 with no reported warnings/errors and explicit valid INF.
No native driver operation follows. This is not Windows name acceptance.

These are separate build artifacts from the candidate-worktree package above;
build paths and generated DriverVer/catalog inputs differ. The integration
package is pinned in `s4-category/integration-package-pins.json`:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| MoirenVad.sys | 145920 | `4F1623B1B14F5EC170858F06BA48507F3295B2D21266592E1D0DD70D36BD7C15` |
| MoirenVad.inf | 1804 | `762EBB206664BA6C9004603FAE8CE27EE44A7F31BF68FFB6531FBFE9FF6D246F` |
| moirenvad.cat | 1178 | `CB8AB492C894719FA55BF69ECB8D62F6BCC2A127A19DEA8ED183F956E09FF817` |

Neither package is signed, installed or observed as a loaded driver. The
00:45 read-only VMware inventory still reports zero running VMs. The earlier
password-required start is not retried, and host installation is not a fallback.

### Native Acceptance Still Required

- Expected classification is UnknownFormFactor, with possible generic icon/UI
  and endpoint-ranking differences; no Speakers form-factor override is applied.
- Endpoint classification changes and later name-hash recreation can affect
  Windows endpoint/cache/settings behavior. No guarantee of preserving the old
  MMDevice GUID, user preferences, third-party selections, or uninterrupted
  audio is made. No new project identity is generated by this patch.
- Existing default exclusion remains active for the default local setting, and
  its Association now targets the actual playback category. Explicit AllowDefault
  opt-in still permits eligibility, not selection. A host registry-payload test
  cannot prove unchanged Windows default roles; the existing S4 target checks
  remain open, including behavior around removal/recreation.
- Native enumeration, enablement, actual Windows name/description/composition,
  client compatibility, streaming/held-client/race/repetition and new-audio
  acceptance are all unverified for this candidate. The INF adapter suffix may
  remain and the strict exact-name harness may still fail. No physical or
  software latency result is claimed.
- This is only the authorized category/policy candidate. It neither declares S4
  complete nor turns Windows-assisted rename into a replacement acceptance gate.

## Primary Microsoft documentation

Read 2026-11-05; direct sources for parent review:

1. Speaker naming exception and bridge-pin-based enumeration:
   https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-endpoint-builder-algorithm
2. OUTPUT_UNDEFINED -> UnknownFormFactor, hidden/disabled defaults, supported
   EnableEndpointByDefault property and mask:
   https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/pkey-audiodevice-enableendpointbydefault
3. Required same-EP Association for default exclusion and exclusion semantics:
   https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/pkey-audiodevice-neversetasdefaultendpoint
4. Association is the KS pin-category GUID represented as a string:
   https://learn.microsoft.com/en-us/windows/win32/coreaudio/pkey-audioendpoint-association
5. Naming, standard/custom categories, device-local registration and Windows UI:
   https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/friendly-names-for-audio-endpoint-devices
6. Default selection, state changes, identity/settings consequences:
   https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/default-audio-endpoint-selection
7. Distinct endpoint description, adapter name and full composed FriendlyName;
   applications read rather than set these basic endpoint properties:
   https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-properties
