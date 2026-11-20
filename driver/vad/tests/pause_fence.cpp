// SPDX-License-Identifier: MIT
#include "../src/Lifecycle.h"
#include "../src/audio/TimerPcm.h"
#include "../src/UnservedDiagnostics.h"
#include <cinttypes>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <mutex>
#include <thread>
#include <type_traits>

// Compile real Stop/Pause, DescribeStream, Discontinuity, Purge and CancelQueued.
// Only WDF/kernel edges are substituted; this does not model kernel concurrency.
using ULONG = uint32_t;
using LONG = int32_t;
using NTSTATUS = int32_t;
using KIRQL = unsigned;
using PVOID = void*;
using PUCHAR = unsigned char*;
constexpr bool FALSE = false;
constexpr bool TRUE = true;
constexpr KIRQL PASSIVE_LEVEL = 0;
constexpr KIRQL DISPATCH_LEVEL = 2;
constexpr unsigned Executive = 0;
constexpr unsigned KernelMode = 0;
constexpr NTSTATUS STATUS_SUCCESS = 0;
constexpr NTSTATUS STATUS_CANCELLED = static_cast<NTSTATUS>(0xc0000120u);
constexpr NTSTATUS STATUS_DEVICE_REMOVED = -1;
constexpr NTSTATUS STATUS_REVISION_MISMATCH = -2;
constexpr NTSTATUS STATUS_INVALID_PARAMETER = -3;
constexpr NTSTATUS STATUS_QUOTA_EXCEEDED = -4;
constexpr NTSTATUS STATUS_INFO_LENGTH_MISMATCH = -5;
constexpr NTSTATUS STATUS_INSUFFICIENT_RESOURCES = -6;
constexpr NTSTATUS STATUS_NOT_FOUND = -7;
constexpr NTSTATUS STATUS_PENDING = 0x103;
constexpr unsigned NormalPagePriority = 0;
constexpr unsigned MdlMappingNoExecute = 0;
constexpr unsigned MdlMappingNoWrite = 0;
#define NT_SUCCESS(status) ((status) >= 0)
#define VAD_PAGE
#define _IRQL_requires_(level)

static void Require(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FIXTURE FAILURE: %s\n", message);
        std::exit(2);
    }
}

static unsigned Failures = 0;
static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        ++Failures;
    }
}

// Covers Stop and join too: no broken lock/handshake may hang the host suite.
class SuiteWatchdog {
public:
    SuiteWatchdog() : Worker([this] {
        std::unique_lock<std::mutex> lock(Lock);
        if (!Changed.wait_for(lock, std::chrono::seconds(30), [this] { return Finished; })) {
            std::fputs("FIXTURE FAILURE: pause-fence suite exceeded thirty seconds\n", stderr);
            std::_Exit(2);
        }
    }) {}
    ~SuiteWatchdog()
    {
        {
            std::lock_guard<std::mutex> lock(Lock);
            Finished = true;
        }
        Changed.notify_one();
        Worker.join();
    }
    SuiteWatchdog(const SuiteWatchdog&) = delete;
    SuiteWatchdog& operator=(const SuiteWatchdog&) = delete;
private:
    std::mutex Lock;
    std::condition_variable Changed;
    bool Finished = false;
    std::thread Worker;
};

static thread_local KIRQL CurrentIrql = PASSIVE_LEVEL;
static thread_local unsigned CriticalDepth = 0;
static bool RecordGuardOrder = false;
static char GuardOrder[16] = {};
static unsigned GuardOrderCount = 0;
static void RecordOrder(char event)
{
    if (RecordGuardOrder) {
        Require(GuardOrderCount + 1 < sizeof(GuardOrder), "bounded guard lifecycle trace");
        GuardOrder[GuardOrderCount++] = event;
        GuardOrder[GuardOrderCount] = '\0';
    }
}
static KIRQL KeGetCurrentIrql() { return CurrentIrql; }
#define NT_ASSERT(condition) Require((condition), "production assertion")
#define PAGED_CODE() Require(CurrentIrql == PASSIVE_LEVEL, "pageable routine is passive")
#define UNREFERENCED_PARAMETER(value) (void)(value)

