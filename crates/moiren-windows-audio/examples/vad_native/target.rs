use crate::{
    NativeMode,
    period::PeriodEvidence,
    validation::{self, Candidate, Observation, PacketProgress},
};
use anyhow::{Context, ensure};
use moiren_windows_audio::vad::{
    Channels, ControlClient, EndpointSnapshot, Intent, PcmCompletion, PcmOwner, PcmTicket,
    SampleFormat, StableEndpointId, StreamState, decode_interleaved, discover_present_interfaces,
    encode_interleaved,
};
use serde::Serialize;
use std::{
    collections::{HashSet, VecDeque},
    io::Write,
    marker::PhantomData,
    rc::Rc,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::ERROR_NOT_FOUND,
        Media::Audio::{
            IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eCommunications, eConsole,
            eMultimedia, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
        },
        System::Performance::QueryPerformanceCounter,
    },
    core::HSTRING,
};

#[path = "identity.rs"]
mod identity;
#[path = "native.rs"]
mod native;

const CONTROL_DEADLINE: Duration = Duration::from_secs(15);

fn require_case_signal(mode: NativeMode, seen: &Observation, minimum: u64) -> anyhow::Result<()> {
    if mode.is_ac_diagnostic() {
        anyhow::bail!(
            "AC diagnostic only: {} nonzero frames, {} silent frames; energy is not waveform validation, always unqualified",
            seen.signal_frames,
            seen.silent_frames
        );
    }
    seen.require_signal(minimum)
}

fn includes_endpoint(mode: NativeMode, intent: Intent, channels: Channels) -> bool {
    !mode.is_shared_capture() || (intent == Intent::Microphone && channels == Channels::Stereo)
}

fn require_shared_shape(
    mode: NativeMode,
    format: Option<SampleFormat>,
    channels: Channels,
    frames: u32,
) -> anyhow::Result<()> {
    ensure!(
        !mode.is_shared_capture()
            || (format == Some(SampleFormat::Pcm16)
                && channels == Channels::Stereo
                && frames == 480),
        "SharedCapture requires native QUERY stereo PCM16/480 without fallback"
    );
    Ok(())
}

#[derive(Clone, Default, Serialize)]
struct SharedCaptureDiagnostics {
    submitted_packets: u64,
    submitted_bytes: u64,
    submitted_samples: u64,
    submitted_nonzero_samples: u64,
    submitted_peak_abs_pcm16: u32,
    first_submitted_samples: [i16; 16],
    first_submitted_sample_count: usize,
    successful_capture_completions: u64,
    successful_capture_bytes: u64,
    dropped_capture_completions: u64,
    capture_all_phases: Option<native::CaptureDiagnostics>,
    capture_before_revoke: Option<native::CaptureDiagnostics>,
}

