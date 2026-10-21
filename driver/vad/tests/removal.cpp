// SPDX-License-Identifier: MIT
#include "../src/Lifecycle.h"
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <initializer_list>
#include <vector>

// Execute production REMOVE/cleanup/destroy bodies, stubbing only DDI boundaries.
// Explicit callback injection checks ordering, not WDF concurrency or target IRQLs.
using ULONG = std::uint32_t;
using LONG = std::int32_t;
using NTSTATUS = std::int32_t;
using KIRQL = unsigned;
constexpr KIRQL PASSIVE_LEVEL = 0;
constexpr KIRQL DISPATCH_LEVEL = 2;
constexpr NTSTATUS STATUS_SUCCESS = 0;
constexpr NTSTATUS STATUS_NOT_FOUND = -3;
constexpr NTSTATUS FirstFailure = -37;
constexpr NTSTATUS RepeatFailure = -91;
constexpr int TRACE_LEVEL_INFORMATION = 4;
constexpr int TRACE_LEVEL_WARNING = 3;
constexpr int TRACE_LEVEL_ERROR = 2;
constexpr int VAD_PDO = 1;
constexpr ULONG TestIndex = 17;

struct Device;
using WDFDEVICE = Device*;
using WDFOBJECT = Device*;
struct SpinLock {
    bool Held = false;
};
struct State {
    SpinLock Lock;
    vad::Slot Slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    WDFDEVICE Children[MOIREN_VAD_MAX_ENDPOINTS] = {};
    unsigned Purges = 0;
    unsigned SharedReferences = 1;
    unsigned SharedReleases = 0;
};
struct ChildContext {
    State* Shared = nullptr;
    ULONG Index = TestIndex;
    uint64_t Id = 0;
    volatile LONG CircuitDeviceAdded = 1;
};
struct Device {
    State* Shared = nullptr;
    uint64_t Session = 7;
    ChildContext Context;
    std::vector<NTSTATUS> RemoveResults;
    unsigned RemoveCalls = 0;
    unsigned Clears = 0;
    unsigned PolicyReleases = 0;
    unsigned ReferenceCalls = 0;
    unsigned DereferenceCalls = 0;
    // A published child begins with framework and parent table references.
    unsigned References = 2;
    bool NativePcm = true;
    bool CleanupDuringRemove = false;
    bool Destroyed = false;
};
static KIRQL CurrentIrql = PASSIVE_LEVEL;

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