struct SpinLock { std::timed_mutex Native; std::atomic<bool> Held{false}; };
struct Mutex { std::timed_mutex Native; std::atomic<bool> Held{false}; };
struct Rundown {
    std::atomic<unsigned> Users{0};
    bool Denied = false;
    std::atomic<unsigned> Attempts{0};
};
struct AudioDevice { bool LifecycleHeld = false; };
using VAD_AUDIO_DEVICE = AudioDevice;
struct VAD_PCM_REQUEST;
using WDFREQUEST = VAD_PCM_REQUEST*;
struct Queue {
    WDFREQUEST Pending[MOIREN_VAD_REQUEST_SLOTS] = {};
    unsigned Count = 0;
    unsigned Purges = 0;
    unsigned Starts = 0;
};
using WDFQUEUE = Queue*;
struct VAD_SHARED {
    SpinLock Lock;
    vad::Slot Slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    vad::UnservedDiagnostics UnservedDiagnostics[MOIREN_VAD_MAX_ENDPOINTS] = {};
    bool PcmInitialized = true;
    Rundown PcmRundown;
    Mutex PcmDrain[MOIREN_VAD_MAX_ENDPOINTS];
    WDFQUEUE PcmQueues[MOIREN_VAD_MAX_ENDPOINTS] = {};
    volatile LONG PcmOnline = 1;
};
struct Child {
    VAD_SHARED* Shared;
    AudioDevice* Audio;
    ULONG Index = 0;
    uint64_t Id = 11;
    volatile LONG ActiveStreams = 1;
};
using WDFDEVICE = Child*;
struct HostTimer { unsigned Stops = 0; };
struct Packet { ULONG RtPacketOffset = 0; };
struct VAD_AUDIO_STREAM {
    WDFDEVICE Device = nullptr;
    SpinLock State;
    HostTimer* Timer = nullptr;
    Packet* Packets = nullptr;
    PVOID Buffers[2] = {};
    ULONG PacketCount = 2;
    ULONG PacketSize = 1920;
    ULONG PacketFrames = 480;
    ULONG BlockAlign = 4;
    vad::audio::TimerDrivenRing TimerRing = {};
    vad::audio::TimerPcmPacket TimerPacket = {};
    PVOID TimerScratch = nullptr;
    ULONG TimerScratchBytes = 0;
    uint64_t Generation = 3;
    vad::audio::Clock Clock = {10000000, 10000000, 960, 1440, true};
    uint64_t CompletedPackets = 3;
    uint64_t DeliveredPackets = 2;
    uint64_t LastCaptureStart = 9990000;
    ULONG RenderReadyMask = 3;
    bool Running = true;
    bool Started = true;
    bool HasEos = true;
};
using ACXSTREAM = VAD_AUDIO_STREAM*;
struct VAD_PCM_REQUEST {
    MOIREN_VAD_PCM_HEADER Input = {};
    MOIREN_VAD_PCM_HEADER Header = {};
    unsigned char Bytes[1920 + sizeof(MOIREN_VAD_PCM_HEADER)] = {};
    ULONG Code = 0;
    PVOID Buffer = nullptr;
    VAD_SHARED* Shared = nullptr;
    ULONG Index = 0;
    volatile LONG Counted = 1;
    unsigned Completions = 0;
    NTSTATUS Status = STATUS_SUCCESS;
    MOIREN_VAD_ENDPOINT_SNAPSHOT AtCompletion = {};
};

