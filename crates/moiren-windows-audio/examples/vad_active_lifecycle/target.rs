use crate::validation::{self, Candidate};
use anyhow::{Context, ensure};
use moiren_windows_audio::vad::{
    Channels, ControlClient, EndpointSnapshot, Intent, PdoState, StableEndpointId, StreamState,
    discover_present_interfaces,
};
use serde::Serialize;
use std::{
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

#[path = "capture.rs"]
mod capture;
#[path = "../vad_native/identity.rs"]
mod identity;

const DEADLINE: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(25);
const STABLE_ID: StableEndpointId = StableEndpointId::new(2 | (2 << 32), 0x4d6f_6972_4143_544c);

struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn new() -> anyhow::Result<Self> {
        // SAFETY: dedicated example thread, balanced after all COM owners drop.
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
struct SnapshotReport {
    state: String,
    runtime_pdo: u64,
    session_epoch: u64,
    endpoint_generation: u64,
    stream_generation: u64,
    stream_state: String,
    gate_open: bool,
    gate_epoch: u64,
}
impl From<EndpointSnapshot> for SnapshotReport {
    fn from(value: EndpointSnapshot) -> Self {
        Self {
            state: format!("{:?}", value.state()),
            runtime_pdo: value.pdo_token().get(),
            session_epoch: value.session_epoch(),
            endpoint_generation: value.endpoint_generation(),
            stream_generation: value.stream_generation(),
            stream_state: format!("{:?}", value.stream_state()),
            gate_open: value.gate_open(),
            gate_epoch: value.gate_epoch(),
        }
    }
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    probe: &'static str,
    scope: &'static str,
    pcm_qualified: bool,
    silence_qualified: bool,
    crash_cleanup_qualified: bool,
    default_policy_qualified: bool,
    allow_default: bool,
    stable_id: String,
    root_instance: String,
    requested_hold_seconds: u32,
    defaults_before: Vec<Option<String>>,
    defaults_after: Option<Vec<Option<String>>>,
    mmdevice_id: Option<String>,
    adapter_instance: Option<String>,
    native_buffer_frames: Option<u32>,
    running: Option<SnapshotReport>,
    gate_open: Option<SnapshotReport>,
    gate_closed_without_native_stop: Option<SnapshotReport>,
    remove_accepted: Option<SnapshotReport>,
    while_client_held: Option<SnapshotReport>,
    removing_observed_while_held: bool,
    removed_while_client_held: bool,
    hold_observation_ms: u64,
    held_native_error: Option<String>,
    stop_after_remove_error: Option<String>,
    after_client_release: Option<SnapshotReport>,
    recreated: Option<SnapshotReport>,
    recreated_mmdevice_id: Option<String>,
    recreated_removed: Option<SnapshotReport>,
    status: &'static str,
    error: Option<String>,
}

pub(super) fn exercise(hold_seconds: u32) -> anyhow::Result<()> {
    let interfaces = discover_present_interfaces()?;
    ensure!(
        interfaces.len() == 1,
        "requires exactly one installed VAD control root"
    );
    let _apartment = Apartment::new()?;
    // SAFETY: new apartment-local enumerator; discovery never chooses defaults.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let root = identity::root_instance(&interfaces[0])?;
    let mut report = Report {
        schema_version: 1,
        probe: "vad-active-lifecycle-stage1",
        scope: "active-remove-only; NOT PCM/silence/crash qualification",
        pcm_qualified: false,
        silence_qualified: false,
        crash_cleanup_qualified: false,
        default_policy_qualified: false,
        allow_default: false,
        stable_id: STABLE_ID.to_string(),
        root_instance: root,
        requested_hold_seconds: hold_seconds,
        defaults_before: defaults(&enumerator)?,
        defaults_after: None,
        mmdevice_id: None,
        adapter_instance: None,
        native_buffer_frames: None,
        running: None,
        gate_open: None,
        gate_closed_without_native_stop: None,
        remove_accepted: None,
        while_client_held: None,
        removing_observed_while_held: false,
        removed_while_client_held: false,
        hold_observation_ms: 0,
        held_native_error: None,
        stop_after_remove_error: None,
        after_client_release: None,
        recreated: None,
        recreated_mmdevice_id: None,
        recreated_removed: None,
        status: "incomplete",
        error: None,
    };
    // Exactly one owning file object is retained through removal and recreate.
    // Any error drops the native client inside active_remove before this owner.
    let mut control = ControlClient::connect(&interfaces[0])?;
    control.capabilities().require_pcm()?;
    let result = active_remove(&mut control, &enumerator, &mut report);
    let defaults_after = defaults(&enumerator);
    let result = result.and_then(|()| {
        let defaults_after = defaults_after
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        require_not_default(defaults_after, report.mmdevice_id.as_deref())
    });
    if let Ok(after) = defaults_after {
        report.defaults_after = Some(after);
    }
    if let Err(error) = &result {
        report.status = "failed-or-bounded-incomplete";
        report.error = Some(format!("{error:#}"));
    } else {
        report.status = "stage1-active-remove-pass-not-pcm-qualified";
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    result
}

fn active_remove(
    control: &mut ControlClient,
    enumerator: &IMMDeviceEnumerator,
    report: &mut Report,
) -> anyhow::Result<()> {
    let created = create_microphone(control)?;
    let mapping = wait_mapping(enumerator, &report.root_instance)?;
    report.mmdevice_id = Some(mapping.id.clone());
    report.adapter_instance = Some(mapping.raw_instance.clone());
    require_not_default(&defaults(enumerator)?, Some(&mapping.id))?;
    // Do not keep an extra IMMDevice activation owner after preparing capture.
    let mut capture = {
        let device = unsafe { enumerator.GetDevice(&HSTRING::from(&mapping.id))? };
        let rechecked = identity::candidate(enumerator, &device, Intent::Microphone)?;
        ensure!(
            validation::select(&[rechecked], STABLE_ID, &report.root_instance)?.is_some(),
            "MMDevice ownership changed before native activation"
        );
        capture::Capture::prepare(&device)?
    };
    report.native_buffer_frames = Some(capture.frames);
    capture.start()?;
    let running = wait_running(control, created)?;
    report.running = Some(running.into());
    let opened = control.open_gate(running)?;
    report.gate_open = Some(opened.into());
    // No Engine PCM writer exists in stage 1. Discarding native packets only
    // services the live client; neither zero bytes nor silence flags qualify it.
    let settle = Instant::now() + Duration::from_millis(200);
    while Instant::now() < settle {
        capture.poll_discard()?;
        std::thread::sleep(Duration::from_millis(2));
    }
    let latest = control
        .query(opened.key())?
        .require_progress_after(opened)?;
    let closed = control.close_gate(latest)?;
    ensure!(
        closed.stream_state() == StreamState::Running
            && closed.stream_generation() == running.stream_generation(),
        "gate close stopped or replaced the original native RUN"
    );
    report.gate_closed_without_native_stop = Some(closed.into());
    marker("before-active-remove", closed)?;
    let mut previous = control
        .remove(closed.key())?
        .require_progress_after(closed)?;
    validation::require_retiring(previous.state(), previous.gate_open())?;
    report.remove_accepted = Some(previous.into());
    marker("remove-accepted-client-still-held", previous)?;
    let began = Instant::now();
    let until = began + Duration::from_secs(u64::from(report.requested_hold_seconds));
    loop {
        validation::require_retiring(previous.state(), previous.gate_open())?;
        report.removing_observed_while_held |= previous.state() == PdoState::Removing;
        report.removed_while_client_held = previous.state() == PdoState::Removed;
        report.while_client_held = Some(previous.into());
        if previous.state() == PdoState::Removed || Instant::now() >= until {
            break;
        }
        if report.held_native_error.is_none()
            && let Err(error) = capture.poll_discard()
        {
            // Device invalidation is evidence, not permission to replace the
            // held client, refresh a session, or invent final PDO destruction.
            report.held_native_error = Some(format!("{error:#}"));
        }
        std::thread::sleep(POLL.min(until.saturating_duration_since(Instant::now())));
        previous = control
            .query(previous.key())?
            .require_progress_after(previous)?;
    }
    report.hold_observation_ms = began.elapsed().as_millis() as u64;
    marker("releasing-original-native-client", previous)?;
    if let Err(error) = capture.stop() {
        report.stop_after_remove_error = Some(format!("{error:#}"));
    }
    drop(capture); // Release all IAudioClient/service/event owners before polling.
    let removed = wait_removed(control, previous)
        .context("bounded removal after releasing original client; no recreate attempted")?;
    report.after_client_release = Some(removed.into());
    let recreated = create_microphone(control)?;
    report.recreated = Some(recreated.into());
    validation::require_fresh_runtime(removed.key(), recreated.key())?;
    let recreated_mapping = wait_mapping(enumerator, &report.root_instance)?;
    report.recreated_mmdevice_id = Some(recreated_mapping.id.clone());
    require_not_default(&defaults(enumerator)?, Some(&recreated_mapping.id))?;
    // Exercise the same stable ID only after confirmed destruction. No second
    // control connect, next session, native fallback, or stale token is used.
    let final_removed = control.remove_and_wait(recreated.key(), DEADLINE)?;
    ensure!(
        final_removed.removal_complete_after(recreated)?,
        "recreated endpoint not destroyed"
    );
    report.recreated_removed = Some(final_removed.into());
    Ok(())
}

fn create_microphone(control: &mut ControlClient) -> anyhow::Result<EndpointSnapshot> {
    Ok(control
        .create_and_wait(
            STABLE_ID,
            Intent::Microphone,
            Channels::Stereo,
            "Moiren active lifecycle stage1",
            false,
            DEADLINE,
        )?
        .require_created(STABLE_ID, Intent::Microphone, Channels::Stereo)?)
}

fn wait_mapping(enumerator: &IMMDeviceEnumerator, root: &str) -> anyhow::Result<Candidate> {
    let until = Instant::now() + DEADLINE;
    loop {
        let (rows, errors) = identity::candidates(enumerator)?;
        if let Some(row) = validation::select(&rows, STABLE_ID, root)? {
            return Ok(row.clone());
        }
        ensure!(
            Instant::now() < until,
            "owned MMDevice not published; diagnostics={errors:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait_running(
    control: &mut ControlClient,
    mut previous: EndpointSnapshot,
) -> anyhow::Result<EndpointSnapshot> {
    let until = Instant::now() + DEADLINE;
    loop {
        previous = control
            .query(previous.key())?
            .require_progress_after(previous)?;
        ensure!(
            previous.state() == PdoState::Present && !previous.gate_open(),
            "unexpected prepare state"
        );
        if previous.stream_state() == StreamState::Running && previous.audio_ready() {
            return Ok(previous);
        }
        ensure!(
            Instant::now() < until,
            "native capture never reached driver RUN"
        );
        std::thread::sleep(POLL);
    }
}

fn wait_removed(
    control: &mut ControlClient,
    mut previous: EndpointSnapshot,
) -> anyhow::Result<EndpointSnapshot> {
    let until = Instant::now() + DEADLINE;
    loop {
        validation::require_retiring(previous.state(), previous.gate_open())?;
        if previous.state() == PdoState::Removed {
            return Ok(previous);
        }
        ensure!(
            Instant::now() < until,
            "old endpoint remains Removing after client release"
        );
        std::thread::sleep(POLL);
        previous = control
            .query(previous.key())?
            .require_progress_after(previous)?;
    }
}

fn defaults(enumerator: &IMMDeviceEnumerator) -> anyhow::Result<Vec<Option<String>>> {
    let mut rows = Vec::with_capacity(6);
    for flow in [eRender, eCapture] {
        for role in [eConsole, eMultimedia, eCommunications] {
            // SAFETY: read-only role queries; no policy configuration interface.
            match unsafe { enumerator.GetDefaultAudioEndpoint(flow, role) } {
                Ok(device) => rows.push(Some(identity::take_string(unsafe { device.GetId()? })?)),
                Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => rows.push(None),
                Err(error) => return Err(error).context("GetDefaultAudioEndpoint"),
            }
        }
    }
    Ok(rows)
}

fn require_not_default(defaults: &[Option<String>], owned: Option<&str>) -> anyhow::Result<()> {
    if let Some(owned) = owned {
        ensure!(
            defaults.iter().flatten().all(|id| id != owned),
            "owned probe became a default endpoint"
        );
    }
    Ok(())
}

fn marker(stage: &str, snapshot: EndpointSnapshot) -> anyhow::Result<()> {
    eprintln!(
        "active-lifecycle stage={stage} runtime={} session={} state={:?} stream={:?}",
        snapshot.pdo_token(),
        snapshot.session_epoch(),
        snapshot.state(),
        snapshot.stream_state()
    );
    std::io::stderr().flush()?;
    Ok(())
}
