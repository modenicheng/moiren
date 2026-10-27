// SPDX-License-Identifier: MIT
#include <cstdio>
#include <cstdlib>

#if !__has_include("../src/audio/ClockDiagnostic.h")
int main()
{
    std::fputs("FAIL: first rebase diagnostic helper is not implemented\n", stderr);
    return 1;
}
#else
#include "../src/audio/ClockDiagnostic.h"
#include <cstring>
#include <limits>

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

using namespace vad::audio;

static Clock RunningClock()
{
    return {10000000, 10000000, 0, 480, true};
}

static void PureFirstRebase()
{
    auto clock = RunningClock();
    const auto before = clock;
    const auto schedule = SchedulePacket(clock, 1, 480, 10200023);
    Check(schedule.Rebased, "fixture reaches actual event deadline rebase");
    const auto event = ObserveClockRebase(ClockRebaseReason::EventDeadline,
        before, clock, 10200023, 480, 2, 1, 0);
    Check(event.DeadlineFrame == 960 && event.DeadlineQpc == 10200000 &&
        event.LatenessTicks == 23 && event.CallbackQpc == 10200023,
        "event lateness uses pre-rebase next deadline, not the new epoch");
    Check(event.Before.EpochQpc == 10000000 && event.After.EpochQpc == 10200023 &&
        event.After.BaseFrames == 480 && event.After.LimitFrames == 960,
        "event retains actual clock before and after scheduling");

    ClockDiagnostic diagnostic = {};
    diagnostic.Reset(3);
    ClockEndpointState endpoint = {3, 11, 1, MOIREN_VAD_STREAM_RUNNING};
    auto after = endpoint;
    ++after.GateEpoch;
    diagnostic.Record(77, endpoint, after, event);
    auto first = diagnostic.TakeUnreported();
    Check(first.ReportFirst && first.Stream == 77 && first.Generation == 3 &&
        first.EndpointBefore.GateEpoch == 11 && first.EndpointAfter.GateEpoch == 12 &&
        first.EndpointBefore.GateOpen == 1 && first.EndpointAfter.GateOpen == 1,
        "first observation correlates one unchanged stream with real gate transition");
    Check(!diagnostic.TakeUnreported().ReportFirst,
        "passive publication consumes only pending flag");
    diagnostic.Record(88, after, after, event);
    Check(!diagnostic.ReportFirst && diagnostic.Stream == 77 &&
        diagnostic.First.CallbackQpc == 10200023,
        "later callbacks cannot replace or re-emit the first observation");

    diagnostic.Reset(4);
    diagnostic.Record(77, endpoint, after, event);
    Check(!diagnostic.ReportFirst &&
        diagnostic.First.Reason == ClockRebaseReason::Unobserved,
        "generation reset rejects a retired stream diagnostic");
    endpoint.Generation = after.Generation = 4;
    diagnostic.Record(99, endpoint, after, event);
    Check(diagnostic.ReportFirst && diagnostic.Stream == 99,
        "new generation can retain its own first rebase");

    TimerDrivenRing ring = {};
    Check(InitializeTimerDrivenRing(ring, 1920, 4, 4096), "timer shape");
    ring.PresentedFrames = 480;
    clock = RunningClock();
    const auto timerBefore = clock;
    const auto step = PlanTimerDrivenStep(clock, ring, 10200209);
    Check(step.Valid && step.Rebased && step.Frames == 0, "real timer stall");
    const auto timer = ObserveClockRebase(ClockRebaseReason::TimerStall,
        timerBefore, clock, 10200209, 480, 1, 0, ring.PresentedFrames);
    Check(timer.DeadlineFrame == 960 && timer.DeadlineQpc == 10200000 &&
        timer.LatenessTicks == 209 && timer.PresentedFrames == 480,
        "timer lateness is excess over one logical packet frontier");
    Check(timer.After.BaseFrames == 480 && timer.After.EpochQpc == 10200209 &&
        timer.After.LimitFrames == 528 && timer.PacketCount == 1,
        "timer observation preserves real rebase values and logical shape");
    const auto exact = ObserveClockRebase(ClockRebaseReason::EventDeadline,
        before, before, 10200000, 480, 2, 1, 0);
    const auto early = ObserveClockRebase(ClockRebaseReason::EventDeadline,
        before, before, 10199999, 480, 2, 1, 0);
    Check(exact.LatenessTicks == 0 && early.LatenessTicks == 0,
        "deadline equality and early observations never underflow lateness");
    Check(sizeof(ClockDiagnostic) <= 256, "fixed endpoint diagnostic budget");
}