static Child* VadChild(WDFDEVICE child) { return child; }
static Child* VadRoot(WDFDEVICE child) { return child; }
static AudioDevice* VadAudioDevice(WDFDEVICE child) { return child->Audio; }
static VAD_AUDIO_STREAM* VadAudioStream(ACXSTREAM stream) { return stream; }
static VAD_PCM_REQUEST* VadPcmRequest(WDFREQUEST request) { return request; }
static SpinLock* PublicationState = nullptr;
static bool TimerDrained = false;
static void (*AfterPublication)() = nullptr;
static void VadAudioLock(AudioDevice* device)
{
    PAGED_CODE();
    Require(!device->LifecycleHeld, "lifecycle does not recurse");
    device->LifecycleHeld = true;
}
static void VadAudioUnlock(AudioDevice* device)
{
    Require(device->LifecycleHeld, "lifecycle unlock is paired");
    device->LifecycleHeld = false;
}
static void KeAcquireSpinLock(SpinLock* lock, KIRQL* previous)
{
    Require(lock->Native.try_lock_for(std::chrono::seconds(5)),
        "spin-lock fixture acquisition timed out (possible recursion)");
    *previous = CurrentIrql;
    CurrentIrql = DISPATCH_LEVEL;
    lock->Held = true;
}
static void KeReleaseSpinLock(SpinLock* lock, KIRQL previous)
{
    Require(lock->Held, "spin lock unlock is paired");
    lock->Held = false;
    CurrentIrql = previous;
    lock->Native.unlock();
    if (lock == PublicationState && TimerDrained && AfterPublication != nullptr) {
        const auto interleave = AfterPublication;
        AfterPublication = nullptr;
        interleave();
    }
}
static LONG InterlockedExchange(volatile LONG* value, LONG next)
{
    const auto previous = *value;
    *value = next;
    return previous;
}
static LONG InterlockedCompareExchange(volatile LONG* value, LONG next, LONG expected)
{
    const auto previous = *value;
    if (previous == expected) {
        *value = next;
    }
    return previous;
}
struct Counter { uint64_t QuadPart; };
static Counter KeQueryPerformanceCounter(void*) { return {10050000}; }
static bool WdfTimerStop(HostTimer* timer, bool wait)
{
    PAGED_CODE();
    Require(wait, "stop drains the timer before purge");
    ++timer->Stops;
    TimerDrained = true;
    return true;
}
static void RtlZeroMemory(PVOID address, size_t bytes) { std::memset(address, 0, bytes); }
static void RtlCopyMemory(PVOID destination, const void* source, size_t bytes)
{ std::memcpy(destination, source, bytes); }
static bool ExAcquireRundownProtection(Rundown* rundown)
{
    RecordOrder('R');
    ++rundown->Attempts;
    if (rundown->Denied) {
        return false;
    }
    ++rundown->Users;
    return true;
}
static void ExReleaseRundownProtection(Rundown* rundown)
{
    RecordOrder('r');
    Require(rundown->Users != 0, "rundown reference is paired");
    --rundown->Users;
}
static Mutex* InterleaveMutex = nullptr;
static void (*BeforeDrainWait)() = nullptr;
static thread_local Mutex* OwnedDrain = nullptr;
static thread_local bool FreshSubmitWorker = false;
static std::mutex InterleaveSync;
static std::condition_variable InterleaveSignal;
static bool FreshSubmitWaiting = false;
static bool FreshSubmitBlocked = false;
static bool FreshSubmitFinished = false;
static void KeEnterCriticalRegion() { PAGED_CODE(); ++CriticalDepth; RecordOrder('C'); }
static void KeLeaveCriticalRegion()
{
    PAGED_CODE();
    Require(CriticalDepth != 0 && OwnedDrain == nullptr,
        "critical region ends only after mutex/rundown release");
    --CriticalDepth;
    RecordOrder('c');
}
static void KeWaitForSingleObject(Mutex* mutex, unsigned, unsigned, bool, void*)
{
    PAGED_CODE();
    RecordOrder('M');
    if (mutex == InterleaveMutex && BeforeDrainWait != nullptr) {
        const auto interleave = BeforeDrainWait;
        BeforeDrainWait = nullptr;
        interleave();
    }
    if (mutex == InterleaveMutex && FreshSubmitWorker) {
        std::lock_guard<std::mutex> lock(InterleaveSync);
        FreshSubmitBlocked = mutex->Held;
        FreshSubmitWaiting = true;
        InterleaveSignal.notify_one();
    }
    Require(OwnedDrain != mutex, "PCM drain does not recurse");
    Require(mutex->Native.try_lock_for(std::chrono::seconds(5)),
        "PCM drain fixture acquisition timed out");
    mutex->Held = true;
    OwnedDrain = mutex;
}
static void KeReleaseMutex(Mutex* mutex, bool)
{
    Require(mutex->Held && OwnedDrain == mutex, "PCM drain unlock is paired");
    mutex->Held = false;
    RecordOrder('m');
    OwnedDrain = nullptr;
    mutex->Native.unlock();
}
static void CancelQueued(WDFQUEUE queue, WDFREQUEST request);
static void WdfIoQueuePurgeSynchronously(WDFQUEUE queue)
{
    PAGED_CODE();
    RecordOrder('P');
    Require(CriticalDepth != 0, "guard keeps normal kernel APCs disabled through purge");
    ++queue->Purges;
    while (queue->Count != 0) {
        auto request = queue->Pending[0];
        Require(request->Shared->PcmDrain[request->Index].Held,
            "real purge holds PCM drain through cancellation");
        for (unsigned i = 1; i < queue->Count; ++i) {
            queue->Pending[i - 1] = queue->Pending[i];
        }
        --queue->Count;
        CancelQueued(queue, request);
    }
}
static void WdfIoQueueStart(WDFQUEUE queue) { RecordOrder('S'); ++queue->Starts; }
static void WdfRequestComplete(WDFREQUEST request, NTSTATUS status)
{
    auto shared = request->Shared;
    Require(!shared->Lock.Held && CurrentIrql == PASSIVE_LEVEL,
        "completion can observe QUERY outside the shared lock");
    Require(request->Counted == 0,
        "real cancellation removes pending count before completion");
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    request->AtCompletion = vad::Snapshot(shared->Slots[request->Index]);
    KeReleaseSpinLock(&shared->Lock, irql);
    request->Status = status;
    ++request->Completions;
}

