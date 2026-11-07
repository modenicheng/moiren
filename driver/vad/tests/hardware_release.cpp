// SPDX-License-Identifier: MIT
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <vector>

// Execute real ReleaseHardware code; these DDIs do not access a device.
using NTSTATUS = std::int32_t;
using WDFCMRESLIST = void*;
using ACXCIRCUIT = void*;
constexpr NTSTATUS STATUS_SUCCESS = 0;
constexpr NTSTATUS Failure = -37;
constexpr bool FALSE = false;
#define PAGED_CODE() ((void)0)
#define UNREFERENCED_PARAMETER(value) ((void)(value))
#define NT_SUCCESS(status) ((status) >= 0)
static void Check(bool ok, const char* message)
{
    if (!ok) { std::fprintf(stderr, "FAIL: %s\n", message); std::exit(1); }
}
#define NT_ASSERT(value) Check((value), "production assertion")

struct VAD_AUDIO_DEVICE {
    bool Added = true;
    bool HardwareReady = true;
    bool Powered = true;
    ACXCIRCUIT Circuit = nullptr;
    void* HostPin = nullptr;
    bool Locked = false;
    bool Ready = true;
    unsigned Stops = 0, Removes = 0, Deletes = 0;
    NTSTATUS RemoveResult = STATUS_SUCCESS;
};
using WDFDEVICE = VAD_AUDIO_DEVICE*;
struct Observation { bool Returned; NTSTATUS Status; bool RemovedCircuit; };
static std::vector<Observation> Observations;
VAD_AUDIO_DEVICE* VadAudioDevice(WDFDEVICE child) { return child; }
void VadAudioLock(VAD_AUDIO_DEVICE* device)
{ Check(!device->Locked, "no recursive lifecycle lock"); device->Locked = true; }
void VadAudioUnlock(VAD_AUDIO_DEVICE* device)
{ Check(device->Locked, "unlock owned lifecycle"); device->Locked = false; }
void VadAudioSetReady(WDFDEVICE device, bool ready)
{ Check(!device->Locked, "gate update outside lifecycle"); device->Ready = ready; }
void VadAudioStopDeviceLocked(VAD_AUDIO_DEVICE* device)
{ Check(device->Locked && !device->Ready, "stop with closed gate under lifecycle"); ++device->Stops; }
void VadAudioRecordRelease(WDFDEVICE device, bool returned, NTSTATUS status, bool removedCircuit)
{
    Check(!device->Locked, "diagnostic collection never nests lifecycle and shared locks");
    Observations.push_back({returned, status, removedCircuit});
}
NTSTATUS AcxDeviceRemoveCircuit(WDFDEVICE device, ACXCIRCUIT circuit)
{
    Check(!device->Locked && !device->Ready && !device->HardwareReady && !device->Powered &&
        device->Stops == 1 && device->Added && circuit == device->Circuit,
        "actual ACX removal retains the existing unlocked stopped-device contract");
    Check(Observations.size() == 1 && !Observations[0].Returned,
        "release entry must be retained before the potentially reentrant ACX call");
    ++device->Removes;
    return device->RemoveResult;
}
void WdfObjectDelete(ACXCIRCUIT circuit)
{
    auto device = static_cast<VAD_AUDIO_DEVICE*>(circuit);
    Check(!device->Locked && !device->Added && !device->Circuit && !device->HostPin,
        "delete unpublished circuit outside lifecycle after clearing borrowed handles");
    ++device->Deletes;
}

#include "hardware-release-production.inc"

static void Run(bool added, bool circuit, NTSTATUS result)
{
    Observations.clear();
    VAD_AUDIO_DEVICE device;
    device.Added = added;
    device.Circuit = circuit ? &device : nullptr;
    device.HostPin = &device;
    device.RemoveResult = result;
    const auto status = Release(&device, nullptr);
    Check(status == (added ? result : STATUS_SUCCESS), "release preserves the real NTSTATUS");
    Check(Observations.size() == 2 && !Observations[0].Returned && Observations[1].Returned &&
        Observations[1].Status == status && Observations[1].RemovedCircuit == added,
        "retain release entry/return and distinguish absent circuit from actual ACX result");
    Check(!device.Locked && !device.Ready && !device.HardwareReady && !device.Powered,
        "diagnostics never reopen gate/readiness or leave lifecycle held");
    Check(device.Removes == (added ? 1u : 0u) && device.Stops == 1 &&
        device.Deletes == (!added && circuit ? 1u : 0u), "no extra remove/delete/stop");
    if (added && !NT_SUCCESS(result)) {
        Check(device.Added && device.Circuit == &device && device.HostPin == &device,
            "failed ACX removal retains existing circuit ownership for framework handling");
    } else {
        Check(!device.Added && !device.Circuit && !device.HostPin,
            "successful/no-publication release clears stale circuit and pin handles");
    }
}

int main()
{
    Run(true, true, Failure);
    Run(true, true, STATUS_SUCCESS);
    Run(false, true, Failure);
    Run(false, false, Failure);
    std::puts("PASS: production hardware-release status, entry/return evidence and ownership (4 cases)");
}
