# Source and licensing

Reference repository: <https://github.com/microsoft/Windows-driver-samples>

Pinned commit: `2dc3fd3a0cc84a2933f2194e7ec0871584979071`.

The existing local checkout was found at
`.tmp/vad-evaluation-20261019/microsoft-samples`; its HEAD was verified with
`git rev-parse HEAD`. No fetch or changes to that checkout were necessary.
Existing AudioCodec x64 Debug outputs/logs were read, not rebuilt or modified.
The new project has no build dependency on that checkout.

The pinned repository's root `LICENSE` is **Microsoft Public License (MS-PL)**,
not MIT. That license is retained verbatim in `licenses/Microsoft-MS-PL.txt`.
The adapted scaffold keeps Microsoft's copyright and MS-PL identifiers.
New files marked `SPDX-License-Identifier: MIT` use the adjacent MIT `LICENSE`.
The combined package must preserve both notices; it is not entirely MIT.

| Local adapted file | Upstream path below `audio/Acx/Samples/AudioCodec/Driver/` | Pinned Git blob |
| --- | --- | --- |
| `src/Driver.cpp` | `Driver.cpp` | `452b0d99eaf24451d965bd1b0f3ef140417e6d98` |
| `src/Device.cpp` | `Device.cpp` | `7e9a0a3c6d273771b9d71ae72dea7e26e583aa34` |
| `MoirenVad.vcxproj` | `AudioCodec.vcxproj` | `0cdc9e6154c6340d384c8b07550d6ecf11a2e6d3` |
| `MoirenVad.inf` | `AudioCodec.inf` | `0bc5fe2314b32e30e13827e3d421911048d8f579` |

Pinned root LICENSE blob: `6ef3ee38d1ce61958787dda3b8c669627b248080`.

## Deliberate differences

- A separate Moiren root hardware ID, service, project, control GUID and private
  raw-device security class; x64 Debug only, ACX 1.1, KMDF 1.31, WDK 28000.
- No static speaker/microphone circuits or audio interfaces in the root INF.
- No sample registry settings, global allocation helpers, WPP recorder dependency,
  DSP controls, offload, keyword detector, SDCA, simulated meter or DSP/SRC.
- New versioned fixed-width control messages, one file-object owner, 64 fixed
  slots, closed admission and asynchronous child retirement.
- New registry staging writes Association plus NeverSetAsDefaultEndpoint to the
  same EP/0 key on registered but disabled interfaces; every failure aborts
  publication.
- Dynamic circuits use an instance component URI derived from their complete
  stable endpoint identity and direction rather than sharing a sample GUID.
- `Common/RenderCircuit.cpp`, `CaptureCircuit.cpp`, and `StreamEngine.cpp` were
  inspected as architectural references. Their tone generation, saved/rendered
  sample files and capture-file reader are not linked into this foundation.
  `StreamEngine.cpp` routes render packets to SaveData and capture packets through
  WaveReader or ToneGenerator; these are sample behaviors, not a Moiren transport.

## Primary API references checked online

- [ACX device enumeration](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/acx-device-enumeration)
- [ACX targets and synchronization](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/acx-targets)
- [NeverSetAsDefaultEndpoint and matching Association](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/pkey-audiodevice-neversetasdefaultendpoint)
- [WdfFdoAddStaticChild](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdffdo/nf-wdffdo-wdffdoaddstaticchild)
- [WdfPdoMarkMissing](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdfpdo/nf-wdfpdo-wdfpdomarkmissing)
- [ACX child registration](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/acxdevice/nf-acxdevice-acxdeviceaddcircuitdevice)
- [ACX child removal](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/acxdevice/nf-acxdevice-acxdeviceremovecircuitdevice)
- [ACX circuit instance identity](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/acx-circuits)
- [Component URI assignment](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/acxcircuit/nf-acxcircuit-acxcircuitinitassigncomponenturi)

The policy mask is `0x307`, combining every flow and role flag. The policy page's
later `0x305` prose is inconsistent with its example and complete flag set.
