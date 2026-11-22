# Child PDO name publication prerequisite

Date: 2026-11-08, Asia/Shanghai. This source slice advances S4's supported
Windows name-publication path. It does not prove final retained MMDevice name
propagation, default/settings preservation, streaming rename or native S4.

## Production ordering

The actual device `EvtDevicePrepareHardware` callback calls `VadPreparePolicy`,
which enqueues and synchronously flushes the child-owned system work item.
The worker takes the existing locked requested-name snapshot and releases the
shared spin lock before property work. This is the documented PrepareHardware
lifetime, not an arbitrary IOCTL caller or audio stream/real-time callback.

`PreparePolicy` now stages `DEVPKEY_Device_FriendlyName` on the exact child PDO
with `IoSetDevicePropertyData`, then reads it with `IoGetDevicePropertyData`.
The setter receives a bounded NUL-terminated plain STRING and persistent flag;
the getter receives reserved flags0 and a fixed-capacity buffer. Type, exact
length, terminator and content must agree. No allocation, retry, root-device
write, interface-link parsing or SWD/MMDEVAPI endpoint-store write is added.

This gate runs before initial category publication and before returning from
the existing three-interface fast path. A cached policy cannot bypass changed
PDO-name validation. Invalid input, setter/getter failure or malformed readback
propagates through the existing preparation failure path and cannot authorize
subsequent ACX allocation/addition. Existing durable identity records, adopted
reference, category selection and ACX source are unchanged.

## Verification

Production-extracted Policy/Prepare/BuildCircuit regression first fails at the
missing pre-interface PDO verification, then passes. Tests cover12 failure
modes for each of initial and cached policy, both audio flows, exact child
ownership, locked-snapshot name, changed cached name and unchanged identity.
Composed failure checks execute the actual Prepare and BuildCircuit bodies and
require zero subsequent ACX allocation/addition, not only a mocked return code.

- All22 MSVC host suites and all17 Ubuntu24.04 portable suites pass.
- WDK Analyze build, catalog/signability and native InfVerif pass. This is the
  ordinary existing local build path, not a signed candidate deployment.
- The final comment-only clarification is followed by focused composed and
  interface suites plus WDK Analyze/InfVerif again.
- Parent independently verifies all three frozen source hashes and six command
  receipt hashes, reruns both focused suites, and checks diff whitespace.
- Independent static review clears the exact PDO/worker/order/failure slice.

Evidence reuses
`D:\coding\moiren-evidence\20261108-112100-collector-worker`.
`s4-pdo-name-source-checkpoint.json` records the exact source/command pins;
parent verification logs are `s4-pdo-parent-publication_reference.log` and
`s4-pdo-parent-interface_name_native.log`.

## Not final Windows endpoint naming

Microsoft explicitly supports driver FriendlyName publication during
Start/PrepareHardware, and the setter copies the supplied data. Its documented
object is the PDO. Neither this contract nor a successful typed PDO readback
proves that Endpoint Manager updates an already retained final MMDevice name.
The raw final MMDevice name reader and adapter-suffix acceptance remain intact;
requested/interface/PDO names cannot substitute for that final observation.

The next native question remains exact final-name propagation with stable
identity/settings/default roles under stopped, streaming, held-client,
sibling, race and repeated A-to-B-to-A cases. No unsupported basic endpoint
property-store writer is approved by this slice. Installed diagnostic613 and
its pending user-owned reboot remain unchanged. No host audio/UI, remote
operation, driver install, new ABI, copied executable tree or gate change occurs.

Primary contracts:

- <https://learn.microsoft.com/en-us/windows-hardware/drivers/develop/removing-coinstallers#setting-device-friendly-names>
- <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-iosetdevicepropertydata>
- <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-iogetdevicepropertydata>
