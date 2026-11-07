// SPDX-License-Identifier: MIT
#include "../src/Lifecycle.h"
#include "../src/RemovalDiagnostic.h"
#include <cstdio>
#include <cstdlib>
#include <vector>

using ULONG = uint32_t;
using NTSTATUS = int32_t;
using KIRQL = unsigned;
constexpr KIRQL PASSIVE_LEVEL = 0, DISPATCH_LEVEL = 2;
constexpr unsigned TRACE_LEVEL_INFORMATION = 4, VAD_CONTROL = 1;
static void Check(bool ok, const char* message)
{
    if (!ok) { std::fprintf(stderr, "FAIL: %s\n", message); std::exit(1); }
}
#define NT_ASSERT(value) Check((value), "production assertion")
static bool TraceEnabled = false;
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) TraceEnabled
struct SpinLock { bool Held = false; };
struct VAD_SHARED {
    SpinLock Lock;
    vad::Slot Slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    vad::RemovalDiagnostic RemovalDiagnostics[MOIREN_VAD_MAX_ENDPOINTS] = {};
};
struct Child { VAD_SHARED* Shared; ULONG Index; uint64_t Id; };
using WDFDEVICE = Child*;
static KIRQL CurrentIrql = PASSIVE_LEVEL;
static uint64_t Qpc = 100;
struct LARGE_INTEGER { long long QuadPart; };
[[maybe_unused]] static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
static Child* VadChild(WDFDEVICE child) { return child; }
static LARGE_INTEGER KeQueryPerformanceCounter(LARGE_INTEGER* frequency)
{
    frequency->QuadPart = 10000000;
    return {static_cast<long long>(Qpc++)};
}
static void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Check(!lock->Held, "shared lock never recurses");
    *previous = CurrentIrql; CurrentIrql = DISPATCH_LEVEL; lock->Held = true;
}
static void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Check(lock->Held, "release the owned shared lock");
    lock->Held = false; CurrentIrql = previous;
}
struct TraceRecord { const char* Format; std::vector<uint64_t> Values; };
static std::vector<TraceRecord> Traces;
static SpinLock* TraceLock = nullptr;
template<typename... Args>
static void VadTrace(unsigned, unsigned, const char* format, Args... args)
{
    Check(CurrentIrql == PASSIVE_LEVEL && TraceLock && !TraceLock->Held,
        "all WPP runs at PASSIVE outside Shared->Lock");
    Check(sizeof...(Args) <= 12, "bounded decodable WPP payload");
    Traces.push_back({format, {static_cast<uint64_t>(args)...}});
    Check(Traces.size() <= 25, "bounded stage fragments plus optional overwrite notice");
}

#include "removal-trace-production.inc"

#ifdef HAS_PRODUCTION_REMOVAL_TRACE
static void ActualCollectionAndPassiveExport()
{
    VAD_SHARED shared;
    Child child = {&shared, 3, 21};
    auto& slot = shared.Slots[3]; slot.Id = 21; slot.Session = 99;
    auto& d = shared.RemovalDiagnostics[3]; d.Reset(21, 99);
    TraceLock = &shared.Lock;
    VadRecordRemoval(&child, vad::RemovalStage::RemoveEnter, false, 0, false);
    VadRecordRemoval(&child, vad::RemovalStage::RemoveReturn, true, 0, true);
    VadAudioRecordRelease(&child, false, 0, false);
    VadAudioRecordRelease(&child, true, -37, true);
    CurrentIrql = DISPATCH_LEVEL;
    VadRecordRemoval(&child, vad::RemovalStage::Cleanup, false, 0, false);
    VadRecordRemoval(&child, vad::RemovalStage::Destroy, false, 0, false);
    Check(CurrentIrql == DISPATCH_LEVEL && Traces.empty() && d.Sequence == 6,
        "destroy-safe scalar collection does not trace or lower the caller's IRQL");
    CurrentIrql = PASSIVE_LEVEL;
    const auto snapshot = vad::Snapshot(slot);
    TracePendingRemovalDiagnostic(&shared, snapshot);
    Check(Traces.empty() && d.Pending != 0, "disabled WPP never consumes pending evidence");
    TraceEnabled = true;
    auto wrong = snapshot; ++wrong.SessionEpoch;
    TracePendingRemovalDiagnostic(&shared, wrong);
    Check(Traces.empty() && d.Pending != 0, "foreign session cannot drain lifecycle evidence");
    TracePendingRemovalDiagnostic(&shared, snapshot);
    Check(!Traces.empty() && d.Pending == 0 && d.Stages[3].FirstFailure.Status == uint32_t(-37) &&
        d.Frequency == 10000000 && d.Stages[5].Last.Sequence == 6,
        "actual pending export preserves native failure and exact runtime clock/order");
    bool retainedFailure = false;
    for (const auto& record : Traces) {
        Check(record.Values.size() >= 2 && record.Values[0] == 21 && record.Values[1] == 99,
            "every fragment carries exact runtime/session correlation");
        for (auto value : record.Values) {
            if (value == static_cast<uint64_t>(NTSTATUS{-37})) { retainedFailure = true; }
        }
    }
    Check(retainedFailure, "real signed NTSTATUS is exported without a success substitute");
    const auto count = Traces.size();
    TracePendingRemovalDiagnostic(&shared, snapshot);
    Check(Traces.size() == count, "unchanged QUERY never emits the same pending stages twice");
    ++slot.Id;
    VadAudioRecordRelease(&child, true, -91, true);
    Check(d.Sequence == 6, "stale child cannot append evidence after slot reuse");
    d.Record(21, 99, vad::RemovalStage::Destroy, 200);
    d.Reset(slot.Id, slot.Session);
    Traces.clear();
    TraceEnabled = false;
    TracePendingRemovalDiagnostic(&shared, vad::Snapshot(slot));
    Check(Traces.empty() && d.Pending == vad::RemovalLossPending,
        "disabled WPP also preserves overwrite evidence");
    TraceEnabled = true;
    TracePendingRemovalDiagnostic(&shared, vad::Snapshot(slot));
    Check(Traces.size() == 1 && d.Pending == 0 && d.DiscardedRuntimes == 1 &&
        Traces[0].Values.size() == 8 && Traces[0].Values[0] == slot.Id &&
        Traces[0].Values[3] == 21 && Traces[0].Values[4] == 99,
        "actual exporter explicitly reports a reused runtime's unexported pending evidence");
    TracePendingRemovalDiagnostic(&shared, vad::Snapshot(slot));
    Check(Traces.size() == 1, "unchanged query never repeats a drained loss notice");
    d.Record(slot.Id, slot.Session, vad::RemovalStage::Destroy, 300);
    d.Reset(slot.Id, slot.Session);
    Traces.clear();
    for (unsigned stage = 0; stage < vad::RemovalStageCount; ++stage) {
        d.Record(slot.Id, slot.Session, static_cast<vad::RemovalStage>(stage),
            400 + stage, true, uint32_t(-37), true);
    }
    TracePendingRemovalDiagnostic(&shared, vad::Snapshot(slot));
    Check(Traces.size() == 25, "worst-case six stages with first/last/failure plus loss is bounded");
}
int main()
{
    ActualCollectionAndPassiveExport();
    std::puts("PASS: production removal collector/export, DISPATCH-safe evidence and passive WPP");
}
#else
int main()
{
    std::fputs("FAIL: passive removal-stage export is not implemented\n", stderr);
    return 1;
}
#endif