using PMDL = void*;
struct WDF_REQUEST_PARAMETERS {
    struct { struct { ULONG InputBufferLength; ULONG OutputBufferLength; }
        DeviceIoControl; } Parameters;
};
static void WDF_REQUEST_PARAMETERS_INIT(WDF_REQUEST_PARAMETERS*) {}
static void WdfRequestGetParameters(WDFREQUEST request, WDF_REQUEST_PARAMETERS* parameters)
{
    parameters->Parameters.DeviceIoControl.InputBufferLength = sizeof(request->Input);
    parameters->Parameters.DeviceIoControl.OutputBufferLength = vad::PcmBytes(request->Input) +
        (request->Code == IOCTL_MOIREN_VAD_WRITE_PCM ? 0 : sizeof(request->Input));
}
static NTSTATUS WdfRequestRetrieveInputBuffer(WDFREQUEST request, size_t, void** input, void*)
{ *input = &request->Input; return STATUS_SUCCESS; }
static NTSTATUS WdfRequestRetrieveOutputWdmMdl(WDFREQUEST request, PMDL* mdl)
{ *mdl = request->Bytes; return STATUS_SUCCESS; }
static PVOID MmGetSystemAddressForMdlSafe(PMDL mdl, unsigned) { return mdl; }
static NTSTATUS WdfRequestForwardToIoQueue(WDFREQUEST request, WDFQUEUE queue)
{
    Require(request->Shared->PcmDrain[request->Index].Held &&
        !request->Shared->Lock.Held && CurrentIrql == PASSIVE_LEVEL,
        "real submit forwards under drain without a spin lock");
    Require(queue->Count < MOIREN_VAD_REQUEST_SLOTS, "bounded fixture queue");
    queue->Pending[queue->Count++] = request;
    return STATUS_SUCCESS;
}

#include "pause-fence-production.inc"

static_assert(!std::is_copy_constructible<VadPcmDrainGuard>::value &&
    !std::is_copy_assignable<VadPcmDrainGuard>::value &&
    !std::is_move_constructible<VadPcmDrainGuard>::value,
    "drain ownership cannot be copied or moved");

static void PrepareSubmission(VAD_PCM_REQUEST& request,
    const MOIREN_VAD_ENDPOINT_SNAPSHOT& snapshot, uint64_t sequence)
{
    request = {};
    request.Input = {sizeof(request.Input), MOIREN_VAD_ABI_VERSION,
        snapshot.EndpointId, snapshot.SessionEpoch, snapshot.EndpointGeneration,
        snapshot.StreamGeneration, snapshot.GateEpoch, sequence, 10050000,
        snapshot.SampleFormat, snapshot.PacketFrames, snapshot.Channels, 0};
    request.Code = snapshot.Intent == MOIREN_VAD_INTENT_MICROPHONE ?
        IOCTL_MOIREN_VAD_WRITE_PCM : IOCTL_MOIREN_VAD_READ_PCM;
    request.Counted = 0;
}

