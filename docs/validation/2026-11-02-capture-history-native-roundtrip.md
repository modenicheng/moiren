# Native packet-history acquisition and analyzer roundtrip

Date: 2026-11-02, Asia/Shanghai. Source: clean committed `1255af2`.
This is one corrected10s source-only Process capture feature exercise, not
another stability soak, graph-path comparison, lower-target scan or physical
measurement. The [previous failed startup](2026-11-02-capture-history-analysis.md)
and its immutable deployment remain authoritative for that earlier attempt.

## Result and scope

The source reaches native Start, submits its complete6s indexed-v2 schedule,
then stops normally. The10.0003001s recorder completes and retains479040stereo
frames and998native packet offers without storage omission or saturation.
The actual framed history feeds the existing unchanged `analyze-indexed-pcm`
CLI. Both channels uniquely resolve all five codes0..4 with NCC1, all section,
onset and identity competition gates satisfied. Packet evidence schema2 has
exact stored coverage and no startup/observation issues. This verifies the
native acquisition-to-file-to-analyzer route that the prior startup failure
could not exercise.

A separate independent scalar PRNG audit compares the **entire retained source
capture** bit-for-bit after placing the independently regenerated five codes
at their already-resolved detector coordinates. Both channels match exactly:
5120nonzero code frames,473920expected-zero frames, zero mismatched frames,
zero unexpected nonzero frames and zero stereo bit differences. All four
relative marker spacings match48000frames exactly. No gain, offset, sample-rate
fit, resampling or per-window correction is applied. Detector coordinates are
input to this audit, not a second independent onset search or source-time origin.
Intentional zeros do not test meaningful delivery of arbitrary audio at those
positions. This source-only result cannot certify the application's VAD/SRC/Bus
route, invisible omissions within deliberate silence, native epochs or physical
latency. `qualified=false` remains in every report.

## Startup correction and preflight

The original harness created an empty activation file while the existing source
protocol requires exactly byte `1`. Only the outside-repository runner changes:
write the byte to a create-new temporary file, flush/close, then atomically move
it to the final control path without replacement. A local no-audio self-check
verifies byte49 and duplicate-publish refusal before deployment. Both actual
remote `activate` and `stop` files are one byte, with SHA256
`6B86B273FF34FCE19D6B804EFF5A3F5747ADA4EAA22F1D49C01E52DDB7875B4B`.
The source protocol and production application are not changed.

Read-only preflight09:54:55.668confirms `LAPTOP-NP49RG6H`, no Moiren/probe
owner, previous task Ready/result1, protected user identities unchanged, and
182033420288free bytes on D. Existing isolated Python3.10.11 is reused with
`-I -B`, single-thread numerical settings, output-local plotting cache, and
checks that numerical imports stay within that prefix. Hashing uses streaming
SHA256, not the unavailable Python3.11 `hashlib.file_digest`. No environment
installation or shared-environment modification is performed.

The new deployment is separate from the failed attempt:

```text
D:\MoirenTest\20261102-packet-native-fixed-0958
e2e_latency_probe.exe: 7485952 bytes
SHA256 AB03BE0D3FA9C6230B31E70E33C19EF535382A214E2CF48CEB5370038F1B1685
runner.ps1: 8155 bytes
SHA256 87C266BBC5457F60EF446CAD87C984125080F50A87EA92A57DA6D4C2538B2321
analyze.py: 6006 bytes
SHA256 4D7EB6D562BD3DC24E3612CA22430F039AB0038C038B2E305736499927799964
```

The probe is the final clean-export release from the preceding implementation,
including all three strict-reader review fixes. The new task is
`Moiren-PacketNativeFixed-20261102-0958`, limited interactive SID
`S-1-5-21-3080296863-1242338503-1346362208-1001`, lifecycle `task`,180s ceiling.
It launches09:56:26.680; the runner executes09:56:57.506-09:57:13.144and ends
Ready/result0. The directory suffix0958is an identifier, not the launch time.
An intermediate read-only observation sees the task Running; it is not
restarted or declared failed while awaiting the authoritative terminal state.

## Exact source and recorder

Only the source's Process-tree output is recorded, without an application
graph, independent recorder return or OBS action:

| Quantity | Exact value |
| --- | --- |
| Source endpoint | `{0.0.0.00000000}.{06eb2c6c-bab1-48b8-a0c9-af825233813d}` |
| Observed name | `Voicemeeter In 2 (VB-Audio Voicemeeter VAIO)` |
| Source mode | Shared, Float32,48000Hz stereo,1056native buffer frames |
| Source schedule | indexed-v2,5codes,1024frames each, amplitude0.008 |
| Nominal starts |24000,72000,120000,168000,216000source frames |
| Total scheduled source |288000frames; lifetime cap45s |
| Source policy | Latency-sensitive, MMCSS registered, event timeouts0 |
| Recorder |10s, max528000frames, max16384packet records |
| Capture native format | Windows-converted48000Hz stereo Float32 Process-tree output |
| Native result | completed,10.0003001s, native Stop succeeds |
| PCM/history |479040frames,998offers of480frames, all retained |

