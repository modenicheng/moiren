// SPDX-License-Identifier: MIT
#include "../src/PcmDiagnostics.h"
#include "../src/audio/CaptureDiagnostic.h"
#include "../src/UnservedDiagnostics.h"
#include "../src/Lifecycle.h"
#include "../src/Ownership.h"
#include <array>
#include <cstdio>
#include <cstdlib>
#include <cstring>

static void Check(bool condition, const char* message)
{
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

// Compile the real transfer body, stubbing only the queue, QPC and kernel memory
// boundaries. This exercises missing-record branches, not a copied algorithm.
using ULONG = uint32_t;
using NTSTATUS = int32_t;
using PVOID = void*;
using PUCHAR = unsigned char*;
using LONG = long;
using KIRQL = unsigned;
struct SpinLock { bool Held = false; };
constexpr NTSTATUS STATUS_SUCCESS = 0;
constexpr NTSTATUS STATUS_REVISION_MISMATCH = -1;
constexpr NTSTATUS STATUS_IO_TIMEOUT = -2;
#define NT_SUCCESS(status) ((status) >= 0)
#define NT_ASSERT(condition) Check((condition), "production assertion")
#define UNREFERENCED_PARAMETER(value) (void)(value)
struct Request {
    MOIREN_VAD_PCM_HEADER Header = {};
    PVOID Buffer = nullptr;
    struct VAD_SHARED* Shared = nullptr;
    ULONG Index = 0;
    LONG Counted = 0;
};
using WDFREQUEST = Request*;
struct Queue {
    std::array<WDFREQUEST, 4> Requests = {};
    ULONG Next = 0;
    ULONG Count = 0;
};
struct VAD_SHARED {
    SpinLock Lock;
    vad::Slot Slots[MOIREN_VAD_MAX_ENDPOINTS] = {};
    vad::PcmDiagnostics PcmDiagnostics[MOIREN_VAD_MAX_ENDPOINTS] = {};
    vad::audio::CaptureDiagnostic CaptureDiagnostics[MOIREN_VAD_MAX_ENDPOINTS] = {};
    vad::UnservedDiagnostics UnservedDiagnostics[MOIREN_VAD_MAX_ENDPOINTS] = {};
    Queue* PcmQueues[MOIREN_VAD_MAX_ENDPOINTS] = {};
    LONG PcmOnline = 1;
    unsigned PcmRundown = 0;
    unsigned PcmDrain[1] = {};
};
struct VAD_PCM_COMPLETION {
    WDFREQUEST Request;
    NTSTATUS Status;
    ULONG Information;
};
struct VAD_PCM_BATCH {
    vad::CompletionBatch<VAD_PCM_COMPLETION, MOIREN_VAD_REQUEST_SLOTS> Completions = {};
    VAD_SHARED* Shared;
    vad::CallbackBoundary Callback = {};
    bool CaptureCopied = false;
    uint64_t CaptureGateEpoch = 0;
    uint64_t CaptureRequestSequence = 0;
    vad::audio::CapturePrefix CapturePrefix = {};
};
struct LARGE_INTEGER { int64_t QuadPart; };
static LARGE_INTEGER KeQueryPerformanceCounter(LARGE_INTEGER* frequency)
{
    if (frequency != nullptr) { frequency->QuadPart = 48000; }
    return {1001};
}
static NTSTATUS WdfIoQueueRetrieveNextRequest(Queue* queue, WDFREQUEST* result)
{
    if (queue->Next == queue->Count) { return -3; }
    *result = queue->Requests[queue->Next++];
    return STATUS_SUCCESS;
}
static Request* VadPcmRequest(WDFREQUEST request) { return request; }
static void DropCountLocked(Request*) {}
static void RtlZeroMemory(PVOID buffer, ULONG bytes) { std::memset(buffer, 0, bytes); }
static void RtlCopyMemory(PVOID target, const void* source, ULONG bytes)
{
    std::memcpy(target, source, bytes);
}
#include "pcm-production.inc"

#ifndef VAD_DIAGNOSTICS_FIXTURE
static void ProductionTransferDiagnostics()
{
    VAD_SHARED shared = {};
    auto& slot = shared.Slots[0];
    slot.Id = 5;
    slot.Session = 6;
    slot.StreamGeneration = 3;
    slot.AdmissionGeneration = 7;
    slot.GateOpen = slot.AudioReady = 1;
    slot.State = MOIREN_VAD_STATE_PRESENT;
    slot.StreamState = MOIREN_VAD_STREAM_RUNNING;
    slot.Intent = MOIREN_VAD_INTENT_MICROPHONE;
    slot.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
    slot.Channels = 1;
    slot.PacketFrames = 480;
    Queue queue = {};
    shared.PcmQueues[0] = &queue;
    std::array<Request, 4> requests = {};
    for (ULONG i = 0; i < 4; ++i) {
        auto& request = requests[i];
        request.Header.EndpointId = request.Header.EndpointGeneration = 5;
        request.Header.SessionEpoch = 6;
        request.Header.StreamGeneration = 3;
        request.Header.GateEpoch = 6; // Four retired requests consume the budget.
        request.Header.SampleFormat = MOIREN_VAD_FORMAT_PCM16;
        request.Header.Channels = 1;
        request.Header.Frames = 480;
        queue.Requests[i] = &request;
    }
    queue.Count = 4;
    std::array<unsigned char, 960> pcm;
    pcm.fill(9);
    VAD_PCM_BATCH batch = {};
    batch.Shared = &shared;
    shared.PcmDiagnostics[0].Reset(3);
    VadPcmTransferLocked(&shared, 0, pcm.data(), 480, 2, 0, 1000, &batch);
    const auto first = shared.PcmDiagnostics[0].TakeUnreported();
    Check(first.ReportFirst &&
        first.FirstTransfer.Outcome == vad::PcmWriteOutcome::NoCurrentRequest &&
        first.FirstTransfer.NativeSequence == 0 &&
        first.FirstTransfer.Frames == 480,
        "production first packet remains visible after four retired requests");
    Check(batch.Completions.Count == 4 && slot.SilencePackets == 1 &&
        pcm[0] == 0 && pcm[959] == 0,
        "diagnostics never change stale completions or capture silence");

    shared.PcmDiagnostics[0].Reset(4);
    slot.StreamGeneration = 4;
    auto& current = requests[0];
    current.Header.StreamGeneration = 4;
    current.Header.GateEpoch = 7;
    current.Header.Sequence = 20;
    current.Header.Qpc = 1001;
    queue.Count = 1;
    queue.Next = 0;
    batch.Completions = {};
    VadPcmTransferLocked(&shared, 0, pcm.data(), 480, 2, 1, 1000, &batch);
    // A later shape change must not rewrite the historical packet's frames.
    slot.PacketFrames = 0;
    const auto rejected = shared.PcmDiagnostics[0].TakeUnreported();
    Check(rejected.FirstRejected.Outcome == vad::PcmWriteOutcome::NewerThanCallback &&
        rejected.FirstRejected.Frames == 480 &&
        rejected.FirstRejected.HeaderQpc == 1001 &&
        rejected.FirstRejected.CallbackQpc == 1000 &&
        rejected.FirstRejected.CheckQpc == 1001 &&
        rejected.FirstRejected.PeriodTicks == 480 &&
        batch.Completions.Entries[0].Status == STATUS_IO_TIMEOUT,
        "production rejection retains actual shape and callback/check QPC window");
}

int main()
{
    ProductionTransferDiagnostics();
    using vad::PcmWriteOutcome;
    Check(vad::ClassifyPcmWrite(90, 100, 10) == PcmWriteOutcome::Success,
        "one-period age remains inclusive");
    Check(vad::ClassifyPcmWrite(89, 100, 10) == PcmWriteOutcome::TooOld,
        "one tick over the real period is over-age");
    Check(vad::ClassifyPcmWrite(101, 100, 10) == PcmWriteOutcome::NewerThanCallback,
        "a newer timestamp is not mislabeled over-age");
    Check(vad::ClassifyPcmWrite(100, 100, 10) == PcmWriteOutcome::Success,
        "same-tick submission remains fresh");

    vad::PcmDiagnostics diagnostics = {};
    diagnostics.Reset(3);
    diagnostics.RecordRun(11, 50);
    diagnostics.Record({PcmWriteOutcome::Empty, 11, 0, 0, 0, 60, 60, 10});
    diagnostics.Record({PcmWriteOutcome::Success, 11, 1, 7, 65, 70, 71, 10});
    diagnostics.Record({PcmWriteOutcome::NewerThanCallback, 11, 2, 8, 81, 80, 82, 10});
    diagnostics.Record({PcmWriteOutcome::TooOld, 11, 3, 9, 70, 90, 91, 10});
    Check(diagnostics.FirstTransfer.Outcome == PcmWriteOutcome::Empty &&
        diagnostics.FirstTransfer.NativeSequence == 0 &&
        diagnostics.FirstTransfer.RequestSequence == 0,
        "an actual empty first packet stays explicit after later writes");
    Check(diagnostics.FirstRejected.Outcome == PcmWriteOutcome::NewerThanCallback &&
        diagnostics.FirstRejected.NativeSequence == 2 &&
        diagnostics.FirstRejected.RequestSequence == 8 &&
        diagnostics.FirstRejected.HeaderQpc == 81 &&
        diagnostics.FirstRejected.CallbackQpc == 80 &&
        diagnostics.FirstRejected.CheckQpc == 82 &&
        diagnostics.FirstRejected.PeriodTicks == 10,
        "first rejection preserves the sampling window and exact request identity");
    auto unread = diagnostics.TakeUnreported();
    Check(unread.ReportFirst && unread.ReportRejected &&
        unread.Generation == 3 && unread.RunGateEpoch == 11 && unread.RunQpc == 50,
        "control snapshot retains run and both first observations");
    auto repeated = diagnostics.TakeUnreported();
    Check(!repeated.ReportFirst && !repeated.ReportRejected &&
        diagnostics.FirstRejected.RequestSequence == 8,
        "repeated control queries do not re-log or erase diagnostics");
    diagnostics.RecordRun(12, 100);
    Check(diagnostics.RunGateEpoch == 11 && diagnostics.RunQpc == 50,
        "a resume does not relabel the first run in the generation");
    diagnostics.Reset(4);
    Check(diagnostics.Generation == 4 && diagnostics.RunQpc == 0 &&
        diagnostics.FirstTransfer.Outcome == PcmWriteOutcome::Unobserved &&
        diagnostics.FirstRejected.Outcome == PcmWriteOutcome::Unobserved,
        "a new stream generation cannot inherit old evidence");
    diagnostics.Record({PcmWriteOutcome::Success, 13, 0, 2, 110, 111, 112, 10});
    auto replaced = diagnostics.TakeUnreported();
    Check(replaced.ReportFirst && !replaced.ReportRejected &&
        replaced.FirstTransfer.GateEpoch == 13 && replaced.Generation == 4,
        "new successful generation is not attributed an old rejection");
    std::puts("PASS: unchanged freshness boundaries, empty/first rejection, "
        "QPC window, bounded control publication and generation reset");
}
#endif