static unsigned Scenarios = 0;
static void StopCancellation(ULONG intent, bool timerDriven, bool reset,
    uint64_t initialEpoch = 5, unsigned repetitions = 1)
{
    ++Scenarios;
    const auto name = timerDriven ? "timer" : "event";
    VAD_SHARED shared = {};
    auto& slot = shared.Slots[0];
    slot.Id = 11;
    slot.Session = 7;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = initialEpoch;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = intent;
    slot.Channels = 2;
    slot.PolicyPrepared = slot.AudioReady = slot.GateOpen = 1;
    slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    slot.PacketFrames = 480;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    slot.HasSequence = 1;
    slot.LastSequence = 123;
    AudioDevice audio = {};
    Child child = {&shared, &audio};
    VAD_PCM_REQUEST request = {};
    Queue queue = {};
    shared.PcmQueues[0] = &queue;
    HostTimer timer = {};
    Packet packets[2] = {};
    unsigned char buffers[2][4096];
    unsigned char scratch[1920];
    VAD_AUDIO_STREAM stream = {};
    stream.Device = &child;
    stream.Timer = &timer;
    stream.Packets = packets;
    stream.Buffers[0] = buffers[0];
    stream.Buffers[1] = buffers[1];
    stream.TimerScratch = scratch;
    stream.TimerScratchBytes = sizeof(scratch);
    Require(vad::audio::InitializeTimerDrivenRing(stream.TimerRing, 1920, 4, 4096),
        "real timer ring fixture is valid");
    stream.TimerRing.PresentedFrames = 1200;
    stream.TimerRing.RingFrameOffset = 128;
    stream.TimerRing.SegmentSequence = 25;
    if (timerDriven) {
        stream.PacketCount = 1;
    }
    for (unsigned iteration = 0; iteration < repetitions; ++iteration) {
        std::memset(buffers, 0x55, sizeof(buffers));
        std::memset(scratch, 0x66, sizeof(scratch));
        stream.TimerPacket = {slot.Session, slot.AdmissionGeneration, true, true, 123};
        const auto previous = vad::Snapshot(slot);
        PrepareSubmission(request, previous, 124 + iteration);
        Require(VadPcmSubmit(&child, &request, request.Code) == STATUS_PENDING,
            "real submit queues PCM, including while already Paused");
        if (reset) {
            LifecycleGuard lifecycle(&audio);
            VadAudioStopLocked(&stream, true);
        } else {
            Require(VadAudioStreamPause(&stream) == STATUS_SUCCESS, "real Pause returns success");
        }
        const auto& observed = request.AtCompletion;
        std::printf("case=%s-%s-%s iteration=%u completions=%u status=%08x stream=%" PRIu64 "->%" PRIu64 " "
            "gate=%" PRIu64 "->%" PRIu64 " open=%u state=%u last_capture=%" PRIu64
            " completed=%" PRIu64 "\n",
            intent == MOIREN_VAD_INTENT_MICROPHONE ? "microphone" : "playback",
            name, reset ? "reset" : "pause", iteration + 1, request.Completions,
            static_cast<unsigned>(request.Status), previous.StreamGeneration,
            observed.StreamGeneration, previous.GateEpoch, observed.GateEpoch,
            observed.GateOpen, observed.StreamState, stream.LastCaptureStart, stream.CompletedPackets);
        Check(request.Completions == 1 && request.Status == STATUS_CANCELLED,
            "one real CancelQueued terminal cancellation");
        Check(observed.StreamState == MOIREN_VAD_STREAM_PAUSED,
            "Paused is published before cancellation completion");
        Check(observed.GateEpoch != previous.GateEpoch || observed.GateOpen == 0,
            "PCM cancellation must expose an epoch fence or closed gate before completion");
        const auto maximumEpoch = ~uint64_t{0};
        Check(observed.GateEpoch == (previous.GateEpoch == maximumEpoch ?
                maximumEpoch : previous.GateEpoch + 1) &&
            observed.GateOpen == (previous.GateEpoch == maximumEpoch ? 0u : 1u) &&
            slot.HasSequence == 0,
            "epoch advances without wrap, exhausted gate fails closed, sequence is invalidated");
        Check(observed.StreamGeneration == previous.StreamGeneration &&
            observed.SessionEpoch == previous.SessionEpoch && observed.EndpointId == previous.EndpointId,
            "stop does not fabricate a new stream or endpoint identity");
        Check(timer.Stops == iteration + 1 && queue.Purges == iteration + 1 &&
            queue.Starts == iteration + 1 && queue.Count == 0 && slot.PendingRequests == 0 &&
            !audio.LifecycleHeld && !stream.State.Held && !shared.Lock.Held &&
            !shared.PcmDrain[0].Held && shared.PcmRundown.Users == 0,
            "timer drain, purge restart and all lock lifetimes are paired");
        Check(!stream.Running && !stream.Clock.Running && stream.RenderReadyMask == 0 &&
            stream.DeliveredPackets == stream.CompletedPackets && !stream.TimerPacket.Active,
            "real stop clears pending packet state");
        Check(stream.LastCaptureStart == (reset ? 0u : 9990000u) &&
            stream.CompletedPackets == (reset ? 0u : 3u) && stream.Started == !reset &&
            stream.Clock.BaseFrames == (reset ? 0u : 1200u) &&
            stream.Clock.EpochQpc == (reset ? 0u : 10000000u) &&
            stream.Clock.Frequency == 10000000 &&
            stream.Clock.LimitFrames == (reset ? 480u : 1440u) && stream.HasEos == !reset,
            "ordinary pause preserves capture time and numbering; reset clears them");
        Check(stream.TimerRing.PresentedFrames == (reset ? 0u : 1200u) &&
            stream.TimerRing.RingFrameOffset == (reset ? 0u : 128u) &&
            stream.TimerRing.SegmentSequence == (reset ? 0u : 25u),
            "ordinary pause does not reset timer-ring position");
        for (ULONG i = 0; i < stream.PacketCount; ++i) {
            const auto bytes = timerDriven ? stream.TimerRing.MappedBytes : stream.PacketSize;
            for (ULONG offset = 0; offset < bytes; ++offset) {
                Require(buffers[i][offset] == 0, "real stop clears mapped PCM bytes");
            }
        }
        for (const auto value : scratch) {
            Require(value == 0, "real stop clears scratch PCM bytes");
        }
    }
}

