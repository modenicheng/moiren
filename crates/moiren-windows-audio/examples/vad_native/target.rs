use crate::{
    NativeMode,
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

fn start_native_with_queued_pcm<T>(
    intent: Intent,
    mode: NativeMode,
    mut submit: impl FnMut() -> anyhow::Result<T>,
    start: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<VecDeque<T>> {
    let mut pending = VecDeque::with_capacity(4);
    let count = match (intent, mode) {
        (Intent::Playback, _) => 4,
        (Intent::Microphone, NativeMode::Timer) => 1,
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
}

impl TransferProgress {
    fn native_started(now: Instant, frames: u32, intent: Intent, mode: NativeMode) -> Self {
        Self {
            // Event capture consumes at the packet boundary; timer capture
            // consumes at its first segment and needs the WRITE immediately.
            // Anchor event headroom before QUERY so it cannot re-arm the delay.
            next_mic_submit: (intent == Intent::Microphone && mode == NativeMode::Event)
                .then(|| Self::microphone_deadline(now, frames)),
            ..Self::default()
        }
    }

    fn microphone_deadline(now: Instant, frames: u32) -> Instant {
        let half_period = Duration::from_nanos(u64::from(frames) * 1_000_000_000 / (48_000 * 2));
        now + half_period
    }

    fn microphone_completed(&mut self, now: Instant, frames: u32) {
        // Probe policy: keep one write pending and shift its replacement by
        // half a period after completion; driver freshness stays unchanged.
        self.next_mic_submit = Some(Self::microphone_deadline(now, frames));
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
    native_buffer_frames: u32,
    driver_packet_frames: u32,
    stream_generation: u64,
    signal_seconds: u32,
    observation: Observation,
    closed_gate_before: Option<Observation>,
    closed_gate_after: Option<Observation>,
    microphone_dropped_packets: u64,
    microphone_completion_diagnostics: validation::MicrophoneCompletionDiagnostics,
    native_discontinuities: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_position: Option<validation::PositionProgress>,
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    native_mode: NativeMode,
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
) -> anyhow::Result<()> {
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
                eprintln!(
                    "native case mode={mode:?} intent={:?} channels={:?} format={format:?} id={}",
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
            schema_version: 2,
            native_mode: mode,
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
) -> anyhow::Result<CaseReport> {
    let mut counters = validation::FailureCounters {
        mode,
        signal_frames: 0,
        silent_frames: 0,
        microphone_dropped_packets: 0,
        microphone_completion_diagnostics: Default::default(),
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
    );
    validation::diagnose_failure(outcome, &counters, |line| {
        // A failed stderr write must not replace the original case error.
        let _ = writeln!(std::io::stderr().lock(), "{line}");
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
        let mut decoded = vec![0.0; shape.frames() as usize * 2];
        // Timer capture consumes at the first sub-packet segment. Prepare its
        // single WRITE while Paused, not after Start/QUERY; event capture must
        // instead retain the half-period freshness headroom below.
        let mut pending = start_native_with_queued_pcm(
            endpoint.intent(),
            mode,
            || {
                Ok(if endpoint.intent() == Intent::Playback {
                    pcm.submit_read()?
                } else {
                    pcm.submit_write(&bytes)?
                })
            },
            || native.start(),
        )?;
        let mut progress = TransferProgress::native_started(
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
        let running = snapshot;
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
                )?;
                std::thread::sleep(Duration::from_millis(1));
            }
            seen.require_signal(u64::from(seconds) * 48_000 / 2)?;
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
            native_buffer_frames: native.frames,
            driver_packet_frames: shape.frames(),
            stream_generation: running.stream_generation(),
            signal_seconds: seconds,
            observation: seen,
            closed_gate_before: before,
            closed_gate_after: after,
            microphone_dropped_packets: progress.microphone_drops,
            microphone_completion_diagnostics: progress.microphone_completion_diagnostics,
            native_discontinuities: native.discontinuities,
            native_position: position,
        })
    })();
    // Capture the final native state even for setup, gate-check or Stop errors.
    counters.native_discontinuities = native.discontinuities;
    if mode == NativeMode::Timer {
        counters.native_position = Some(native.position.clone());
    }
    outcome
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

fn pump_pcm(
    pcm: &mut PcmOwner,
    pending: &mut VecDeque<PcmTicket>,
    encoded: &[u8],
    decoded: &mut [f32],
    pattern: [f32; 2],
    seen: &mut Observation,
    progress: &mut TransferProgress,
) -> anyhow::Result<()> {
    let shape = pcm.shape();
    let playback = shape.snapshot().intent() == Intent::Playback;
    while let Some(&ticket) = pending.front() {
        let Some(completed) = pcm.try_complete(ticket)? else {
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
                progress.microphone_completed(Instant::now(), shape.frames());
            }
            PcmCompletion::CaptureDropped { .. } => {
                progress.microphone_drops += 1;
                progress.microphone_completed(Instant::now(), shape.frames());
            }
        }
        pending.pop_front();
    }
    if playback {
        while pending.len() < 4 {
            pending.push_back(pcm.submit_read()?);
        }
    } else if progress.microphone_ready(Instant::now(), pending.len()) {
        pending.push_back(pcm.submit_write(encoded)?);
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
        let report = CaseReport {
            stable_id: "owned endpoint".into(),
            mmdevice_id: "opaque MMDevice".into(),
            adapter_instance: "owned adapter".into(),
            intent: "Microphone".into(),
            channels: 1,
            format: "Pcm16".into(),
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
            native_discontinuities: 0,
            native_position: None,
        };
        let outcome = validation::diagnose_failure(
            Ok(report),
            &validation::FailureCounters {
                mode: NativeMode::Event,
                signal_frames: 0,
                silent_frames: 0,
                microphone_dropped_packets: 0,
                microphone_completion_diagnostics: Default::default(),
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
    }

    #[test]
    fn microphone_keeps_one_pending_write_and_rearms_after_completion() {
        let now = Instant::now();
        let mut progress =
            TransferProgress::native_started(now, 480, Intent::Microphone, NativeMode::Event);
        assert!(!progress.microphone_ready(now, 0));
        assert!(progress.microphone_ready(now + Duration::from_millis(5), 0));
        let completed = now + Duration::from_millis(10);
        progress.microphone_completed(completed, 480);
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
        progress.microphone_completed(now, 49);
        let half_period = Duration::from_nanos(510_416);
        assert_eq!(progress.next_mic_submit, Some(now + half_period));
        assert!(!progress.microphone_ready(now + half_period - Duration::from_nanos(1), 0));
        assert!(progress.microphone_ready(now + half_period, 0));
        let later = now + Duration::from_secs(1);
        progress.microphone_completed(later, 4800);
        assert_eq!(
            progress.next_mic_submit,
            Some(later + Duration::from_millis(50))
        );
        assert!(!progress.microphone_ready(later, 0));
        assert!(progress.microphone_ready(later + Duration::from_millis(50), 0));
    }
}
