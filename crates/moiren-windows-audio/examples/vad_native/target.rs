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
    },
    core::HSTRING,
};

#[path = "identity.rs"]
mod identity;
#[path = "native.rs"]
mod native;

const CONTROL_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Default)]
struct TransferProgress {
    playback: PacketProgress,
    microphone_drops: u64,
    next_mic_submit: Option<Instant>,
}

impl TransferProgress {
    fn microphone_completed(&mut self, now: Instant, frames: u32) {
        // Probe policy: keep one write pending and shift its replacement by
        // half a period after completion; driver freshness stays unchanged.
        let half_period = Duration::from_nanos(u64::from(frames) * 1_000_000_000 / (48_000 * 2));
        self.next_mic_submit = Some(now + half_period);
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
    native_discontinuities: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_position: Option<validation::PositionProgress>,
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    native_mode: NativeMode,
    root_instance: String,
    defaults_before: Vec<Option<String>>,
    defaults_after: Vec<Option<String>>,
    cases: Vec<CaseReport>,
}

pub(super) fn exercise(seconds: u32, mode: NativeMode) -> anyhow::Result<()> {
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
        native_buffer_frames: 0,
        driver_packet_frames: endpoint.packet_frames(),
        stream_generation: endpoint.stream_generation(),
        native_discontinuities: 0,
        native_position: None,
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
        native.start()?;
        let mut snapshot = endpoint;
        let deadline = Instant::now() + CONTROL_DEADLINE;
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
        let before = if endpoint.intent() == Intent::Microphone {
            Some(observe_closed(&mut native, pattern)?)
        } else {
            None
        };
        let opened = control.open_gate(snapshot)?;
        counters.stream_generation = opened.stream_generation();
        counters.driver_packet_frames = opened.packet_frames();
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
        counters.driver_packet_frames = shape.frames();
        counters.stream_generation = opened.stream_generation();
        let mut seen = Observation::default();
        let mut progress = TransferProgress::default();
        let outcome = (|| -> anyhow::Result<_> {
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
            let mut pending = VecDeque::with_capacity(4);
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
        // Cut admission first, then drain every outstanding direct request before
        // closing its native stream, including phase and revocation failures.
        let outcome = validation::revoke_and_drain(
            outcome,
            || revoke(control, opened),
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
            stream_generation: opened.stream_generation(),
            signal_seconds: seconds,
            observation: seen,
            closed_gate_before: before,
            closed_gate_after: after,
            microphone_dropped_packets: progress.microphone_drops,
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

#[cfg(test)]
mod pacing_tests {
    use super::*;

    #[test]
    fn microphone_first_write_is_immediate_then_waits_half_a_period() {
        let now = Instant::now();
        let mut progress = TransferProgress::default();
        assert!(progress.microphone_ready(now, 0));
        assert!(!progress.microphone_ready(now, 1));
        progress.microphone_completed(now, 480);
        let deadline = now + Duration::from_millis(5);
        assert_eq!(progress.next_mic_submit, Some(deadline));
        for milliseconds in 0..5 {
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