static Child* InterleaveChild = nullptr;
static VAD_PCM_REQUEST* InterleaveRequest = nullptr;
static NTSTATUS InterleaveSubmitStatus = STATUS_SUCCESS;
static std::thread FreshThread;
static void SubmitFreshQueryStamp()
{
    auto shared = InterleaveChild->Shared;
    KIRQL irql;
    KeAcquireSpinLock(&shared->Lock, &irql);
    const auto snapshot = vad::Snapshot(shared->Slots[0]);
    KeReleaseSpinLock(&shared->Lock, irql);
    auto& request = *InterleaveRequest;
    PrepareSubmission(request, snapshot, 124);
    InterleaveSubmitStatus = VadPcmSubmit(InterleaveChild, &request, request.Code);
}

static void StartFreshSubmitAfterPublication()
{
    FreshThread = std::thread([] {
        FreshSubmitWorker = true;
        SubmitFreshQueryStamp();
        std::lock_guard<std::mutex> lock(InterleaveSync);
        FreshSubmitFinished = true;
        InterleaveSignal.notify_one();
    });
    std::unique_lock<std::mutex> lock(InterleaveSync);
    Require(InterleaveSignal.wait_for(lock, std::chrono::seconds(5),
            [] { return FreshSubmitWaiting; }),
        "fresh submit did not reach the drain within five seconds");
}

static void FreshStampAfterPublication(ULONG intent, bool timerDriven, bool reset)
{
    ++Scenarios;
    VAD_SHARED shared = {};
    auto& slot = shared.Slots[0];
    slot.Id = 11;
    slot.Session = 7;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = 5;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = intent;
    slot.Channels = 2;
    slot.PolicyPrepared = slot.AudioReady = slot.GateOpen = 1;
    slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    slot.PacketFrames = 480;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    AudioDevice audio = {};
    Child child = {&shared, &audio};
    VAD_PCM_REQUEST request = {};
    Queue queue = {};
    shared.PcmQueues[0] = &queue;
    HostTimer timer = {};
    VAD_AUDIO_STREAM stream = {};
    stream.Device = &child;
    stream.Timer = &timer;
    stream.PacketCount = timerDriven ? 1 : 2;
    InterleaveChild = &child;
    InterleaveRequest = &request;
    InterleaveMutex = &shared.PcmDrain[0];
    PublicationState = &stream.State;
    TimerDrained = FreshSubmitWaiting = FreshSubmitBlocked = FreshSubmitFinished = false;
    AfterPublication = StartFreshSubmitAfterPublication;
    {
        LifecycleGuard lifecycle(&audio);
        VadAudioStopLocked(&stream, reset);
    }
    Require(FreshThread.joinable(), "actual Stop reached its post-fence State unlock");
    {
        std::unique_lock<std::mutex> lock(InterleaveSync);
        Require(InterleaveSignal.wait_for(lock, std::chrono::seconds(5),
                [] { return FreshSubmitFinished; }),
            "Stop did not release the fresh submit before its bounded join");
    }
    FreshThread.join();
    std::printf("case=%s-%s-%s-fresh-after-publication blocked=%u submit=%08x "
        "submitted_gate=%" PRIu64 " completions=%u queued=%u\n",
        intent == MOIREN_VAD_INTENT_MICROPHONE ? "microphone" : "playback",
        timerDriven ? "timer" : "event", reset ? "reset" : "pause",
        FreshSubmitBlocked ? 1u : 0u, static_cast<unsigned>(InterleaveSubmitStatus),
        request.Header.GateEpoch, request.Completions, queue.Count);
    Check(FreshSubmitBlocked && InterleaveSubmitStatus == STATUS_PENDING &&
        request.Header.GateEpoch == 6 && request.Completions == 0 && queue.Count == 1,
        "fresh epoch submit waits through purge/restart and remains queued afterwards");
    PublicationState = nullptr;
    InterleaveChild = nullptr;
    InterleaveRequest = nullptr;
    InterleaveMutex = nullptr;
    // Retire the queued request only after assertions, using another real fence.
    VadPcmDiscontinuity(&child, stream.Generation, true);
    VadPcmPurge(&shared, 0, true);
    Require(request.Completions == 1 && request.AtCompletion.GateEpoch == 7 &&
        shared.PcmRundown.Users == 0 && CriticalDepth == 0,
        "fixture teardown uses its own fenced drain with balanced ownership");
}