#define NT_ASSERT(condition) Check((condition), "kernel assertion")
bool NT_SUCCESS(NTSTATUS status) { return status >= 0; }
KIRQL KeGetCurrentIrql() { return CurrentIrql; }
template<typename... Args>
void VadTrace(int, int, const char*, Args...) {}
Device* VadRoot(Device* device) { return device; }
ChildContext* VadChild(Device* child)
{
    Check(!child->Destroyed, "child context stays live through the final reference");
    return &child->Context;
}
void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Check(!lock->Held, "State cannot recurse");
    *previous = CurrentIrql;
    CurrentIrql = DISPATCH_LEVEL;
    lock->Held = true;
}
void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Check(lock->Held, "State must be held");
    lock->Held = false;
    CurrentIrql = previous;
}
LONG InterlockedCompareExchange(volatile LONG* value, LONG next, LONG expected)
{
    const auto previous = *value;
    if (previous == expected) {
        *value = next;
    }
    return previous;
}
LONG InterlockedExchange(volatile LONG* value, LONG next)
{
    const auto previous = *value;
    *value = next;
    return previous;
}
void VadChildCleanup(WDFOBJECT object);
void VadChildDestroy(WDFOBJECT object);
void WdfObjectReference(Device* child)
{
    Check(child->Shared->Lock.Held && child->References != 0 && !child->Destroyed,
        "temporary REMOVE reference is acquired under State before dropping it");
    ++child->References;
    ++child->ReferenceCalls;
}
void WdfObjectDereference(Device* child)
{
    Check(!child->Shared->Lock.Held && child->References != 0 && !child->Destroyed,
        "release a live reference outside State");
    ++child->DereferenceCalls;
    if (--child->References == 0) {
        Check(child->PolicyReleases == 1 &&
            child->Shared->Children[child->Context.Index] == nullptr,
            "final destroy follows cleanup and table-reference release");
        VadChildDestroy(child);
        child->Destroyed = true;
    }
}
static void CheckRevoked(Device* child)
{
    const auto& slot = child->Shared->Slots[child->Context.Index];
    Check(!child->Shared->Lock.Held && CurrentIrql == PASSIVE_LEVEL &&
        slot.State == MOIREN_VAD_STATE_REMOVING &&
        slot.GateOpen == 0 && slot.AudioReady == 0,
        "revoke gate/audio under State before any passive DDI");
}
void VadAudioClearGateBuffers(Device* child)
{
    CheckRevoked(child);
    Check(child->References >= 3, "native clear retains child across unlocked work");
    child->NativePcm = false;
    ++child->Clears;
}
void VadPcmPurge(State* shared, ULONG index, bool restart)
{
    Check(!shared->Lock.Held && CurrentIrql == PASSIVE_LEVEL && restart &&
        shared->Slots[index].GateOpen == 0 && shared->Slots[index].AudioReady == 0,
        "purge revoked requests outside State");
    shared->Slots[index].PendingRequests = 0;
    ++shared->Purges;
}
void VadReleasePolicy(Device* child)
{
    CheckRevoked(child);
    Check(child->Shared->Children[child->Context.Index] == nullptr,
        "cleanup unpublishes the table before releasing policy storage");
    ++child->PolicyReleases;
}
void VadSharedRelease(State* shared)
{
    Check(!shared->Lock.Held && shared->SharedReferences == 1 &&
        vad::Reusable(shared->Slots[TestIndex]),
        "final destroy retires the slot before releasing its shared reference");
    --shared->SharedReferences;
    ++shared->SharedReleases;
}
NTSTATUS AcxDeviceRemoveCircuitDevice(Device* root, Device* child)
{
    Check(root->Shared == child->Shared && root->Session == 7,
        "retry uses the same root owner and child identity");
    CheckRevoked(child);
    Check(child->References == 3 && !child->NativePcm &&
        child->Shared->Slots[child->Context.Index].PendingRequests == 0 &&
        child->Context.CircuitDeviceAdded == 1 &&
        child->Clears == child->RemoveCalls + 1 &&
        child->Shared->Purges == child->RemoveCalls + 1,
        "each ACX attempt follows native clear/purge and retains a temporary reference");
    Check(child->RemoveCalls < child->RemoveResults.size(), "no unexpected ACX remove");
    const auto status = child->RemoveResults[child->RemoveCalls++];
    if (child->CleanupDuringRemove) {
        Check(NT_SUCCESS(status), "callback injection follows an accepted ACX remove");
        VadChildCleanup(child);
        WdfObjectDereference(child); // The framework releases its publication reference.
        Check(child->References == 1 && !child->Destroyed &&
            !vad::Reusable(child->Shared->Slots[child->Context.Index]),
            "REMOVE's temporary reference pins context and slot through ACX return");
    }
    return status;
}

#include "removal-production.inc"

struct Fixture {
    State Shared;
    Device Root;
    Device Child;
    uint64_t NextId = 0;

    Fixture()
    {
        Root.Shared = Child.Shared = &Shared;
        for (ULONG i = 0; i < MOIREN_VAD_MAX_ENDPOINTS; ++i) {
            Check(vad::Reserve(Shared.Slots, Root.Session, NextId,
                MOIREN_VAD_INTENT_PLAYBACK, 2) == static_cast<int>(i),
                "fill quota so only destruction can make capacity available");
        }
        auto& slot = Shared.Slots[TestIndex];
        slot.State = MOIREN_VAD_STATE_PRESENT;
        slot.PolicyPrepared = slot.GateOpen = slot.AudioReady = 1;
        slot.AdmissionGeneration = 42;
        slot.StreamGeneration = 6;
        slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
        slot.HasSequence = 1;
        slot.PendingRequests = 2;
        slot.StableId = {101, 202};
        Child.Context.Shared = &Shared;
        Child.Context.Id = slot.Id;
        Shared.Children[TestIndex] = &Child;
    }