impl SharedCaptureDiagnostics {
    fn submitted(&mut self, payload: &[u8]) {
        self.submitted_packets += 1;
        self.submitted_bytes += payload.len() as u64;
        for pair in payload.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes(*pair);
            self.submitted_samples += 1;
            self.submitted_nonzero_samples += u64::from(sample != 0);
            self.submitted_peak_abs_pcm16 = self
                .submitted_peak_abs_pcm16
                .max(u32::from(sample.unsigned_abs()));
            if self.first_submitted_sample_count < self.first_submitted_samples.len() {
                self.first_submitted_samples[self.first_submitted_sample_count] = sample;
                self.first_submitted_sample_count += 1;
            }
        }
    }

    fn completed(
        &mut self,
        completion: &PcmCompletion<'_>,
        expected_bytes: usize,
    ) -> anyhow::Result<()> {
        match completion {
            PcmCompletion::Capture { bytes, .. } => {
                ensure!(
                    *bytes == expected_bytes,
                    "successful WRITE completion byte count changed: {bytes} != {expected_bytes}"
                );
                self.successful_capture_completions += 1;
                self.successful_capture_bytes += *bytes as u64;
            }
            PcmCompletion::CaptureDropped { .. } => self.dropped_capture_completions += 1,
            PcmCompletion::Render { .. } => {
                anyhow::bail!("SharedCapture received a render completion")
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingPcm<T> {
    ticket: T,
    write: Option<validation::WriteSubmission>,
}

#[derive(Clone, Copy)]
struct WriteTiming {
    origin: Instant,
    deadline: Option<Instant>,
    pending: usize,
}

fn submit_microphone<T, E>(
    diagnostics: &mut validation::MicrophoneSubmitDiagnostics,
    timing: WriteTiming,
    now: impl FnOnce() -> Instant,
    mut qpc: impl FnMut() -> Option<u64>,
    submit: impl FnOnce() -> Result<T, E>,
) -> Result<PendingPcm<T>, E> {
    let entry = now();
    let before = qpc();
    let result = submit();
    let after = qpc();
    // No formatting, allocation or diagnostic failure is introduced into PCM.
    // The entry Instant measures wrapper-entry lateness, not header QPC time.
    let record = diagnostics.submitted(validation::WriteSubmission {
        probe_ticket_id: None,
        submit_before_qpc: before,
        submit_after_qpc: after,
        target_deadline_elapsed_ns: timing
            .deadline
            .map(|deadline| deadline.saturating_duration_since(timing.origin).as_nanos()),
        submit_entry_elapsed_ns: entry.saturating_duration_since(timing.origin).as_nanos(),
        submit_entry_lateness_ns: timing
            .deadline
            .map(|deadline| entry.saturating_duration_since(deadline).as_nanos()),
        pending_before: timing.pending,
        pending_after: timing.pending + usize::from(result.is_ok()),
        submit_succeeded: result.is_ok(),
        completion: None,
    });
    result.map(|ticket| PendingPcm {
        ticket,
        write: Some(record),
    })
}

fn record_microphone_completion<T>(
    diagnostics: &mut validation::MicrophoneSubmitDiagnostics,
    pending: &PendingPcm<T>,
    completion: &PcmCompletion<'_>,
) {
    let (status, stamp) = match completion {
        PcmCompletion::Capture { stamp, .. } => ("capture", stamp),
        PcmCompletion::CaptureDropped { stamp } => ("capture_dropped", stamp),
        PcmCompletion::Render { .. } => return,
    };
    if let Some(record) = pending.write {
        diagnostics.completed(
            record,
            validation::WriteCompletion {
                status,
                header_sequence: stamp.sequence,
                header_qpc: stamp.qpc,
            },
        );
    }
}

fn start_native_with_queued_pcm<T>(
    intent: Intent,
    mode: NativeMode,
    mut submit: impl FnMut() -> anyhow::Result<T>,
    start: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<VecDeque<T>> {
    let mut pending = VecDeque::with_capacity(4);
    let count = match (intent, mode) {
        (Intent::Playback, _) => 4,
        (
            Intent::Microphone,
            NativeMode::Timer
            | NativeMode::SharedCapture
            | NativeMode::SharedRawCapture
            | NativeMode::SharedAcDiagnostic
            | NativeMode::SharedRawAcDiagnostic,
        ) => 1,
        (Intent::Microphone, NativeMode::Event) => 0,
    };
    for _ in 0..count {
        pending.push_back(submit()?);
    }
    start()?;
    Ok(pending)
}

#[derive(Default)]
struct TransferProgress {
    playback: PacketProgress,
    microphone_drops: u64,
    microphone_completion_diagnostics: validation::MicrophoneCompletionDiagnostics,
    next_mic_submit: Option<Instant>,
    last_mic_phase: Option<Instant>,
    immediate_microphone: bool,
}

impl TransferProgress {
    fn native_started(now: Instant, frames: u32, intent: Intent, mode: NativeMode) -> Self {
        Self {
            // Event capture consumes at the packet boundary; timer capture
            // consumes at its first segment and needs the WRITE immediately.
            // Anchor event headroom before QUERY so it cannot re-arm the delay.
            next_mic_submit: (intent == Intent::Microphone && mode == NativeMode::Event)
                .then(|| Self::microphone_deadline(now, frames)),
            immediate_microphone: mode.is_shared_capture(),
            ..Self::default()
        }
    }

    fn microphone_deadline(now: Instant, frames: u32) -> Instant {
        let half_period = Duration::from_nanos(u64::from(frames) * 1_000_000_000 / (48_000 * 2));
        now + half_period
    }

    fn microphone_period(frames: u32) -> Duration {
        Duration::from_nanos(u64::from(frames) * 1_000_000_000 / 48_000)
    }

    fn microphone_completed(&mut self, now: Instant, frames: u32, dropped: bool) {
        if self.immediate_microphone {
            self.last_mic_phase = None;
            self.next_mic_submit = Some(now);
            return;
        }
        // Successful collection can lag the native boundary. Advance the prior
        // intended phase, not collection time; overdue means one WRITE now,
        // never a catch-up burst. First/unanchored and dropped completions keep
        // the old collection-relative halfway re-arm, without relaxing freshness.
        let cadence = self
            .last_mic_phase
            .take()
            .filter(|_| !dropped)
            .and_then(|phase| phase.checked_add(Self::microphone_period(frames)))
            .map(|phase| phase.max(now));
        self.next_mic_submit =
            Some(cadence.unwrap_or_else(|| Self::microphone_deadline(now, frames)));
    }

    fn microphone_submitted(&mut self, now: Instant, frames: u32) {
        // Match the production worker's late-skip rule: an accepted WRITE a
        // whole period late cannot carry an arbitrarily old intended phase.
        self.last_mic_phase = self.next_mic_submit.take().filter(|&phase| {
            now.saturating_duration_since(phase) < Self::microphone_period(frames)
        });
    }

    fn microphone_ready(&self, now: Instant, pending: usize) -> bool {
        pending == 0 && self.next_mic_submit.is_none_or(|deadline| now >= deadline)
    }
}

struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn new() -> anyhow::Result<Self> {
        // SAFETY: this dedicated probe thread has no prior COM ownership.
        // S_OK and S_FALSE are both balanced by exactly one CoUninitialize.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[derive(Serialize)]
struct CaseReport {
    stable_id: String,
    mmdevice_id: String,
    adapter_instance: String,
    intent: String,
    channels: u32,
    format: String,
    period: PeriodEvidence,
    native_buffer_frames: u32,
    driver_packet_frames: u32,
    stream_generation: u64,
    signal_seconds: u32,
    observation: Observation,
    closed_gate_before: Option<Observation>,
    closed_gate_after: Option<Observation>,
    microphone_dropped_packets: u64,
    microphone_completion_diagnostics: validation::MicrophoneCompletionDiagnostics,
    microphone_submit_diagnostics: validation::MicrophoneSubmitDiagnostics,
    native_discontinuities: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_position: Option<validation::PositionProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shared_capture_diagnostics: Option<SharedCaptureDiagnostics>,
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    native_mode: NativeMode,
    minimum_period: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    intent_filter: Option<&'static str>,
    root_instance: String,
    defaults_before: Vec<Option<String>>,
    defaults_after: Vec<Option<String>>,
    cases: Vec<CaseReport>,
}

pub(super) fn exercise(
    seconds: u32,
    mode: NativeMode,
    intent_filter: Option<Intent>,
    minimum_period: bool,
) -> anyhow::Result<()> {
    ensure!(
        !mode.is_shared_capture() || (!minimum_period && intent_filter == Some(Intent::Microphone)),
        "SharedCapture requires microphone-only and no minimum period"
    );
    let interfaces = discover_present_interfaces()?;
    ensure!(
        interfaces.len() == 1,
        "native probe requires exactly one present control root, found {}",
        interfaces.len()
    );
    let _apartment = Apartment::new()?;
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let root = identity::root_instance(&interfaces[0]).context("control root identity")?;
    let before = defaults(&enumerator)?;
    let mut control = ControlClient::connect(&interfaces[0])?;
    control.capabilities().require_pcm()?;
    let mut endpoints = Vec::with_capacity(4);
    let result = (|| -> anyhow::Result<Report> {
        for intent in [Intent::Playback, Intent::Microphone] {
            for channels in [Channels::Mono, Channels::Stereo] {
                if !includes_endpoint(mode, intent, channels) {
                    continue;
                }
                let stable_id = StableEndpointId::new(
                    intent as u64 | ((channels as u64) << 32),
                    0x4d6f_6972_656e_4e41,
                );
                let endpoint = control.create_and_wait(
                    stable_id,
                    intent,
                    channels,
                    &format!("Moiren native probe {intent:?} {channels:?}"),
                    false,
                    CONTROL_DEADLINE,
                )?;
                endpoints.push(endpoint);
            }
        }
        let mappings = wait_mappings(&enumerator, &endpoints, &root)?;
        require_never_default(&enumerator, &mappings)?;
        let mut cases = Vec::with_capacity(8);
        for (index, (endpoint, mapping)) in endpoints.iter_mut().zip(&mappings).enumerate() {
            if intent_filter.is_some_and(|intent| endpoint.intent() != intent) {
                continue;
            }
            for format in [SampleFormat::Pcm16, SampleFormat::Float32] {
                if mode.is_shared_capture() && format != SampleFormat::Pcm16 {
                    continue;
                }
                eprintln!(
                    "native case mode={mode:?} minimum_period={minimum_period} intent={:?} channels={:?} format={format:?} id={}",
                    endpoint.intent(),
                    endpoint.channels(),
                    endpoint.stable_id()
                );
                let report = exercise_case(
                    &mut control,
                    &enumerator,
                    *endpoint,
                    mapping,
                    &root,
                    format,
                    index,
                    seconds,
                    mode,
                    minimum_period,
                )
                .with_context(|| {
                    format!(
                        "native {mode:?} {:?} {:?} {format:?}",
                        endpoint.intent(),
                        endpoint.channels()
                    )
                })?;
                cases.push(report);
                *endpoint = wait_closed(&mut control, *endpoint)?;
            }
        }
        require_never_default(&enumerator, &mappings)?;
        let after = defaults(&enumerator)?;
        Ok(Report {
            schema_version: 3,
            native_mode: mode,
            minimum_period,
            intent_filter: intent_filter.map(|intent| match intent {
                Intent::Playback => "Playback",
                Intent::Microphone => "Microphone",
            }),
            root_instance: root,
            defaults_before: before,
            defaults_after: after,
            cases,
        })
    })();

    let cleanup = validation::cleanup_all(&endpoints, |endpoint| {
        cleanup_endpoint(&mut control, endpoint)
    });
    let report = match (result, cleanup) {
        (Ok(report), Ok(())) => report,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(error.context(format!(
                "VAD endpoint cleanup also failed: {cleanup_error:#}"
            )));
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn defaults(enumerator: &IMMDeviceEnumerator) -> anyhow::Result<Vec<Option<String>>> {
    let mut result = Vec::with_capacity(6);
    for flow in [eRender, eCapture] {
        for role in [eConsole, eMultimedia, eCommunications] {
            // SAFETY: read-only default-role observations in the current apartment.
            match unsafe { enumerator.GetDefaultAudioEndpoint(flow, role) } {
                Ok(device) => result.push(Some(identity::take_string(unsafe { device.GetId()? })?)),
                Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => result.push(None),
                Err(error) => return Err(error).context("GetDefaultAudioEndpoint"),
            }
        }
    }
    Ok(result)
}

fn require_never_default(
    enumerator: &IMMDeviceEnumerator,
    mappings: &[Candidate],
) -> anyhow::Result<()> {
    for default in defaults(enumerator)?.into_iter().flatten() {
        ensure!(
            mappings.iter().all(|row| row.id != default),
            "probe endpoint unexpectedly became a default: {default}"
        );
    }
    Ok(())
}

fn wait_mappings(
    enumerator: &IMMDeviceEnumerator,
    endpoints: &[EndpointSnapshot],
    root: &str,
) -> anyhow::Result<Vec<Candidate>> {
    let deadline = Instant::now() + CONTROL_DEADLINE;
    loop {
        let (rows, errors) = identity::candidates(enumerator)?;
        let mut mapped = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            if let Some(row) =
                validation::select(&rows, endpoint.stable_id(), endpoint.intent(), root)?
            {
                mapped.push(row.clone());
            }
        }
        if mapped.len() == endpoints.len() {
            let distinct: HashSet<_> = mapped.iter().map(|row| &row.id).collect();
            ensure!(
                distinct.len() == endpoints.len(),
                "sibling MMDevice identities alias"
            );
            return Ok(mapped);
        }
        ensure!(
            Instant::now() < deadline,
            "only {}/{} owned endpoints mapped to MMDevices; identity diagnostics={errors:?}",
            mapped.len(),
            endpoints.len()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_closed(
    control: &mut ControlClient,
    mut previous: EndpointSnapshot,
) -> anyhow::Result<EndpointSnapshot> {
    let deadline = Instant::now() + CONTROL_DEADLINE;
    loop {
        previous = control
            .query(previous.key())?
            .require_progress_after(previous)?;
        ensure!(!previous.gate_open(), "gate unexpectedly reopened");
        if previous.stream_state() == StreamState::Closed {
            return Ok(previous);
        }
        ensure!(
            Instant::now() < deadline,
            "previous native stream did not close"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn cleanup_endpoint(control: &mut ControlClient, endpoint: EndpointSnapshot) -> anyhow::Result<()> {
    let mut latest = control
        .query(endpoint.key())?
        .require_progress_after(endpoint)?;
    if latest.gate_open() {
        latest = control.close_gate(latest)?;
    }
    let removed = control.remove_and_wait(latest.key(), CONTROL_DEADLINE)?;
    ensure!(
        removed.removal_complete_after(latest)?,
        "removal accepted without final Removed"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn exercise_case(
    control: &mut ControlClient,
    enumerator: &IMMDeviceEnumerator,
    endpoint: EndpointSnapshot,
    mapping: &Candidate,
    root: &str,
    format: SampleFormat,
    index: usize,
    seconds: u32,
    mode: NativeMode,
    minimum_period: bool,
) -> anyhow::Result<CaseReport> {
    // Caller-owned evidence survives every native preparation error path.
    let mut period = PeriodEvidence::new(minimum_period);
    let mut shared = mode
        .is_shared_capture()
        .then(SharedCaptureDiagnostics::default);
    let mut counters = validation::FailureCounters {
        mode,
        signal_frames: 0,
        silent_frames: 0,
        microphone_dropped_packets: 0,
        microphone_completion_diagnostics: Default::default(),
        microphone_submit_diagnostics: Default::default(),
        native_buffer_frames: 0,
        driver_packet_frames: endpoint.packet_frames(),
        stream_generation: endpoint.stream_generation(),
        native_discontinuities: 0,
        native_position: None,
        admitted_opened: None,
        admitted_pcm_shape: None,
        failure_observation: None,
    };
    let outcome = exercise_case_inner(
        control,
        enumerator,
        endpoint,
        mapping,
        root,
        format,
        index,
        seconds,
        mode,
        &mut counters,
        &mut period,
        &mut shared,
    );
    diagnose_case_failure(outcome, &counters, &period, shared.as_ref(), |line| {
        // A failed stderr write must not replace the original case error.
        let _ = writeln!(std::io::stderr().lock(), "{line}");
    })
}

fn diagnose_case_failure<T>(
    outcome: anyhow::Result<T>,
    counters: &validation::FailureCounters,
    period: &PeriodEvidence,
    shared: Option<&SharedCaptureDiagnostics>,
    emit: impl FnOnce(&str),
) -> anyhow::Result<T> {
    validation::diagnose_failure(outcome, counters, |line| {
        let enriched = (|| -> serde_json::Result<String> {
            let mut diagnostic: serde_json::Value = serde_json::from_str(line)?;
            diagnostic["counters"]["period"] = serde_json::to_value(period)?;
            if counters.mode.is_ac_diagnostic() {
                diagnostic["signal_validation"] =
                    "unqualified-ac-energy-only; no waveform matching".into();
            }
            if let Some(shared) = shared {
                diagnostic["counters"]["shared_capture_diagnostics"] =
                    serde_json::to_value(shared)?;
            }
            serde_json::to_string(&diagnostic)
        })();
        let enriched = enriched.unwrap_or_else(|error| {
            format!("{line}; period={period:?}; period diagnostic formatting: {error}")
        });
        emit(&enriched);
    })
}

#[allow(clippy::too_many_arguments)]
fn exercise_case_inner(
    control: &mut ControlClient,
    enumerator: &IMMDeviceEnumerator,
    endpoint: EndpointSnapshot,
    mapping: &Candidate,
    root: &str,
    format: SampleFormat,
    index: usize,
    seconds: u32,
    mode: NativeMode,
    counters: &mut validation::FailureCounters,
    period: &mut PeriodEvidence,
    shared: &mut Option<SharedCaptureDiagnostics>,
) -> anyhow::Result<CaseReport> {
    let case_index = index * 2 + usize::from(format == SampleFormat::Float32);
    let pattern = validation::pattern(case_index, endpoint.channels());
    let device = unsafe { enumerator.GetDevice(&HSTRING::from(&mapping.id))? };
    // Recheck the opened object's topology before Initialize can occupy a stream.
    let current = [identity::candidate(enumerator, &device, endpoint.intent())?];
    ensure!(
        validation::select(&current, endpoint.stable_id(), endpoint.intent(), root)?.is_some(),
        "opened endpoint lost its owned adapter identity"
    );
    let mut native = native::Stream::prepare(
        &device,
        endpoint.intent(),
        format,
        endpoint.channels(),
        pattern,
        mode,
        period,
    )?;
    counters.native_buffer_frames = native.frames;
    let outcome = (|| -> anyhow::Result<CaseReport> {
        // Admit the prepared stream before native Start so the driver's first
        // scheduled packet belongs to the open gate, not closed-gate silence.
        let mut snapshot = endpoint;
        let prepare_deadline = Instant::now() + CONTROL_DEADLINE;
        loop {
            snapshot = control
                .query(snapshot.key())?
                .require_progress_after(snapshot)?;
            counters.driver_packet_frames = snapshot.packet_frames();
            counters.stream_generation = snapshot.stream_generation();
            ensure!(
                !snapshot.gate_open(),
                "gate opened before explicit probe admission"
            );
            if snapshot.stream_state() != StreamState::Closed {
                break;
            }
            ensure!(
                Instant::now() < prepare_deadline,
                "native preparation was not observed as a prepared stream"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        require_shared_shape(
            mode,
            snapshot.sample_format(),
            snapshot.channels(),
            snapshot.packet_frames(),
        )?;
        let opened = control.open_gate(snapshot)?;
        counters.admitted_opened = Some(opened.into());
        counters.stream_generation = opened.stream_generation();
        counters.driver_packet_frames = opened.packet_frames();
        // Gate admission now precedes native Start, so there is no closed-gate
        // running interval to sample as a before/after diagnostic.
        let before: Option<Observation> = None;

        // Queue playback reads while the native stream is still Paused. Timer
        // clients may consume the first packet immediately after Start; a
        // post-Start owner would leave that packet with no PCM request and
        // turn the startup window into measured silence.
        let mut pcm = match control.pcm_owner(opened) {
            Ok(pcm) => pcm,
            Err(error) => {
                return validation::revoke_and_drain(
                    Err(error.into()),
                    || revoke(control, opened),
                    || Ok(()),
                );
            }
        };
        let setup = (|| -> anyhow::Result<_> {
            let shape = pcm.shape();
            counters.admitted_pcm_shape = Some(shape.snapshot().into());
            counters.driver_packet_frames = shape.frames();
            counters.stream_generation = opened.stream_generation();
            ensure!(shape.format() == format, "PCM helper format changed");
            let mut bytes = vec![0; shape.pcm_bytes()];
            encode_interleaved(
                &pattern.repeat(shape.frames() as usize),
                &mut bytes,
                format,
                endpoint.channels(),
            )
            .map_err(|error| anyhow::anyhow!("driver pattern encoding: {error:?}"))?;
            if mode.is_ac_diagnostic() {
                bytes = validation::ac_microphone_payload(shape.frames())?;
            }
            let decoded = vec![0.0; shape.frames() as usize * 2];
            // Fixed diagnostic storage is already initialized in the case counters.
            // Keep the same origin across Timer's pre-Start WRITE and later pumps.
            let submit_origin = Instant::now();
            // Timer capture consumes at the first sub-packet segment. Prepare its
            // single WRITE while Paused, not after Start/QUERY; event capture must
            // instead retain the half-period freshness headroom below.
            let pending = start_native_with_queued_pcm(
                endpoint.intent(),
                mode,
                || {
                    if endpoint.intent() == Intent::Playback {
                        Ok(PendingPcm {
                            ticket: pcm.submit_read()?,
                            write: None,
                        })
                    } else {
                        let submitted = submit_microphone(
                            &mut counters.microphone_submit_diagnostics,
                            WriteTiming {
                                origin: submit_origin,
                                deadline: None,
                                pending: 0,
                            },
                            Instant::now,
                            collection_qpc,
                            || pcm.submit_write(&bytes).map_err(anyhow::Error::from),
                        );
                        if submitted.is_ok()
                            && let Some(metrics) = shared.as_mut()
                        {
                            metrics.submitted(&bytes);
                        }
                        submitted
                    }
                },
                || native.start(),
            )?;
            let progress = TransferProgress::native_started(
                Instant::now(),
                shape.frames(),
                endpoint.intent(),
                mode,
            );
            let mut snapshot = opened;
            let deadline = Instant::now() + CONTROL_DEADLINE;
            loop {
                snapshot = control
                    .query(snapshot.key())?
                    .require_progress_after(snapshot)?;
                counters.driver_packet_frames = snapshot.packet_frames();
                counters.stream_generation = snapshot.stream_generation();
                ensure!(snapshot.gate_open(), "gate closed before native Start");
                if snapshot.stream_state() == StreamState::Running {
                    require_shared_shape(
                        mode,
                        snapshot.sample_format(),
                        snapshot.channels(),
                        snapshot.packet_frames(),
                    )?;
                    ensure!(
                        snapshot.sample_format() == Some(format) && snapshot.packet_frames() > 0,
                        "driver native format differs from exclusive request"
                    );
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "native Start was not observed as Running"
                );
                // A control-operation deadline, not a runtime no-PCM fault criterion.
                native.poll(pattern, &mut Observation::default())?;
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok((pending, progress, snapshot, bytes, decoded, submit_origin))
        })();
        let (mut pending, mut progress, running, bytes, mut decoded, submit_origin) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                return validation::revoke_and_drain(
                    Err(error),
                    || revoke(control, opened),
                    || Ok(pcm.shutdown(Duration::from_secs(5))?),
                );
            }
        };
        let shape = pcm.shape();
        counters.driver_packet_frames = shape.frames();
        counters.stream_generation = opened.stream_generation();
        let mut seen = Observation::default();
        let outcome = (|| -> anyhow::Result<_> {
            native.position.begin_phase();
            let until = Instant::now() + Duration::from_secs(u64::from(seconds));
            while Instant::now() < until {
                native.poll(pattern, &mut seen)?;
                pump_pcm(
                    &mut pcm,
                    &mut pending,
                    &bytes,
                    &mut decoded,
                    pattern,
                    &mut seen,
                    &mut progress,
                    &mut counters.microphone_submit_diagnostics,
                    submit_origin,
                    shared.as_mut(),
                )?;
                std::thread::sleep(Duration::from_millis(1));
            }
            require_case_signal(mode, &seen, u64::from(seconds) * 48_000 / 2)?;
            if mode == NativeMode::Timer {
                native.position.require_advance()?;
            }
            let position = if mode == NativeMode::Timer {
                Some(native.position.clone())
            } else {
                None
            };
            Ok(position)
        })();
        counters.signal_frames = seen.signal_frames;
        counters.silent_frames = seen.silent_frames;
        counters.microphone_dropped_packets = progress.microphone_drops;
        counters.microphone_completion_diagnostics = progress.microphone_completion_diagnostics;
        if let Some(metrics) = shared.as_mut() {
            metrics.capture_before_revoke = native.capture_diagnostics.clone();
        }
        // Observe only a failed signal phase, before revocation changes its
        // admission epoch or the native stream is stopped. Never use this later
        // diagnostic snapshot to rebind the owner or authorize more PCM.
        let outcome = validation::observe_signal_failure(
            outcome,
            running,
            &mut counters.failure_observation,
            |key| Ok(control.query(key)?),
        );
        // Cut admission first, then drain every outstanding direct request before
        // closing its native stream, including phase and revocation failures.
        let outcome = validation::revoke_and_drain(
            outcome,
            || revoke(control, running),
            || Ok(pcm.shutdown(Duration::from_secs(5))?),
        );
        drop(pcm);
        let position = outcome?;
        let after = if endpoint.intent() == Intent::Microphone {
            // Already delivered WASAPI buffers may still contain valid old audio.
            // Drain two full buffers before measuring silence on the same RUN stream.
            let flush_until =
                Instant::now() + native.buffer_duration() * 2 + Duration::from_millis(20);
            let mut discarded = Observation::default();
            while Instant::now() < flush_until {
                native.poll(pattern, &mut discarded)?;
                std::thread::sleep(Duration::from_millis(1));
            }
            Some(observe_closed(&mut native, pattern)?)
        } else {
            None
        };
        native.stop()?;
        Ok(CaseReport {
            stable_id: endpoint.stable_id().to_string(),
            mmdevice_id: mapping.id.clone(),
            adapter_instance: mapping.raw_instance.clone(),
            intent: format!("{:?}", endpoint.intent()),
            channels: endpoint.channels() as u32,
            format: format!("{format:?}"),
            period: *period,
            native_buffer_frames: native.frames,
            driver_packet_frames: shape.frames(),
            stream_generation: running.stream_generation(),
            signal_seconds: seconds,
            observation: seen,
            closed_gate_before: before,
            closed_gate_after: after,
            microphone_dropped_packets: progress.microphone_drops,
            microphone_completion_diagnostics: progress.microphone_completion_diagnostics,
            microphone_submit_diagnostics: std::mem::take(
                &mut counters.microphone_submit_diagnostics,
            ),
            native_discontinuities: native.discontinuities,
            native_position: position,
            shared_capture_diagnostics: shared.clone(),
        })
    })();
    // Capture the final native state even for setup, gate-check or Stop errors.
    counters.native_discontinuities = native.discontinuities;
    if let Some(metrics) = shared.as_mut() {
        metrics.capture_all_phases = native.capture_diagnostics.clone();
    }
    if mode == NativeMode::Timer {
        counters.native_position = Some(native.position.clone());
    }
    outcome.map(|mut report| {
        report.shared_capture_diagnostics = shared.clone();
        report
    })
}

fn revoke(control: &mut ControlClient, opened: EndpointSnapshot) -> anyhow::Result<()> {
    let latest = control
        .query(opened.key())?
        .require_progress_after(opened)?;
    control.close_gate(latest)?;
    Ok(())
}

fn observe_closed(native: &mut native::Stream, pattern: [f32; 2]) -> anyhow::Result<Observation> {
    let until = Instant::now() + Duration::from_millis(250);
    let mut seen = Observation::default();
    while Instant::now() < until {
        native.poll(pattern, &mut seen)?;
        std::thread::sleep(Duration::from_millis(1));
    }
    seen.require_silence()?;
    Ok(seen)
}

#[allow(clippy::too_many_arguments)]
fn pump_pcm(
    pcm: &mut PcmOwner,
    pending: &mut VecDeque<PendingPcm<PcmTicket>>,
    encoded: &[u8],
    decoded: &mut [f32],
    pattern: [f32; 2],
    seen: &mut Observation,
    progress: &mut TransferProgress,
    submit_diagnostics: &mut validation::MicrophoneSubmitDiagnostics,
    submit_origin: Instant,
    mut shared: Option<&mut SharedCaptureDiagnostics>,
) -> anyhow::Result<()> {
    let shape = pcm.shape();
    let playback = shape.snapshot().intent() == Intent::Playback;
    while let Some(entry) = pending.front() {
        let Some(completed) = pcm.try_complete(entry.ticket)? else {
            break;
        };
        progress
            .microphone_completion_diagnostics
            .observe(&completed, collection_qpc);
        match completed {
            PcmCompletion::Render { stamp, pcm } => {
                progress.playback.observe(stamp)?;
                decode_interleaved(pcm, decoded, shape.format(), shape.channels())
                    .map_err(|error| anyhow::anyhow!("driver render decoding: {error:?}"))?;
                seen.observe(decoded, pattern)?;
            }
            PcmCompletion::Capture { .. } => {
                progress.microphone_completed(Instant::now(), shape.frames(), false);
            }
            PcmCompletion::CaptureDropped { .. } => {
                progress.microphone_drops += 1;
                progress.microphone_completed(Instant::now(), shape.frames(), true);
            }
        }
        // Recording cannot choose a later pacing origin or turn a drop into
        // another PCM result.
        record_microphone_completion(submit_diagnostics, entry, &completed);
        if let Some(metrics) = shared.as_mut() {
            metrics.completed(&completed, shape.pcm_bytes())?;
        }
        pending.pop_front();
    }
    if playback {
        while pending.len() < 4 {
            pending.push_back(PendingPcm {
                ticket: pcm.submit_read()?,
                write: None,
            });
        }
    } else if progress.microphone_ready(Instant::now(), pending.len()) {
        let submitted_at = Instant::now();
        let entry = submit_microphone(
            submit_diagnostics,
            WriteTiming {
                origin: submit_origin,
                deadline: progress.next_mic_submit,
                pending: pending.len(),
            },
            || submitted_at,
            collection_qpc,
            || pcm.submit_write(encoded).map_err(anyhow::Error::from),
        )?;
        if let Some(metrics) = shared.as_mut() {
            metrics.submitted(encoded);
        }
        progress.microphone_submitted(submitted_at, shape.frames());
        pending.push_back(entry);
    }
    Ok(())
}

fn collection_qpc() -> Option<u64> {
    let mut qpc = 0;
    // SAFETY: writable stack output on the control-side probe thread. Missing
    // diagnostic timing must not turn an otherwise valid completion into error.
    unsafe { QueryPerformanceCounter(&mut qpc) }.ok()?;
    u64::try_from(qpc).ok().filter(|&qpc| qpc != 0)
}

#[cfg(test)]
mod pacing_tests {
    use super::*;

    #[test]
    fn ac_energy_never_passes_functional_signal_gate_and_failure_receipt_keeps_mode() {
        for mode in [
            NativeMode::SharedAcDiagnostic,
            NativeMode::SharedRawAcDiagnostic,
        ] {
            assert!(includes_endpoint(
                mode,
                Intent::Microphone,
                Channels::Stereo
            ));
            assert!(!includes_endpoint(mode, Intent::Playback, Channels::Stereo));
            assert!(
                require_shared_shape(mode, Some(SampleFormat::Float32), Channels::Stereo, 480)
                    .is_err()
            );
            for signal_frames in [0, 96_000] {
                let seen = Observation {
                    signal_frames,
                    silent_frames: 480,
                };
                let outcome = require_case_signal(mode, &seen, 48_000);
                assert!(
                    outcome
                        .as_ref()
                        .unwrap_err()
                        .to_string()
                        .contains("always unqualified")
                );
                let counters = validation::FailureCounters {
                    mode,
                    signal_frames,
                    silent_frames: 480,
                    microphone_dropped_packets: 0,
                    microphone_completion_diagnostics: Default::default(),
                    microphone_submit_diagnostics: Default::default(),
                    native_buffer_frames: 1056,
                    driver_packet_frames: 480,
                    stream_generation: 1,
                    native_discontinuities: 0,
                    native_position: None,
                    admitted_opened: None,
                    admitted_pcm_shape: None,
                    failure_observation: None,
                };
                let mut receipt = String::new();
                assert!(
                    diagnose_case_failure(
                        outcome,
                        &counters,
                        &PeriodEvidence::new(false),
                        Some(&SharedCaptureDiagnostics::default()),
                        |line| receipt = line.to_owned()
                    )
                    .is_err()
                );
                let receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
                assert_eq!(
                    receipt["counters"]["mode"],
                    serde_json::to_value(mode).unwrap()
                );
                assert_eq!(receipt["counters"]["signal_frames"], signal_frames);
                assert_eq!(
                    receipt["signal_validation"],
                    "unqualified-ac-energy-only; no waveform matching"
                );
            }
        }
        assert!(
            require_case_signal(
                NativeMode::SharedCapture,
                &Observation {
                    signal_frames: 48_000,
                    silent_frames: 0
                },
                48_000
            )
            .is_ok()
        );
    }

    #[test]
    fn raw_shared_capture_keeps_exact_shape_owned_endpoint_and_prestart_write_contract() {
        let mode = NativeMode::SharedRawCapture;
        assert!(includes_endpoint(
            mode,
            Intent::Microphone,
            Channels::Stereo
        ));
        assert!(!includes_endpoint(mode, Intent::Playback, Channels::Stereo));
        assert!(!includes_endpoint(mode, Intent::Microphone, Channels::Mono));
        assert!(
            require_shared_shape(mode, Some(SampleFormat::Pcm16), Channels::Stereo, 480).is_ok()
        );
        for (format, channels, frames) in [
            (None, Channels::Stereo, 480),
            (Some(SampleFormat::Float32), Channels::Stereo, 480),
            (Some(SampleFormat::Pcm16), Channels::Mono, 480),
            (Some(SampleFormat::Pcm16), Channels::Stereo, 144),
        ] {
            assert!(require_shared_shape(mode, format, channels, frames).is_err());
        }
        let calls = std::cell::RefCell::new(Vec::new());
        let pending = start_native_with_queued_pcm(
            Intent::Microphone,
            mode,
            || {
                calls.borrow_mut().push("write");
                Ok(())
            },
            || {
                calls.borrow_mut().push("start");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), ["write", "start"]);
        assert_eq!(pending.len(), 1);
        let now = Instant::now();
        let mut progress = TransferProgress::native_started(now, 480, Intent::Microphone, mode);
        for dropped in [false, true] {
            progress.microphone_completed(now, 480, dropped);
            assert!(progress.microphone_ready(now, 0));
            assert!(!progress.microphone_ready(now, 1));
            progress.microphone_submitted(now, 480);
        }
    }

    #[test]
    fn shared_capture_shape_refuses_any_native_format_channel_or_period_change() {
        assert!(
            require_shared_shape(
                NativeMode::SharedCapture,
                Some(SampleFormat::Pcm16),
                Channels::Stereo,
                480
            )
            .is_ok()
        );
        for (format, channels, frames) in [
            (None, Channels::Stereo, 480),
            (Some(SampleFormat::Float32), Channels::Stereo, 480),
            (Some(SampleFormat::Pcm16), Channels::Mono, 480),
            (Some(SampleFormat::Pcm16), Channels::Stereo, 144),
            (Some(SampleFormat::Pcm16), Channels::Stereo, 0),
        ] {
            assert!(
                require_shared_shape(NativeMode::SharedCapture, format, channels, frames).is_err()
            );
        }
    }

    #[test]
    fn shared_capture_payload_and_terminal_metrics_use_actual_bytes() {
        use moiren_windows_audio::vad::PcmPacketStamp;
        let payload: Vec<u8> = [8192_i16, -16384, 0, i16::MIN]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        let mut metrics = SharedCaptureDiagnostics::default();
        metrics.submitted(&payload);
        assert_eq!(metrics.submitted_packets, 1);
        assert_eq!(metrics.submitted_bytes, 8);
        assert_eq!(metrics.submitted_samples, 4);
        assert_eq!(metrics.submitted_nonzero_samples, 3);
        assert_eq!(metrics.submitted_peak_abs_pcm16, 32768);
        assert_eq!(
            &metrics.first_submitted_samples[..4],
            &[8192, -16384, 0, i16::MIN]
        );
        let stamp = PcmPacketStamp {
            sequence: 1,
            qpc: 100,
        };
        metrics
            .completed(&PcmCompletion::Capture { stamp, bytes: 8 }, 8)
            .unwrap();
        assert!(
            metrics
                .completed(&PcmCompletion::Capture { stamp, bytes: 7 }, 8)
                .is_err()
        );
        metrics
            .completed(&PcmCompletion::CaptureDropped { stamp }, 8)
            .unwrap();
        assert_eq!(metrics.successful_capture_completions, 1);
        assert_eq!(metrics.successful_capture_bytes, 8);
        assert_eq!(metrics.dropped_capture_completions, 1);
    }

    #[test]
    fn shared_capture_dispatch_is_one_owned_stereo_microphone() {
        let endpoints: Vec<_> = [Intent::Playback, Intent::Microphone]
            .into_iter()
            .flat_map(|intent| {
                [Channels::Mono, Channels::Stereo].map(|channels| (intent, channels))
            })
            .filter(|&(intent, channels)| {
                includes_endpoint(NativeMode::SharedCapture, intent, channels)
            })
            .collect();
        assert_eq!(endpoints, [(Intent::Microphone, Channels::Stereo)]);
        for mode in [NativeMode::Event, NativeMode::Timer] {
            assert!(
                [Intent::Playback, Intent::Microphone]
                    .into_iter()
                    .all(|intent| {
                        [Channels::Mono, Channels::Stereo]
                            .into_iter()
                            .all(|channels| includes_endpoint(mode, intent, channels))
                    })
            );
        }
    }

    #[test]
    fn shared_capture_prequeues_exactly_one_write_before_native_start() {
        let calls = std::cell::RefCell::new(Vec::new());
        let pending = start_native_with_queued_pcm(
            Intent::Microphone,
            NativeMode::SharedCapture,
            || {
                calls.borrow_mut().push("write");
                Ok(())
            },
            || {
                calls.borrow_mut().push("start");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), ["write", "start"]);
        assert_eq!(pending.len(), 1);
        let now = Instant::now();
        let progress = TransferProgress::native_started(
            now,
            480,
            Intent::Microphone,
            NativeMode::SharedCapture,
        );
        assert!(progress.microphone_ready(now, 0));
        assert!(!progress.microphone_ready(now, 1));
    }

    #[test]
    fn shared_capture_rearms_immediately_after_success_or_drop_without_a_burst() {
        let origin = Instant::now();
        let mut progress = TransferProgress::native_started(
            origin,
            480,
            Intent::Microphone,
            NativeMode::SharedCapture,
        );
        for dropped in [false, true, false] {
            let completed = origin + Duration::from_millis(13);
            progress.microphone_completed(completed, 480, dropped);
            assert!(progress.microphone_ready(completed, 0));
            assert!(!progress.microphone_ready(completed, 1));
            progress.microphone_submitted(completed, 480);
        }
    }

    #[test]
    fn shared_capture_failure_receipt_retains_silence_malformed_and_payload_evidence() {
        let counters = validation::FailureCounters {
            mode: NativeMode::SharedCapture,
            signal_frames: 0,
            silent_frames: 480,
            microphone_dropped_packets: 1,
            microphone_completion_diagnostics: Default::default(),
            microphone_submit_diagnostics: Default::default(),
            native_buffer_frames: 960,
            driver_packet_frames: 480,
            stream_generation: 7,
            native_discontinuities: 1,
            native_position: None,
            admitted_opened: None,
            admitted_pcm_shape: None,
            failure_observation: None,
        };
        for malformed in [false, true] {
            let mut metrics = SharedCaptureDiagnostics::default();
            metrics.submitted(&[0, 32, 0, 192]);
            let stamp = moiren_windows_audio::vad::PcmPacketStamp {
                sequence: 1,
                qpc: 100,
            };
            metrics
                .completed(&PcmCompletion::Capture { stamp, bytes: 4 }, 4)
                .unwrap();
            let samples: &[f32] = if malformed {
                &[0.75, -0.125]
            } else {
                &[0.0, 0.0]
            };
            metrics.capture_all_phases = Some(native::CaptureDiagnostics {
                packets: 1,
                frames: 1,
                first_flags: Some(if malformed { 1 } else { 2 }),
                flags_or: if malformed { 1 } else { 2 },
                silent_flag_packets: u64::from(!malformed),
                silent_flag_frames: u64::from(!malformed),
                ..Default::default()
            });
            let mut seen = Observation::default();
            let result = native::observe_capture_samples(
                metrics.capture_all_phases.as_mut(),
                samples,
                [0.25, -0.5],
                &mut seen,
            )
            .and_then(|()| seen.require_signal(1));
            let original = format!("{:#}", result.as_ref().unwrap_err());
            let mut receipt = String::new();
            let error = diagnose_case_failure(
                result,
                &counters,
                &PeriodEvidence::new(false),
                Some(&metrics),
                |line| receipt = line.to_owned(),
            )
            .unwrap_err();
            assert_eq!(format!("{error:#}"), original);
            let receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
            assert_eq!(receipt["error"], original);
            assert_eq!(
                receipt["counters"]["shared_capture_diagnostics"],
                serde_json::to_value(&metrics).unwrap()
            );
            let diagnostic = &receipt["counters"]["shared_capture_diagnostics"];
            assert_eq!(diagnostic["submitted_nonzero_samples"], 2);
            assert_eq!(diagnostic["successful_capture_bytes"], 4);
            assert_eq!(
                diagnostic["capture_all_phases"]["nonzero_samples"],
                if malformed { 2 } else { 0 }
            );
            assert_eq!(
                diagnostic["capture_all_phases"]["silent_flag_frames"],
                u64::from(!malformed)
            );
        }
    }

    #[test]
    fn failed_prepare_retains_period_evidence_and_original_error() {
        let counters = validation::FailureCounters {
            mode: NativeMode::Event,
            signal_frames: 0,
            silent_frames: 0,
            microphone_dropped_packets: 0,
            microphone_completion_diagnostics: Default::default(),
            microphone_submit_diagnostics: Default::default(),
            native_buffer_frames: 0,
            driver_packet_frames: 480,
            stream_generation: 7,
            native_discontinuities: 0,
            native_position: None,
            admitted_opened: None,
            admitted_pcm_shape: None,
            failure_observation: None,
        };
        for stage in [
            "before-query",
            "invalid-minimum",
            "initial-refusal",
            "aligned-refusal",
        ] {
            let mut period = PeriodEvidence::new(true);
            let outcome = match stage {
                "before-query" => Err(anyhow::anyhow!(std::io::Error::other("Activate failed"))),
                "invalid-minimum" => period.select(100_000, 0).map(|_| ()),
                _ => {
                    period.select(100_000, 10_000).unwrap();
                    if stage == "aligned-refusal" {
                        period.record_alignment_retry(106_667);
                    }
                    Err(
                        anyhow::anyhow!(std::io::Error::other("native initialization failed"))
                            .context(stage.to_owned()),
                    )
                }
            };
            let original = format!("{:#}", outcome.as_ref().unwrap_err());
            let mut line = String::new();
            let error = diagnose_case_failure(outcome, &counters, &period, None, |value| {
                line = value.to_owned();
            })
            .unwrap_err();
            assert_eq!(format!("{error:#}"), original);
            if stage != "invalid-minimum" {
                assert!(error.downcast_ref::<std::io::Error>().is_some());
            }
            let json: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                json["counters"]["period"],
                serde_json::to_value(period).unwrap()
            );
            assert_eq!(json["diagnostic"], "vad-native-case-failure");
            assert_eq!(json["error"], original);
            assert_eq!(json["counters"]["native_buffer_frames"], 0);
            assert_eq!(json["counters"]["driver_packet_frames"], 480);
        }
        assert_eq!(
            diagnose_case_failure(
                Ok(42),
                &counters,
                &PeriodEvidence::new(false),
                None,
                |_| panic!("success must not emit failure evidence")
            )
            .unwrap(),
            42
        );
    }

    #[test]
    fn successful_completion_retains_intended_halfway_phase_after_delayed_collection() {
        let origin = Instant::now();
        let mut progress =
            TransferProgress::native_started(origin, 480, Intent::Microphone, NativeMode::Event);
        progress.microphone_submitted(origin + Duration::from_millis(5), 480);
        progress.microphone_completed(origin + Duration::from_millis(13), 480, false);
        assert_eq!(
            progress.next_mic_submit,
            Some(origin + Duration::from_millis(15))
        );
        assert!(!progress.microphone_ready(origin + Duration::from_millis(14), 0));
        assert!(progress.microphone_ready(origin + Duration::from_millis(15), 0));
    }

    #[test]
    fn intended_phase_survives_injected_lateness_before_the_native_boundary() {
        let origin = Instant::now();
        let mut progress =
            TransferProgress::native_started(origin, 480, Intent::Microphone, NativeMode::Event);
        progress.microphone_submitted(origin + Duration::from_millis(5), 480);
        progress.microphone_completed(origin + Duration::from_millis(13), 480, false);
        let late_entry = progress.next_mic_submit.unwrap() + Duration::from_micros(2_395);
        let boundary = origin + Duration::from_nanos(20_378_700);
        assert!(
            late_entry < boundary,
            "collection-relative phase misses this boundary"
        );
        assert!(progress.microphone_ready(late_entry, 0));
        let mut diagnostics = validation::MicrophoneSubmitDiagnostics::default();
        let mut qpcs = [Some(173_950), Some(173_970)].into_iter();
        let pending = submit_microphone(
            &mut diagnostics,
            WriteTiming {
                origin,
                deadline: progress.next_mic_submit,
                pending: 0,
            },
            || late_entry,
            || qpcs.next().unwrap(),
            || Ok::<_, anyhow::Error>("one opaque WRITE"),
        )
        .unwrap();
        progress.microphone_submitted(late_entry, 480);
        assert!(!progress.microphone_ready(boundary, 1));
        record_microphone_completion(
            &mut diagnostics,
            &pending,
            &PcmCompletion::Capture {
                stamp: moiren_windows_audio::vad::PcmPacketStamp {
                    sequence: 1553,
                    qpc: 173_960,
                },
                bytes: 960,
            },
        );
        let json = serde_json::to_value(diagnostics).unwrap();
        let record = &json["first_completion"];
        assert_eq!(record["target_deadline_elapsed_ns"], 15_000_000);
        assert_eq!(record["submit_entry_lateness_ns"], 2_395_000);
        // Injected header QPC is distinct from both wrapper samples. At the
        // 20.3787 ms callback it is 2.9827 ms old, below unchanged 10 ms.
        assert_eq!(record["completion"]["header_qpc"], 173_960);
        assert!(203_787u64 - record["completion"]["header_qpc"].as_u64().unwrap() <= 100_000);
    }

    #[test]
    fn overdue_success_is_due_now_once_then_advances_without_catch_up() {
        let origin = Instant::now();
        let mut progress =
            TransferProgress::native_started(origin, 480, Intent::Microphone, NativeMode::Event);
        progress.microphone_submitted(origin + Duration::from_millis(5), 480);
        let collected = origin + Duration::from_millis(38);
        progress.microphone_completed(collected, 480, false);
        assert_eq!(progress.next_mic_submit, Some(collected));
        let mut pending = 0;
        let mut writes = 0;
        for _ in 0..4 {
            if progress.microphone_ready(collected, pending) {
                progress.microphone_submitted(collected, 480);
                pending += 1;
                writes += 1;
            }
        }
        assert_eq!((writes, pending), (1, 1));
        progress.microphone_completed(collected + Duration::from_millis(1), 480, false);
        assert_eq!(
            progress.next_mic_submit,
            Some(collected + Duration::from_millis(10))
        );
    }

    #[test]
    fn submission_one_full_period_late_cannot_carry_a_stale_phase() {
        for late_ms in [10, 25] {
            let origin = Instant::now();
            let mut progress = TransferProgress::native_started(
                origin,
                480,
                Intent::Microphone,
                NativeMode::Event,
            );
            let submitted = origin + Duration::from_millis(5 + late_ms);
            progress.microphone_submitted(submitted, 480);
            let collected = submitted + Duration::from_millis(1);
            progress.microphone_completed(collected, 480, false);
            assert_eq!(
                progress.next_mic_submit,
                Some(collected + Duration::from_millis(5))
            );
        }
    }

    #[test]
    fn submission_retains_phase_only_while_less_than_one_period_late() {
        for (late_micros, retained) in [(9_999, true), (10_000, false), (25_000, false)] {
            let origin = Instant::now();
            let mut progress = TransferProgress::native_started(
                origin,
                480,
                Intent::Microphone,
                NativeMode::Event,
            );
            let phase = origin + Duration::from_millis(5);
            progress.microphone_submitted(phase + Duration::from_micros(late_micros), 480);
            assert_eq!(progress.last_mic_phase, retained.then_some(phase));
            assert_eq!(progress.next_mic_submit, None);
        }
    }

    #[test]
    fn dropped_completion_discards_phase_and_keeps_collection_relative_half_period_rearm() {
        let origin = Instant::now();
        let mut progress =
            TransferProgress::native_started(origin, 480, Intent::Microphone, NativeMode::Event);
        progress.microphone_submitted(origin + Duration::from_millis(5), 480);
        let dropped_at = origin + Duration::from_millis(13);
        progress.microphone_completed(dropped_at, 480, true);
        assert_eq!(progress.last_mic_phase, None);
        assert_eq!(
            progress.next_mic_submit,
            Some(origin + Duration::from_millis(18))
        );
        assert!(!progress.microphone_ready(origin + Duration::from_millis(15), 0));
        assert!(progress.microphone_ready(origin + Duration::from_millis(18), 0));
        progress.microphone_submitted(origin + Duration::from_millis(18), 480);
        progress.microphone_completed(origin + Duration::from_millis(21), 480, false);
        assert_eq!(
            progress.next_mic_submit,
            Some(origin + Duration::from_millis(28))
        );

        // Timer's pre-Start WRITE has no intended phase. Its first successful
        // completion still establishes the old halfway phase after collection.
        let mut timer =
            TransferProgress::native_started(origin, 480, Intent::Microphone, NativeMode::Timer);
        assert!(timer.microphone_ready(origin, 0));
        timer.microphone_completed(origin + Duration::from_millis(2), 480, false);
        assert_eq!(
            timer.next_mic_submit,
            Some(origin + Duration::from_millis(7))
        );
    }

    #[test]
    fn submission_helper_correlates_fifo_record_with_real_completion_not_opaque_ticket() {
        use moiren_windows_audio::vad::PcmPacketStamp;
        let origin = Instant::now();
        let timing = WriteTiming {
            origin,
            deadline: Some(origin + Duration::from_millis(5)),
            pending: 0,
        };
        let mut diagnostics = validation::MicrophoneSubmitDiagnostics::default();
        let mut ticks = [Some(100), Some(140)].into_iter();
        let pending = submit_microphone(
            &mut diagnostics,
            timing,
            || origin + Duration::from_millis(7),
            || ticks.next().unwrap(),
            || Ok::<_, std::io::Error>("opaque ticket, not sequence 991"),
        )
        .unwrap();
        assert_eq!(pending.ticket, "opaque ticket, not sequence 991");
        let record = pending
            .write
            .expect("successful WRITE must retain its submission record");
        assert_eq!(record.submit_before_qpc, Some(100));
        assert_eq!(record.submit_after_qpc, Some(140));
        assert_eq!(record.target_deadline_elapsed_ns, Some(5_000_000));
        assert_eq!(record.submit_entry_lateness_ns, Some(2_000_000));
        assert!(record.completion.is_none());
        let next = submit_microphone(
            &mut diagnostics,
            timing,
            || origin + Duration::from_millis(8),
            || Some(200),
            || Ok::<_, std::io::Error>("another opaque ticket"),
        )
        .unwrap();
        let mut fifo = VecDeque::from([pending, next]);
        record_microphone_completion(
            &mut diagnostics,
            fifo.front().unwrap(),
            &PcmCompletion::CaptureDropped {
                stamp: PcmPacketStamp {
                    sequence: 991,
                    qpc: 117,
                },
            },
        );
        fifo.pop_front();
        let json = serde_json::to_value(&diagnostics).unwrap();
        assert_eq!(
            json["first_completion"]["completion"]["header_sequence"],
            991
        );
        assert_eq!(
            json["first_dropped_completion"]["completion"]["header_qpc"],
            117
        );
        assert_eq!(json["recent"][0]["probe_ticket_id"], 1);
        assert_eq!(json["recent"][0]["pending_before"], 0);
        assert_eq!(json["recent"][0]["pending_after"], 1);
        record_microphone_completion(
            &mut diagnostics,
            fifo.front().unwrap(),
            &PcmCompletion::Capture {
                stamp: PcmPacketStamp {
                    sequence: 3,
                    qpc: 202,
                },
                bytes: 960,
            },
        );
        fifo.pop_front();
        let json = serde_json::to_value(&diagnostics).unwrap();
        assert_eq!(json["recent"][1]["probe_ticket_id"], 2);
        assert_eq!(json["recent"][1]["completion"]["header_sequence"], 3);
        assert_eq!(
            json["first_completion"]["completion"]["header_sequence"],
            991
        );
        record_microphone_completion(
            &mut diagnostics,
            &pending,
            &PcmCompletion::Render {
                stamp: PcmPacketStamp {
                    sequence: 1,
                    qpc: 2,
                },
                pcm: &[],
            },
        );
        assert_eq!(serde_json::to_value(&diagnostics).unwrap(), json);
    }

    #[test]
    fn submission_helper_keeps_original_error_even_when_diagnostic_samples_are_unavailable() {
        let origin = Instant::now();
        let mut diagnostics = validation::MicrophoneSubmitDiagnostics::default();
        let original = anyhow::Error::new(std::io::Error::from_raw_os_error(5))
            .context("original WRITE failure");
        let identity = original.downcast_ref::<std::io::Error>().unwrap() as *const _;
        let error = submit_microphone::<(), _>(
            &mut diagnostics,
            WriteTiming {
                origin,
                deadline: None,
                pending: 0,
            },
            || origin,
            || None,
            || Err(original),
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap() as *const _,
            identity
        );
        let json = serde_json::to_value(&diagnostics).unwrap();
        assert_eq!(json["recent"][0]["submit_succeeded"], false);
        assert_eq!(json["recent"][0]["pending_after"], 0);
        assert!(json["recent"][0]["submit_before_qpc"].is_null());
        assert!(json["recent"][0]["completion"].is_null());
        assert!(json["first_completion"].is_null());
        let pending = submit_microphone(
            &mut diagnostics,
            WriteTiming {
                origin,
                deadline: None,
                pending: 0,
            },
            || origin,
            || None,
            || Ok::<_, anyhow::Error>(42),
        )
        .unwrap();
        assert_eq!(pending.ticket, 42);
        assert!(pending.write.unwrap().submit_before_qpc.is_none());
    }

    #[test]
    fn native_start_follows_the_required_prequeued_requests_for_each_mode() {
        use std::cell::RefCell;

        for (intent, mode, expected) in [
            (Intent::Playback, NativeMode::Event, 4),
            (Intent::Playback, NativeMode::Timer, 4),
            (Intent::Microphone, NativeMode::Event, 0),
            (Intent::Microphone, NativeMode::Timer, 1),
        ] {
            let events = RefCell::new(Vec::new());
            let pending = start_native_with_queued_pcm(
                intent,
                mode,
                || {
                    events.borrow_mut().push("submit");
                    Ok(())
                },
                || {
                    events.borrow_mut().push("start");
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(pending.len(), expected, "{intent:?} {mode:?}");
            let events = events.into_inner();
            assert_eq!(events.last(), Some(&"start"));
            assert_eq!(&events[..expected], vec!["submit"; expected]);
        }
    }

    #[test]
    fn prequeue_failure_never_starts_native() {
        use std::cell::Cell;

        let started = Cell::new(false);
        let error = start_native_with_queued_pcm::<()>(
            Intent::Microphone,
            NativeMode::Timer,
            || anyhow::bail!("submit failed"),
            || {
                started.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "submit failed");
        assert!(!started.get());
    }

    #[test]
    fn timer_microphone_first_write_is_ready_before_the_first_native_segment() {
        let acknowledged = Instant::now();
        // The VM timer callbacks begin the first logical packet at about
        // Run+1.5 ms, rather than consuming it at an event packet boundary.
        for frames in [512, 1024, 2048] {
            let progress = TransferProgress::native_started(
                acknowledged,
                frames,
                Intent::Microphone,
                NativeMode::Timer,
            );
            assert!(progress.microphone_ready(acknowledged, 0));
            assert!(progress.microphone_ready(acknowledged + Duration::from_micros(1_000), 0));
            assert!(!progress.microphone_ready(acknowledged, 1));
        }
    }

    #[test]
    fn microphone_first_write_keeps_freshness_headroom_at_the_observed_late_callback() {
        let run = Instant::now();
        // Actual mono Float32 failure: WRITE at Run+0.3513 ms was rejected at
        // Run+11.0159 ms. Even the latest possible Start acknowledgement,
        // bounded by that old WRITE, must leave room before the first packet.
        let acknowledged = run + Duration::from_nanos(351_300);
        let progress = TransferProgress::native_started(
            acknowledged,
            480,
            Intent::Microphone,
            NativeMode::Event,
        );
        assert!(!progress.microphone_ready(acknowledged, 0));
        assert!(!progress.microphone_ready(acknowledged + Duration::from_millis(4), 0));
        let submitted = acknowledged + Duration::from_millis(5);
        assert!(progress.microphone_ready(submitted, 0));
        assert!(!progress.microphone_ready(submitted, 1));
        let callback = run + Duration::from_nanos(11_015_900);
        assert_eq!(callback - submitted, Duration::from_nanos(5_664_600));
        assert!(callback - submitted < Duration::from_millis(10));
        // A later Running QUERY must not start another half-period delay.
        assert!(progress.microphone_ready(acknowledged + Duration::from_millis(6), 0));
    }

    #[test]
    fn successful_report_retains_first_capture_and_first_drop_without_packet_history() {
        use moiren_windows_audio::vad::PcmPacketStamp;

        let mut diagnostics = validation::MicrophoneCompletionDiagnostics::default();
        diagnostics.observe(
            &PcmCompletion::Render {
                stamp: PcmPacketStamp {
                    sequence: 999,
                    qpc: 999,
                },
                pcm: &[],
            },
            || panic!("render is not a microphone completion"),
        );
        let qpc = (1_u64 << 53) + 17;
        diagnostics.observe(
            &PcmCompletion::Capture {
                stamp: PcmPacketStamp { sequence: 73, qpc },
                bytes: 960,
            },
            || Some(qpc + 101),
        );
        diagnostics.observe(
            &PcmCompletion::Capture {
                stamp: PcmPacketStamp {
                    sequence: 74,
                    qpc: qpc + 10,
                },
                bytes: 960,
            },
            || panic!("a later successful completion needs no QPC sample"),
        );
        let before_drop = serde_json::to_value(diagnostics).unwrap();
        assert_eq!(before_drop["completion_count"], 2);
        assert_eq!(before_drop["first_completion"]["ordinal"], 1);
        assert!(before_drop["first_dropped_completion"].is_null());
        diagnostics.observe(
            &PcmCompletion::CaptureDropped {
                stamp: PcmPacketStamp {
                    sequence: 80,
                    qpc: qpc + 20,
                },
            },
            || Some(qpc + 202),
        );
        for sequence in 81..4_177 {
            diagnostics.observe(
                &PcmCompletion::CaptureDropped {
                    stamp: PcmPacketStamp {
                        sequence,
                        qpc: qpc + sequence,
                    },
                },
                || panic!("the two retained records must never be replaced"),
            );
        }
        let mut period = PeriodEvidence::new(true);
        period.select(100_000, 10_000).unwrap();
        period.record_alignment_retry(106_667);
        let mut report = CaseReport {
            stable_id: "owned endpoint".into(),
            mmdevice_id: "opaque MMDevice".into(),
            adapter_instance: "owned adapter".into(),
            intent: "Microphone".into(),
            channels: 1,
            format: "Pcm16".into(),
            period,
            native_buffer_frames: 480,
            driver_packet_frames: 480,
            stream_generation: 7,
            signal_seconds: 1,
            observation: Observation {
                signal_frames: 47_040,
                silent_frames: 480,
            },
            closed_gate_before: None,
            closed_gate_after: None,
            microphone_dropped_packets: 4_097,
            microphone_completion_diagnostics: diagnostics,
            microphone_submit_diagnostics: Default::default(),
            native_discontinuities: 0,
            native_position: None,
            shared_capture_diagnostics: None,
        };
        // Capacity and the logical packet can differ, or coincide after alignment.
        // Neither is the accepted request, nor proof of measured stream cadence.
        for capacity in [2_048, 480] {
            report.native_buffer_frames = capacity;
            let json = serde_json::to_value(&report).unwrap();
            assert_eq!(json["native_buffer_frames"], capacity);
            assert_eq!(json["driver_packet_frames"], 480);
            assert_eq!(json["period"]["initial_request_hns"], 10_000);
            assert_eq!(json["period"]["final_request_hns"], 106_667);
            assert_eq!(json["period"]["qualification"], "unqualified-request-only");
        }
        let outcome = validation::diagnose_failure(
            Ok(report),
            &validation::FailureCounters {
                mode: NativeMode::Event,
                signal_frames: 0,
                silent_frames: 0,
                microphone_dropped_packets: 0,
                microphone_completion_diagnostics: Default::default(),
                microphone_submit_diagnostics: Default::default(),
                native_buffer_frames: 0,
                driver_packet_frames: 0,
                stream_generation: 0,
                native_discontinuities: 0,
                native_position: None,
                admitted_opened: None,
                admitted_pcm_shape: None,
                failure_observation: None,
            },
            |_| panic!("success must retain diagnostics in the report, not emit failure logging"),
        )
        .unwrap();
        let json = serde_json::to_value(outcome).unwrap();
        let diagnostics = &json["microphone_completion_diagnostics"];
        assert_eq!(diagnostics["completion_count"], 4_099);
        assert_eq!(diagnostics["ordinal_overflow"], false);
        assert_eq!(
            diagnostics["first_completion"],
            serde_json::json!({
                "status": "capture", "ordinal": 1, "stamp_sequence": 73,
                "stamp_qpc": qpc, "collected_qpc": qpc + 101,
            })
        );
        assert_eq!(
            diagnostics["first_dropped_completion"],
            serde_json::json!({
                "status": "capture_dropped", "ordinal": 3, "stamp_sequence": 80,
                "stamp_qpc": qpc + 20, "collected_qpc": qpc + 202,
            })
        );
        // A fresh case cannot inherit either identity or the previous ordinal.
        let fresh =
            serde_json::to_value(validation::MicrophoneCompletionDiagnostics::default()).unwrap();
        assert!(fresh["first_completion"].is_null());
        assert!(fresh["first_dropped_completion"].is_null());
        assert_eq!(fresh["completion_count"], 0);
        let writes = &json["microphone_submit_diagnostics"];
        assert_eq!(writes["capacity"], 64);
        assert_eq!(writes["recent"], serde_json::json!([]));
    }

    #[test]
    fn microphone_keeps_one_pending_write_and_rearms_after_completion() {
        let now = Instant::now();
        let mut progress =
            TransferProgress::native_started(now, 480, Intent::Microphone, NativeMode::Event);
        assert!(!progress.microphone_ready(now, 0));
        assert!(progress.microphone_ready(now + Duration::from_millis(5), 0));
        let completed = now + Duration::from_millis(10);
        progress.microphone_completed(completed, 480, false);
        let deadline = now + Duration::from_millis(15);
        assert_eq!(progress.next_mic_submit, Some(deadline));
        for milliseconds in 10..15 {
            assert!(!progress.microphone_ready(now + Duration::from_millis(milliseconds), 0));
        }
        assert!(progress.microphone_ready(deadline, 0));
        assert!(!progress.microphone_ready(deadline, 1));
        assert!(!progress.microphone_ready(deadline, 4));
    }

    #[test]
    fn microphone_each_completion_rearms_from_collection_time_without_catch_up() {
        let now = Instant::now();
        let mut progress = TransferProgress::default();
        progress.microphone_completed(now, 49, false);
        let half_period = Duration::from_nanos(510_416);
        assert_eq!(progress.next_mic_submit, Some(now + half_period));
        assert!(!progress.microphone_ready(now + half_period - Duration::from_nanos(1), 0));
        assert!(progress.microphone_ready(now + half_period, 0));
        let later = now + Duration::from_secs(1);
        progress.microphone_completed(later, 4800, false);
        assert_eq!(
            progress.next_mic_submit,
            Some(later + Duration::from_millis(50))
        );
        assert!(!progress.microphone_ready(later, 0));
        assert!(progress.microphone_ready(later + Duration::from_millis(50), 0));
    }
}