// These are the production callback and shared-lock bodies, not a second
// scheduling implementation. Only the kernel/ACX/PCM boundaries are stubbed.
using ULONG = uint32_t;
using KIRQL = unsigned;
using VOID = void;
using PUCHAR = unsigned char*;
constexpr bool FALSE = false;
constexpr KIRQL PASSIVE_LEVEL = 0;
constexpr KIRQL DISPATCH_LEVEL = 2;
#ifndef _IRQL_requires_
#define _IRQL_requires_(level)
#endif
#define NT_ASSERT(condition) Check((condition), "production assertion")
struct SpinLock { bool Held = false; };
struct Shared {
    SpinLock Lock;
    vad::Slot Slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    ClockDiagnostic ClockDiagnostics[MOIREN_VAD_MAX_ENDPOINTS] = {};
};
using VAD_SHARED = Shared;
struct Child { Shared* Shared; ULONG Index = 0; uint64_t Id = 5; };
using WDFDEVICE = Child*;
struct VAD_AUDIO_STREAM {
    WDFDEVICE Device;
    SpinLock State;
    uint64_t Generation = 3;
    void* Stream = reinterpret_cast<void*>(77);
    uint64_t Timer = 1;
    void* Packets = this;
    ULONG PacketCount = 2;
    ULONG PacketFrames = 480;
    ULONG PacketSize = 1920;
    ULONG BlockAlign = 4;
    Clock Clock = RunningClock();
    TimerDrivenRing TimerRing = {};
    uint64_t CompletedPackets = 0;
    uint64_t DeliveredPackets = 0;
    uint64_t LastCaptureStart = 0;
    ULONG RenderReadyMask = 3;
    bool Running = true;
    bool Capture = false;
    bool HasEos = false;
    ULONG EosPacket = 0;
    ULONG EosLength = 0;
    unsigned char Buffer[1920] = {};
};
struct TimerContext { VAD_AUDIO_STREAM* Stream; };
using WDFTIMER = TimerContext*;
struct VAD_PCM_BATCH {};
static KIRQL CurrentIrql = PASSIVE_LEVEL;
static uint64_t CallbackQpc = 0;
static unsigned Starts = 0;
static unsigned Notifications = 0;
static unsigned Transfers = 0;
static unsigned Quiesces = 0;
static long long LastDue = 0;
static bool TraceEnabled = false;
static unsigned TraceEvents = 0;
static SpinLock* TraceLock = nullptr;
#define WPP_LEVEL_FLAGS_ENABLED(level, flags) TraceEnabled
constexpr unsigned TRACE_LEVEL_INFORMATION = 4;
constexpr unsigned VAD_CONTROL = 1;
static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
template<typename... Args>
static void VadTrace(unsigned, unsigned, const char*, Args...)
{
    Check(CurrentIrql == PASSIVE_LEVEL && TraceLock != nullptr && !TraceLock->Held,
        "WPP emission is passive and outside the shared lock");
    ++TraceEvents;
}
static Child* VadChild(WDFDEVICE child) { return child; }
static TimerContext* VadAudioTimerContext(WDFTIMER timer) { return timer; }
static void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Check(!lock->Held, "spin lock must not recurse");
    *previous = CurrentIrql;
    CurrentIrql = DISPATCH_LEVEL;
    lock->Held = true;
}
static void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Check(lock->Held, "release owned spin lock");
    lock->Held = false;
    CurrentIrql = previous;
}
struct Counter { uint64_t QuadPart; };
static Counter KeQueryPerformanceCounter(void*) { return {CallbackQpc}; }
static bool VadAudioIsStreamCurrent(WDFDEVICE child, uint64_t generation)
{
    return child->Shared->Slots[0].StreamGeneration == generation;
}
static void QuiesceStreamBuffers(VAD_AUDIO_STREAM*) { ++Quiesces; }
static void ZeroPackets(VAD_AUDIO_STREAM*) {}
static unsigned char* PacketAddress(VAD_AUDIO_STREAM* stream, ULONG) { return stream->Buffer; }
static void RtlZeroMemory(void* address, ULONG size) { std::memset(address, 0, size); }
static void TransferTimerSegment(VAD_AUDIO_STREAM*, const TimerDrivenSegment&,
    uint64_t, VAD_PCM_BATCH*) { ++Transfers; }