    void NoReuse()
    {
        Check(!vad::Reusable(Shared.Slots[TestIndex]) &&
            vad::Reserve(Shared.Slots, 8, NextId, MOIREN_VAD_INTENT_MICROPHONE, 1) == -1 &&
            NextId == MOIREN_VAD_MAX_ENDPOINTS,
            "failure, acceptance and cleanup cannot reuse quota before final PDO destroy");
    }

    void CheckRemovedSnapshot(const MOIREN_VAD_ENDPOINT_SNAPSHOT& snapshot)
    {
        Check(snapshot.Size == sizeof(snapshot) && snapshot.Version == MOIREN_VAD_ABI_VERSION &&
            snapshot.EndpointId == Child.Context.Id && snapshot.SessionEpoch == Root.Session &&
            snapshot.EndpointGeneration == Child.Context.Id &&
            snapshot.StableId.Low == 101 && snapshot.StableId.High == 202 &&
            snapshot.StreamGeneration == 6 && snapshot.GateEpoch == 43 &&
            snapshot.State == MOIREN_VAD_STATE_REMOVING &&
            snapshot.GateOpen == 0 && snapshot.AudioReady == 0,
            "every reply keeps exact identity, closes admission once and reports Removing");
    }

    void Cleanup()
    {
        VadChildCleanup(&Child);
        Check(Shared.Children[TestIndex] == nullptr && Child.References == 1 &&
            Child.PolicyReleases == 1 && !Child.Destroyed,
            "cleanup releases exactly the table reference, not the framework reference");
        NoReuse();
    }

    void Destroy()
    {
        WdfObjectDereference(&Child);
        Check(Child.Destroyed && Child.References == 0 &&
            Child.DereferenceCalls == Child.ReferenceCalls + 2 &&
            Shared.SharedReleases == 1 && Shared.SharedReferences == 0 &&
            vad::Reusable(Shared.Slots[TestIndex]),
            "final PDO destroy balances table/framework/temporary/shared references");
    }
};

static void RepeatedFailuresRetrySameOwnerUntilSuccess()
{
    Fixture fixture;
    fixture.Child.RemoveResults = {FirstFailure, RepeatFailure, STATUS_SUCCESS};
    unsigned attempts = 0;
    for (const auto expected : {FirstFailure, RepeatFailure, STATUS_SUCCESS}) {
        MOIREN_VAD_ENDPOINT_SNAPSHOT snapshot = {};
        Check(VadRemoveChild(&fixture.Root, fixture.Child.Context.Id, &snapshot) == expected,
            "REMOVE propagates each real ACX failure until a retry succeeds");
        ++attempts;
        fixture.CheckRemovedSnapshot(snapshot);
        Check(fixture.Child.RemoveCalls == attempts &&
            fixture.Child.Context.CircuitDeviceAdded == (NT_SUCCESS(expected) ? 0 : 1) &&
            fixture.Child.References == 2 &&
            fixture.Child.ReferenceCalls == attempts && fixture.Child.DereferenceCalls == attempts &&
            fixture.Shared.Slots[TestIndex].HasSequence == 0 && !fixture.Child.NativePcm,
            "a failed remove retains publication for the next real ACX retry without leaks");
        fixture.NoReuse();
    }
    fixture.Cleanup();
    fixture.Destroy();
}

