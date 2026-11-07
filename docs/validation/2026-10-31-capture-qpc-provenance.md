# Retained MIC capture QPC label provenance

## Result and scope

Two selected MIC packet labels exactly equal the retained native callback/
completion QPC witnesses in the [both-minimum recording](2026-10-31-event-arm-both-minimum.md).
This narrows timestamp provenance for those packets. It does not establish
packet start times, every packet's timestamp origin, a calibrated sample clock,
whole-waveform continuity or accepted quality. All9686 original native-QPC
diagnostic policy violations remain visible. S1-S3 remain incomplete.

Evidence root:
`D:\coding\moiren-evidence\20261031-capture-qpc-provenance`.
Repository base96dbd84. This slice adds a bounded, host-only Python association
tool, portable regressions and their CI entry. No production Rust, driver,
ABI, timer, waveform/model, buffer/default or acceptance policy changes. It
uses already-acquired bytes; no native acquisition, installation, reboot,
system-control change or physical rendering occurs.

## Contracts and source boundary

The three relevant documented timestamps have distinct contracts:

- [AcxRtStreamNotifyPacketComplete](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/acxstreams/nf-acxstreams-acxrtstreamnotifypacketcomplete)
  takes a completed packet's QPC timestamp, intended to be close to actual
  completion. It is not a packet-start argument. This contract alone does not
  prove that this driver's timestamp is optimally close to the copy boundary.
- [EVT_ACX_STREAM_GET_CAPTURE_PACKET](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/acxstreams/nc-acxstreams-evt_acx_stream_get_capture_packet)
  separately returns the last capture packet and its start QPC.