static void FreshStampResetInterleave(ULONG intent, bool timerDriven)
{
    ++Scenarios;
    VAD_SHARED shared = {};
    auto& slot = shared.Slots[0];
    slot.Id = 11;
    slot.Session = 7;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = 5;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = intent;
    slot.Channels = 2;
    slot.PolicyPrepared = slot.AudioReady = slot.GateOpen = 1;
    slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    slot.PacketFrames = 480;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    AudioDevice audio = {};
    Child child = {&shared, &audio};
    VAD_PCM_REQUEST request = {};
    Queue queue = {};
    shared.PcmQueues[0] = &queue;
    HostTimer timer = {};
    Packet packets[2] = {};
    unsigned char buffers[2][4096] = {};
    VAD_AUDIO_STREAM stream = {};
    stream.Device = &child;
    stream.Timer = &timer;
    stream.Packets = packets;
    stream.Buffers[0] = buffers[0];
    stream.Buffers[1] = buffers[1];
    Require(vad::audio::InitializeTimerDrivenRing(stream.TimerRing, 1920, 4, 4096),
        "interleave uses a real event/timer packet shape");
    stream.PacketCount = timerDriven ? 1 : 2;
    InterleaveChild = &child;
    InterleaveRequest = &request;
    InterleaveMutex = &shared.PcmDrain[0];
    BeforeDrainWait = SubmitFreshQueryStamp;
    {
        LifecycleGuard lifecycle(&audio);
        VadAudioStopLocked(&stream, true);
    }
    std::printf("case=%s-%s-fresh-stamp-reset-interleave submit=%08x submitted_gate=%" PRIu64
        " completion_gate=%" PRIu64 " open=%u completions=%u status=%08x\n",
        intent == MOIREN_VAD_INTENT_MICROPHONE ? "microphone" : "playback",
        timerDriven ? "timer" : "event", static_cast<unsigned>(InterleaveSubmitStatus),
        request.Header.GateEpoch,
        request.AtCompletion.GateEpoch, request.AtCompletion.GateOpen,
        request.Completions, static_cast<unsigned>(request.Status));
    Require(BeforeDrainWait == nullptr && InterleaveSubmitStatus == STATUS_PENDING,
        "real submit completes before Stop obtains the submission drain");
    Check(request.Completions == 0 || request.Status != STATUS_CANCELLED ||
        request.Header.GateEpoch != request.AtCompletion.GateEpoch ||
        request.AtCompletion.GateOpen == 0,
        "fresh-stamp PCM must not be cancelled by the same drain with unchanged open epoch");
    Require(shared.Slots[0].PendingRequests == 0 && shared.PcmRundown.Users == 0,
        "real cancellation balances the interleaved submit");
    InterleaveChild = nullptr;
    InterleaveRequest = nullptr;
    InterleaveMutex = nullptr;
}

static void ReleaseAfterStop(ULONG intent, bool timerDriven)
{
    ++Scenarios;
    VAD_SHARED shared = {};
    auto& slot = shared.Slots[0];
    slot.Id = 11;
    slot.Session = 7;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = 5;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.Intent = intent;
    slot.Channels = 2;
    slot.PolicyPrepared = slot.AudioReady = slot.GateOpen = 1;
    slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    slot.PacketFrames = 480;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    AudioDevice audio = {};
    Child child = {&shared, &audio};
    Queue queue = {};
    shared.PcmQueues[0] = &queue;
    HostTimer timer = {};
    VAD_AUDIO_STREAM stream = {};
    stream.Device = &child;
    stream.Timer = &timer;
    stream.PacketCount = timerDriven ? 1 : 2;
    {
        LifecycleGuard lifecycle(&audio);
        VadAudioStopLocked(&stream, true);
    }
    VAD_PCM_REQUEST request = {};
    PrepareSubmission(request, vad::Snapshot(slot), 125);
    Require(VadPcmSubmit(&child, &request, request.Code) == STATUS_PENDING,
        "a fresh request is genuinely admitted after Stop while Paused");
    VadAudioReleaseStream(&child);
    std::printf("case=%s-%s-release-after-stop submitted_gate=%" PRIu64
        " completion_gate=%" PRIu64 " state=%u status=%08x\n",
        intent == MOIREN_VAD_INTENT_MICROPHONE ? "microphone" : "playback",
        timerDriven ? "timer" : "event", request.Header.GateEpoch,
        request.AtCompletion.GateEpoch, request.AtCompletion.StreamState,
        static_cast<unsigned>(request.Status));
    Check(request.Completions == 1 && request.Status == STATUS_CANCELLED &&
        (request.Header.GateEpoch != request.AtCompletion.GateEpoch ||
            request.AtCompletion.GateOpen == 0),
        "Release fences requests admitted after Stop before cancellation completion");
    Check(request.AtCompletion.StreamState == MOIREN_VAD_STREAM_CLOSED &&
        request.AtCompletion.SampleFormat == 0 && request.AtCompletion.PacketFrames == 0 &&
        child.ActiveStreams == 0 && queue.Count == 0 && slot.PendingRequests == 0,
        "Release publishes Closed/empty shape and clears active owner after drain");
    VAD_PCM_REQUEST late = {};
    late.Input = request.Input;
    late.Code = request.Code;
    Check(VadPcmSubmit(&child, &late, late.Code) == STATUS_REVISION_MISMATCH,
        "Closed rejects subsequent requests even with the former valid shape");
}