static void AcceptedRemovalIsIdempotentUntilDestroy()
{
    Fixture fixture;
    fixture.Child.RemoveResults = {STATUS_SUCCESS};
    MOIREN_VAD_ENDPOINT_SNAPSHOT snapshot = {};
    for (unsigned attempt = 1; attempt <= 3; ++attempt) {
        Check(VadRemoveChild(&fixture.Root, fixture.Child.Context.Id, &snapshot) == STATUS_SUCCESS,
            "accepted repeated REMOVE remains successful");
        fixture.CheckRemovedSnapshot(snapshot);
        Check(fixture.Child.RemoveCalls == 1 && fixture.Child.Context.CircuitDeviceAdded == 0 &&
            fixture.Child.ReferenceCalls == attempt && fixture.Child.DereferenceCalls == attempt,
            "accepted remove never calls ACX twice and releases each temporary reference");
        fixture.NoReuse();
    }
    fixture.Cleanup();
    const auto purges = fixture.Shared.Purges;
    Check(VadRemoveChild(&fixture.Root, fixture.Child.Context.Id, &snapshot) == STATUS_SUCCESS &&
        fixture.Child.RemoveCalls == 1 && fixture.Child.ReferenceCalls == 3 &&
        fixture.Shared.Purges == purges,
        "an already cleaned-up child has no second native remove or table reference");
    fixture.NoReuse();
    fixture.Destroy();
    Check(vad::Reserve(fixture.Shared.Slots, 8, fixture.NextId,
        MOIREN_VAD_INTENT_MICROPHONE, 1) == static_cast<int>(TestIndex) &&
        fixture.Shared.Slots[TestIndex].Id == MOIREN_VAD_MAX_ENDPOINTS + 1,
        "only final destruction permits reuse with a fresh runtime identity");
    Check(VadRemoveChild(&fixture.Root, fixture.Child.Context.Id, &snapshot) == STATUS_NOT_FOUND &&
        fixture.Child.RemoveCalls == 1,
        "the retired runtime identity cannot remove the reused slot");
}

static void CleanupInsideAcxKeepsTemporaryReferenceUntilReturn()
{
    Fixture fixture;
    fixture.Child.RemoveResults = {STATUS_SUCCESS};
    fixture.Child.CleanupDuringRemove = true;
    MOIREN_VAD_ENDPOINT_SNAPSHOT snapshot = {};
    Check(VadRemoveChild(&fixture.Root, fixture.Child.Context.Id, &snapshot) == STATUS_SUCCESS,
        "accepted remove tolerates cleanup before the ACX call returns");
    fixture.CheckRemovedSnapshot(snapshot);
    Check(fixture.Child.Context.CircuitDeviceAdded == 0 && fixture.Child.Destroyed &&
        fixture.Child.RemoveCalls == 1 && fixture.Child.ReferenceCalls == 1 &&
        fixture.Child.DereferenceCalls == 3 && fixture.Shared.SharedReleases == 1 &&
        vad::Reusable(fixture.Shared.Slots[TestIndex]),
        "MarkMissing and temporary release finish before exact-once final destruction");
}

static void ForeignSessionAndUnknownIdsCannotRemove()
{
    Fixture fixture;
    fixture.Child.RemoveResults = {STATUS_SUCCESS};
    MOIREN_VAD_ENDPOINT_SNAPSHOT snapshot = {};
    snapshot.EndpointId = 999;
    ++fixture.Root.Session;
    Check(VadRemoveChild(&fixture.Root, fixture.Child.Context.Id, &snapshot) == STATUS_NOT_FOUND,
        "another session cannot remove an old owner's identity");
    --fixture.Root.Session;
    for (const auto id : {uint64_t{0}, fixture.NextId + 1}) {
        Check(VadRemoveChild(&fixture.Root, id, &snapshot) == STATUS_NOT_FOUND,
            "zero or unknown runtime identity cannot revoke a child");
    }
    Check(fixture.Child.RemoveCalls == 0 && fixture.Child.Clears == 0 &&
        fixture.Shared.Purges == 0 && fixture.Child.ReferenceCalls == 0 &&
        fixture.Shared.Slots[TestIndex].GateOpen == 1 &&
        fixture.Shared.Slots[TestIndex].AudioReady == 1 && snapshot.EndpointId == 999,
        "rejected identity checks leave gate, PCM, references and output untouched");
    Check(VadRemoveChild(&fixture.Root, fixture.Child.Context.Id, &snapshot) == STATUS_SUCCESS,
        "the original owner can still remove its child");
    fixture.Cleanup();
    fixture.Destroy();
}

int main()
{
    RepeatedFailuresRetrySameOwnerUntilSuccess();
    AcceptedRemovalIsIdempotentUntilDestroy();
    CleanupInsideAcxKeepsTemporaryReferenceUntilReturn();
    ForeignSessionAndUnknownIdsCannotRemove();
    std::puts("PASS: production-extracted REMOVE retry/failure propagation, revocation, "
        "accepted idempotence, quota retirement, identity and reference lifetime (4 cases)");
}