Source PID14352/creation100ns134380582182259004and capture25968/
creation134380582193721478are checked against their held process identities.
The recorder starts before source activation, with800ms readiness allowance.
Source final elapsed9.2658025s, acknowledged source288000, schedule complete,
native submitted445536including1056preroll, nonzero submitted5120, native Stop
success, worker joined, exit0. Acknowledged Release is submission, not physical
presentation. Recorder exits1by design because recording never physically
qualifies; its separate `recording_complete=true` is retained.

Capture aggregate counts are zero for dropped frames, overflow, discontinuity,
timestamp-error and SILENT-flag frames. Position-gap detection is false for
Process capture, and all998zero positions remain unknown. The delivery bound
is unset; its zero bound-violation counts are not a proved cadence contract.
Maximum observed callback gap111292in100ns units is11.1292ms, distinct from
the raw packet QPC spacing10ms. No source origin or latency is inferred from
either quantity. The unconsumed compatibility bridge's discarded copies are
separate from the native PCM tap, not artifact loss.

## Unchanged analysis and visualization

The existing exact-nonzero support convention splits after more than32zero
frames, pads64frames and supplies five1152-frame recorder windows per channel.
No source labels are supplied to those searches; every window competes with
all five original source codes. NCC0.7, margin0.05and RMS0.0001are unchanged,
including global/four-section/codeword/onset gates. Each channel charges
3302400correlation samples under the256000000cap. Both CLI commands exit0
for complete diagnostic identity sets, while `qualified=false`, unknown native
epoch and unknown source-to-recorder time origin remain explicit.

| Code | Recorder frame, both channels | NCC | Nominal relative spacing |
| ---: | ---: | ---: | ---: |
|0|64000|1|0|
|1|112000|1|48000|
|2|160000|1|96000|
|3|208000|1|144000|
|4|256000|1|192000|

The0.687s remote offline analysis produces `packet-file-smoke.png`: full10ms
RMS-bin tracks, all retained raw packet QPC intervals, and diagnostic NCC. Visual
review confirms all five signal islands and explicit unqualified labels.
The independent full-bit audit uses scalar xorshift32 seed0x6d2b79f5and exact
f32amplitude, including the whole expected-zero tail. Both tasks hash their
actual native inputs before/after unchanged. Raw3.83MB PCM stays remote;
only compact results and PNG are transferred.

## Artifacts and cleanup

Evidence root:
`D:\coding\moiren-evidence\20261101-clean-comb-probe-1320`.
Preflight, self-check, intent, runner, launcher, status and final audit use
`s1-packet-native-fixed-`. Compact downloads are below
`s1-packet-native-fixed-compact`; actual outputs remain in remote `run`.
All compact artifact lengths and SHA256 are compared with remote manifests:

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `summary.json` |14840|`1DF3712AC0F07E378F5D61F8D46BDE567483D5D94FF4BCC0E07AF2AF401702F6`|
| `analysis-0.json` |6868|`2E2827E7FEA7F94841A1CF000DF64B689A5B2D6EDF9AD36ECDCF0C19C6DAD0CB`|
| `analysis-1.json` |6868|`E0717976326E15D03A3DFD0FE57F79C36B5E3EFD8941762964A440B2D580D1E1`|
| `packet-file-smoke.png` |132548|`FD88A68A71DA4E323E9A5B2AADDE07EA4152E237E3E28FC85857C38FD9F00CB2`|
| `exact-source-audit.json` |1988|`615D94BF7FBE6FB0EDB56BE388C44FE0FC60F9ADC836FB0C757F4DC2559D0350`|
| `result.json` |34073|`733867D443C9011CB493E60F5B8EA6D992CEB43E1089640C1A8ADFEDE9797741`|

The3832320-byte PCM hash is
`4954C04EF73346E00274618D7E305332C8E7F25279C2C5920471F31DF6F84DF2`;
the177602-byte framed history hash is
`7122070E855D75D4C7698CAE369CF839D8F846283E3CA761F11137A787741B3B`.
Final audit pins native reports, metadata, binaries and controls separately.

Source, recorder and owned analysis PID23132/creation134380582295731265all
exit normally. No forced kill or cleanup error occurs, stderr is empty, and
final read-only audit finds no owned process. OBS12648/start1793505454949and
QQMusic15096/start1793504036422remain unchanged, as do endpoint defaults,
volume/mute. No driver operation or host audio/UI/process action is performed.
No production code/default/gate changes, push or memory update. First-packet
work remains deferred. The successful source-only roundtrip does not repair
the old aggregate-only VAD recordings or resolve their three waveform refusals;
matched graph-waveform and calibrated physical qualification remain open.

Fresh hardware-free regression against the clean export passes all13history
tests, exit0,666filtered. Receipt:
`s1-packet-native-fixed-regression-1102.log`. No source-code change follows the
preceding678-test full probe/workspace2178/8ignored verification; this increment
records native evidence and the external harness correction only.