static void GuardPaths()
{
    // uninitialized, denied rundown, null queue, offline, no restart, normal.
    const char* expected[] = {"", "CRc", "CRMmrc", "CRMPmrc", "CRMPmrc", "CRMPSmrc"};
    for (unsigned path = 0; path < 6; ++path) {
        ++Scenarios;
        VAD_SHARED shared = {};
        Queue queue = {};
        shared.PcmInitialized = path != 0;
        shared.PcmRundown.Denied = path == 1;
        shared.PcmQueues[0] = path == 2 ? nullptr : &queue;
        shared.PcmOnline = path == 3 ? 0 : 1;
        GuardOrderCount = 0;
        GuardOrder[0] = '\0';
        RecordGuardOrder = true;
        VadPcmPurge(&shared, 0, path != 4);
        RecordGuardOrder = false;
        Check(std::strcmp(GuardOrder, expected[path]) == 0,
            "critical/rundown/mutex/purge/restart release order is exact on every wrapper path");
        Check(shared.PcmRundown.Users == 0 && !shared.PcmDrain[0].Held && CriticalDepth == 0 &&
            shared.PcmRundown.Attempts == (path == 0 ? 0u : 1u),
            "inert and active guards balance rundown and critical region without recursion");
        std::printf("case=guard-path-%u order=%s purges=%u starts=%u\n",
            path, GuardOrder, queue.Purges, queue.Starts);
    }
}

static void InertStop(bool initialized, bool reset)
{
    ++Scenarios;
    VAD_SHARED shared = {};
    shared.PcmInitialized = initialized;
    shared.PcmRundown.Denied = initialized;
    auto& slot = shared.Slots[0];
    slot.Id = 11;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = 5;
    slot.GateOpen = 1;
    slot.Channels = 2;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    AudioDevice audio = {};
    Child child = {&shared, &audio};
    HostTimer timer = {};
    VAD_AUDIO_STREAM stream = {};
    stream.Device = &child;
    stream.Timer = &timer;
    {
        LifecycleGuard lifecycle(&audio);
        VadAudioStopLocked(&stream, reset);
    }
    Check(slot.AdmissionGeneration == 6 && slot.StreamState == MOIREN_VAD_STREAM_PAUSED &&
        !stream.Running && !stream.Clock.Running &&
        stream.CompletedPackets == (reset ? 0u : 3u) &&
        stream.LastCaptureStart == (reset ? 0u : 9990000u) && stream.Started == !reset &&
        timer.Stops == 1 && shared.PcmRundown.Users == 0 && CriticalDepth == 0 &&
        !shared.PcmDrain[0].Held && !audio.LifecycleHeld,
        "inert guard cannot skip stop, fence, reset or timestamp preservation");
    std::printf("case=inert-stop initialized=%u reset=%u gate=%" PRIu64 "\n",
        initialized ? 1u : 0u, reset ? 1u : 0u, slot.AdmissionGeneration);
}

int main()
{
    SuiteWatchdog watchdog;
    GuardPaths();
    for (const bool initialized : {false, true}) {
        InertStop(initialized, false);
        InertStop(initialized, true);
    }
    for (const auto intent : {MOIREN_VAD_INTENT_PLAYBACK, MOIREN_VAD_INTENT_MICROPHONE}) {
        for (const bool timerDriven : {false, true}) {
            StopCancellation(intent, timerDriven, false);
            StopCancellation(intent, timerDriven, true);
            StopCancellation(intent, timerDriven, false, 5, 2);
            StopCancellation(intent, timerDriven, false, ~uint64_t{0} - 1, 2);
            StopCancellation(intent, timerDriven, false, ~uint64_t{0});
            StopCancellation(intent, timerDriven, true, ~uint64_t{0} - 1);
            StopCancellation(intent, timerDriven, true, ~uint64_t{0});
            FreshStampResetInterleave(intent, timerDriven);
            ReleaseAfterStop(intent, timerDriven);
            FreshStampAfterPublication(intent, timerDriven, false);
            FreshStampAfterPublication(intent, timerDriven, true);
        }
    }
    std::printf("pause fence: %u scenarios, %u failures\n", Scenarios, Failures);
    return Failures == 0 ? 0 : 1;
}