static void VadPcmTransfer(WDFDEVICE, uint64_t, void*, ULONG, ULONG,
    uint64_t, uint64_t, VAD_PCM_BATCH*) { ++Transfers; }
static void VadPcmComplete(VAD_PCM_BATCH*)
{
    Check(CurrentIrql == PASSIVE_LEVEL, "PCM completion stays outside spin locks");
}
static int WdfTimerStart(uint64_t, long long due) { ++Starts; LastDue = due; return 0; }
static int AcxRtStreamNotifyPacketComplete(void*, uint64_t, uint64_t)
{
    Check(CurrentIrql == PASSIVE_LEVEL, "ACX notification stays outside spin locks");
    ++Notifications;
    return 0;
}
static void Schedule(VAD_AUDIO_STREAM*, uint64_t) { ++Starts; }
static void VadAudioRebaseDiscontinuity(WDFDEVICE child, uint64_t generation,
    uint64_t stream, const ClockRebaseObservation& observation);
#include "clock-production.inc"

static void ProductionCallSites()
{
    Shared shared = {};
    Child child = {&shared};
    auto& slot = shared.Slots[0];
    slot.Id = 5;
    slot.Session = 9;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = 1;
    slot.GateOpen = slot.AudioReady = 1;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    slot.HasSequence = 1;
    shared.ClockDiagnostics[0].Reset(3);
    VAD_AUDIO_STREAM stream = {};
    stream.Device = &child;
    TimerContext timer = {&stream};
    CallbackQpc = 10200023;
    VadAudioTimer(&timer);
    const auto event = shared.ClockDiagnostics[0].TakeUnreported();
    Check(event.ReportFirst && event.First.Reason == ClockRebaseReason::EventDeadline &&
        event.First.LatenessTicks == 23 && event.First.CompletedPackets == 1 &&
        event.First.PacketCount == 2 && event.Stream == 77,
        "actual event callback records the rebase that advances the epoch");
    Check(slot.AdmissionGeneration == 2 && slot.HasSequence == 0 &&
        event.EndpointBefore.GateEpoch == 1 && event.EndpointAfter.GateEpoch == 2 &&
        event.EndpointBefore.Generation == 3 && event.EndpointAfter.Generation == 3 &&
        event.EndpointBefore.GateOpen == 1 && event.EndpointAfter.GateOpen == 1 &&
        event.EndpointBefore.StreamState == MOIREN_VAD_STREAM_RUNNING &&
        event.EndpointAfter.StreamState == MOIREN_VAD_STREAM_RUNNING,
        "real shared-lock invalidation correlates Running/open epoch 1 to 2");
    Check(stream.CompletedPackets == 1 && Starts == 1 && Notifications == 1 &&
        Transfers == 0 && LastDue == -100000 && !shared.Lock.Held && !stream.State.Held,
        "event diagnostics do not add scheduling, transfers or notifications");
    CallbackQpc += 200001;
    VadAudioTimer(&timer);
    Check(slot.AdmissionGeneration == 3 && !shared.ClockDiagnostics[0].ReportFirst &&
        shared.ClockDiagnostics[0].First.CallbackQpc == 10200023,
        "next real rebase still invalidates PCM but cannot overwrite first evidence");

    slot.StreamGeneration = 4;
    slot.AdmissionGeneration = 11;
    slot.HasSequence = 1;
    shared.ClockDiagnostics[0].Reset(4);
    stream.Generation = 4;
    stream.PacketCount = 1;
    stream.CompletedPackets = 0;
    stream.Clock = RunningClock();
    Check(InitializeTimerDrivenRing(stream.TimerRing, 1920, 4, 4096), "production timer ring");
    CallbackQpc = 10100209;
    Starts = Notifications = Transfers = Quiesces = 0;
    VadAudioTimer(&timer);
    const auto stalled = shared.ClockDiagnostics[0].TakeUnreported();
    Check(stalled.ReportFirst && stalled.First.Reason == ClockRebaseReason::TimerStall &&
        stalled.First.LatenessTicks == 209 && stalled.First.PresentedFrames == 0 &&
        stalled.First.Before.EpochQpc == 10000000 &&
        stalled.First.After.EpochQpc == CallbackQpc &&
        stalled.EndpointBefore.GateEpoch == 11 && stalled.EndpointAfter.GateEpoch == 12,
        "actual timer planner and invalidation retain the first stalled callback");
    Check(Starts == 1 && Notifications == 0 && Transfers == 0 && Quiesces == 1 &&
        LastDue == -10000 && stream.TimerRing.PresentedFrames == 0,
        "timer diagnostics preserve existing no-catch-up schedule and quiescence");

    const auto old = slot.AdmissionGeneration;
    VadAudioRebaseDiscontinuity(&child, 3, 77, event.First);
    Check(slot.AdmissionGeneration == old, "retired generation cannot invalidate or record");
    shared.ClockDiagnostics[0].Reset(4);
    slot.AdmissionGeneration = std::numeric_limits<uint64_t>::max();
    slot.GateOpen = 1;
    VadAudioRebaseDiscontinuity(&child, 4, 77, stalled.First);
    Check(shared.ClockDiagnostics[0].First.Reason == ClockRebaseReason::TimerStall &&
        shared.ClockDiagnostics[0].EndpointBefore.GateOpen == 1 &&
        shared.ClockDiagnostics[0].EndpointAfter.GateOpen == 0 &&
        slot.AdmissionGeneration == std::numeric_limits<uint64_t>::max(),
        "epoch exhaustion retains the actual fail-closed gate transition");

    TraceLock = &shared.Lock;
    auto snapshot = vad::Snapshot(slot);
    TraceEnabled = false;
    TracePendingClockDiagnostic(&shared, snapshot);
    Check(TraceEvents == 0 && shared.ClockDiagnostics[0].ReportFirst,
        "disabled passive trace must not consume pending evidence");
    TraceEnabled = true;
    ++snapshot.StreamGeneration;
    TracePendingClockDiagnostic(&shared, snapshot);
    Check(TraceEvents == 0 && shared.ClockDiagnostics[0].ReportFirst,
        "passive query cannot misattribute evidence to a different generation");
    --snapshot.StreamGeneration;
    TracePendingClockDiagnostic(&shared, snapshot);
    Check(TraceEvents == 1 && !shared.ClockDiagnostics[0].ReportFirst &&
        shared.ClockDiagnostics[0].First.LatenessTicks == stalled.First.LatenessTicks,
        "actual control body emits one event after unlock and retains first evidence");
    TracePendingClockDiagnostic(&shared, snapshot);
    Check(TraceEvents == 1, "repeated passive query does not re-emit first rebase");
}

int main()
{
    PureFirstRebase();
    ProductionCallSites();
    std::printf("PASS: first event/timer rebase, real callback/epoch causality, "
        "lateness, generation reset, passive WPP; fixed diagnostic=%zu bytes\n", sizeof(ClockDiagnostic));
}
#endif