- [IAudioCaptureClient::GetBuffer](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
  documents a device position and associated first-frame QPC converted to
  100ns units. A returned numerical label is not independent calibration of
  that virtual device's position or capture-start semantics.

[ACX streaming](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/acx-streaming)
also describes shared completion packet-count/QPC information updated by the
notification call; clients can separately request capture packet information.
This makes a missing GetCapturePacket witness insufficient to prove an error
or that the client never used another ACX path. The four official pages are
retained with URL, byte count and SHA256 in `official-contracts-receipt.json`.

In `driver/vad/src/audio/Streams.cpp`, the event callback samples `now`, passes
it through `VadPcmTransfer` and `RecordEventCaptureCopy`, then supplies it to
`NotifyPacketComplete` after PCM completion. The recorded `copy_qpc` is that
callback timestamp, not a fresh timestamp sampled at memcpy. Separately,
`LastCaptureStart=schedule.CompletedStartQpc` is returned by
`VadAudioGetCapturePacket` when that callback runs. There is no basis here to
replace the completion argument with a start timestamp or to subtract one
nominal period from every raw WASAPI label. Capture-start/position witnesses
are absent from this decoded trace; their actual path remains unknown.

`crates/moiren-windows-audio/src/capture/wasapi/packet.rs` passes GetBuffer's
position/QPC outputs unchanged into evidence and ingress. Thus the equality
below is present in raw packet metadata, not introduced by a later analyzer
conversion. The selected provider reports10MHz; comparison is deliberately
limited to exact numerical equality at that frequency. It does not generalize
to arbitrary QPC frequency or prove the complete Windows propagation path.

## Strict bounded association

Run from the repository root, with new output paths:

```text
python -B scripts/vad-capture-qpc.py <decoded-driver.txt> --packets <measurement.packets.jsonl> --report <report.json> --output <new-diagnostic.json>
python -B scripts/tests/vad-capture-qpc.tests.py
```

The tool joins all six required fixed fragments: first-copy shape/prefix,
first successful notify, first-nonzero shape/prefix and nonzero notify.
Interleaving and valid sibling endpoint messages do not change the join.
Missing/duplicate selected fragments, unsupported fields, malformed integers,
identity/stream/gate/request/packet differences, failed notification and
ambiguous or merely nearby QPC labels refuse. A first nonzero prefix can be
the first copy itself; the tool does not manufacture a later packet.

Identity comes from the Fresh last-Running/open MIC snapshot. The native stamp
is `[PDO token, session, endpoint generation, stream generation, gate]`.
The existing ABI requires PDO token and endpoint generation to be equal;
split identities refuse. WPP endpoint refers to that runtime id, not the
persistent stable GUID. Selected packet native sequence, request, WASAPI
sequence, storage coordinate, position, flags and Float32/stereo shape must
agree. It supports a complete sequence-one, fixed-shape stored packet set;
other shapes refuse rather than being repaired or inferred.

The real report stores capture details under
`collection.measurement.native_capture`; its top-level `native.capture` is
null. Observed info/capture/evidence format and sink/evidence packet/frame
totals are checked against metadata. Unknown or nonzero flags in unselected
packets remain counted, unknown timestamp intervals remain unchecked, and
QPC repeat/rewind remains a separate diagnostic. The scaled-integer interval
comparison is unchanged from `pcm/packets.rs`: one frame plus1000ppm at the
nominal schedule rate. No selected-label success relaxes that policy.

Trace/report/metadata reads are capped at4/16/16MiB respectively, using bounded
reads rather than only a prior file-size check. Trace has100000-line and
4096-byte-line caps; packet input has100000-record and1024-byte-record caps.
Unterminated packet records, duplicate JSON keys, nonfinite JSON constants,
invalid encodings and invalid integer types/bounds refuse. Output uses
create-new semantics. Every successful diagnostic still has `qualified=false`,
`all_interval_trace_complete=false` and
`packet_start_time=unknown_not_derived`.

## Acquired evidence outcome

Both rows share endpoint6, session1327385863953, stream generation1,
gate1 and stream46173760573400. Both have144 frames, flags0, two native packets,
1152-byte packet/native shape, ring offset2944 and packet offset0.

| Witness | Native/request | WASAPI sequence | Stored start | Device position | Callback/raw QPC |
| --- | --- | --- | --- | --- | --- |
| First copy | 0/1 | 1 | 0 | 144 | 1327398897743 |
| First nonzero prefix | 228/229 | 229 | 32832 | 32976 | 1327405736851 |

First-copy16-byte source/destination prefixes are equal and zero; the
nonzero witness's16-byte prefixes are equal and nonzero. These fixed prefixes
do not establish whole-packet PCM equality or source marker identity.
First notify reports STATUS_SUCCESS with the same callback/completion label.
Nonzero notify enter/return are1327405736926/1327405736930,75 ticks after the
callback at entry and4 ticks within the call. At the reported10MHz these are
7.5us and0.4us, respectively, not capture-to-playback delay or scheduling
uncertainty bounds.

All10111 retained packets cover1455984 stored frames. All device positions
increment144, all flags are0 and there are no unknown/repeated/rewound QPC
labels. Of10110 checked QPC intervals,9686 remain outside the original policy.
Raw interval min/max are24443/35320hns; median30133hns and mean
29999.798516320476hns. A nominal-looking mean does not eliminate the individual
violations or establish a native loss count. The nominal14-unique/15-missing
marker refusal and subsequent [29-code model diagnostic](2026-10-31-both-minimum-src-batch.md)
are unchanged, separate evidence.

The original `quality_passed=true` coexists with `quality_complete=false`.
Both the collector receipt and `Quality::boundary_evidence()` in
`vad_collection.rs` explicitly write incomplete quality. The14 unknown
boundary observations are therefore not the sole reason for incompleteness;
they remain unknown, not zero faults. Removing that constant requires an
independently justified complete-quality contract and evidence, not this
two-witness diagnostic.

## Verification and retained revision history

The original11-test stub run has three supported-path errors and one CLI
missing-output error; its rejection passes are not meaningful RED evidence.
After correcting the real report fixture and failure assertions, the16-test
RED run has seven expected assertion failures. The implemented first version
then passes17 tests and the existing15 rebase regressions; its single offline
analysis and40-check independent review are retained.

Review then finds that the draft incorrectly permits a split native stamp.
The new focused regression first fails with `ValueError not raised`; the
minimal guard aligns the tool with the existing protocol. Nothing changes in
the source recording. A separately pinned final revision passes17 association
tests and15 rebase tests, and evaluates the same frozen inputs once over
15:43:54.906580-15:43:55.312676UTC with exit0. That exit means association
success only. The final JSON is byte-identical to the retained first-version
JSON. These are two disclosed offline analyses of one historical acquisition,
not a new native replay or an overwritten diagnostic.

The final45-check independent review imports neither the tool nor its tests.
It verifies12 original source/evidence inputs, nine final execution pins,
four official-page hashes, both exact packet/witness tuples, all packet shapes,
positions and flags, rationally recomputed tolerance violations, the unchanged
original nominal report, output/input pins and retained quality refusal.
Syntax and `git diff --check` also pass. CI now runs the association regressions
beside the existing rebase regressions; no remote CI run is claimed and no
Cargo/native suite is relabelled as current by this Python-only change.

Key SHA256 pins:

- Frozen hypothesis4044 bytes:
  `E91EFC6BFC9DDFC3D0C7CD7068B45E84DFC4E59E5443AC669D6BC48278B01D8E`.
- Final execution pins2367 bytes:
  `C935530BB49F205A598EB80969DD434ECDE6691DBB874F83F28BF107A3D1D39B`.
- Final association3136 bytes:
  `71CB897CBB666D350ED8D734AEECE9C6243C9797B6D3E45ED2C58EC90862455B`.
- Final execution receipt1604 bytes:
  `BF9A28B84E28735E428B45D94DDC077621BFBFE2E31952189D2DADA27E1A04D3`.
- Final independent receipt4660 bytes:
  `6B0F29EF63B143262A8FC4F304E72D98BFA95E67C7AC2A050A5986A3A5FBBBFA`.

Complete capture/clock handling, continuous-waveform quality, a matched
preceding-arm/rebase witness, native verification of final retired QUERY,
sustained lower-budget delivery, matched source-path comparison and calibrated
physical acceptance remain open. No stable sub-10-ms result, device floor,
impossibility claim or lower-default promotion follows.
