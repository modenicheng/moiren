//! Pure text projections from [`AppSnapshot`]. These functions format
//! snapshot state for display; they hold no UI state and dispatch no intent.

use crate::service::{AppSnapshot, DispatchError, InputSelection, SessionPhase};
#[cfg(windows)]
use crate::service::{AudioBackend, SessionGeneration};
use crate::ui::state::{UiFailure, input_rows, output_rows};
use rust_i18n::t;
use std::borrow::Cow;

/// Display-level stale guard mirroring the service's write-side filter. The
/// owner identity is not published to the UI, so this deliberately checks only
/// phase, generation, and the observation's own running flag.
#[cfg(windows)]
fn fresh_observation(
    phase: SessionPhase,
    generation: SessionGeneration,
    status_generation: SessionGeneration,
    running: bool,
) -> bool {
    phase == SessionPhase::Running && generation == status_generation && running
}

/// The running engine's stream observations behind the display-level stale
/// guard ([`fresh_observation`]): `None` when idle, off Windows, or the
/// snapshot predates the current session generation. The one gate every
/// observation reader goes through (design 23 §3.7).
#[cfg(windows)]
pub(crate) fn professional_observations(
    snapshot: &AppSnapshot,
) -> Option<&moiren_windows_audio::engine_backend::ProfessionalObservationSnapshot> {
    let status = snapshot.professional_observations.as_ref()?;
    fresh_observation(
        snapshot.phase,
        snapshot.generation,
        status.generation,
        status.observation.running,
    )
    .then_some(&status.observation)
}

/// What the status bar reads off one observed output stream, reduced to
/// platform-free values so the projections below compile everywhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OutputStream {
    pub node: moiren_core::graph::NodeId,
    /// Whether this output owns the native device clock.
    pub master: bool,
    pub sample_rate: u32,
    pub format: moiren_engine::audio_config::SampleFormat,
    pub buffer_frames: u32,
    pub backend: moiren_engine::audio_config::AudioBackend,
    pub mode: moiren_engine::node::DeviceMode,
    /// ASIO's driver-reported `(input, output)` frames, when the driver
    /// published them.
    pub driver_latency: Option<(u32, u32)>,
}

/// The fresh output streams behind the display-level stale guard, master
/// first. Empty when idle, off Windows, or the observation is stale.
#[cfg(windows)]
pub(crate) fn output_streams(snapshot: &AppSnapshot) -> Vec<OutputStream> {
    let Some(observations) = professional_observations(snapshot) else {
        return Vec::new();
    };
    let mut streams: Vec<OutputStream> = observations
        .streams
        .iter()
        .filter(|stream| !stream.input)
        .map(|stream| OutputStream {
            node: stream.node,
            master: stream.master,
            sample_rate: stream.actual.sample_rate,
            format: stream.actual.format,
            buffer_frames: stream.actual.buffer_frames,
            backend: stream.requested.backend,
            mode: stream.requested.mode,
            driver_latency: stream
                .driver_latency
                .map(|latency| (latency.input_frames, latency.output_frames)),
        })
        .collect();
    streams.sort_by_key(|stream| !stream.master);
    streams
}

#[cfg(not(windows))]
pub(crate) fn output_streams(_: &AppSnapshot) -> Vec<OutputStream> {
    Vec::new()
}

/// The fresh master output stream, when one runs.
pub(crate) fn master_stream(snapshot: &AppSnapshot) -> Option<OutputStream> {
    output_streams(snapshot)
        .into_iter()
        .find(|stream| stream.master)
}

/// A share as the status bar spells it: two decimals, or "<0.01%" while the
/// value is real but rounds to zero — `0.00%` for three XRUNs would read as
/// an exact zero the counters disprove.
pub(crate) fn percent_text(value: f64) -> String {
    if value > 0. && value < 0.01 {
        "<0.01%".into()
    } else {
        format!("{value:.2}%")
    }
}

/// How long ago something happened, for the XRUN summary.
pub(crate) fn ago_text(ago: std::time::Duration) -> String {
    let seconds = ago.as_secs();
    if seconds < 60 {
        t!("%{seconds} s ago", seconds = seconds).to_string()
    } else {
        t!(
            "%{minutes} min %{seconds} s ago",
            minutes = seconds / 60,
            seconds = seconds % 60
        )
        .to_string()
    }
}

/// A plain time span, for the clean-streak readout.
pub(crate) fn span_text(span: std::time::Duration) -> String {
    let seconds = span.as_secs();
    if seconds < 60 {
        t!("%{seconds} s", seconds = seconds).to_string()
    } else {
        t!(
            "%{minutes} min %{seconds} s",
            minutes = seconds / 60,
            seconds = seconds % 60
        )
        .to_string()
    }
}

/// A recording phase as the card and inspector spell it. `None` is the idle
/// recorder with no session yet.
pub(crate) fn recording_phase_text(phase: Option<crate::service::RecordingPhase>) -> String {
    use crate::service::RecordingPhase;
    match phase {
        None => t!("Ready"),
        Some(RecordingPhase::Recording) => t!("Recording"),
        Some(RecordingPhase::Paused) => t!("Paused"),
        Some(RecordingPhase::Rebinding) => t!("Reconnecting"),
        Some(RecordingPhase::Stopping) => t!("Stopping"),
        Some(RecordingPhase::Finished) => t!("Saved"),
        Some(RecordingPhase::Failed) => t!("Failed"),
    }
    .to_string()
}

/// Recorded audio as a clock, `HH:MM:SS`. Pause gaps are already excluded
/// upstream (the engine skips paused frames), so this counts tape, not wall
/// time.
pub(crate) fn recording_duration_text(secs: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// One buffer period as the bar spells it: frames and their duration.
pub(crate) fn buffer_text(frames: u32, sample_rate: u32) -> String {
    if frames == 0 || sample_rate == 0 {
        return "—".into();
    }
    let ms = f64::from(frames) * 1000. / f64::from(sample_rate);
    t!(
        "%{frames} smp / %{ms} ms",
        frames = frames,
        ms = format!("{ms:.1}")
    )
    .to_string()
}

/// The bar's compact rate line: rate plus whatever depth is known — the
/// measured device format while running, the configured engine precision
/// otherwise.
pub(crate) fn format_short_text(rate: u32, depth: Option<Cow<'static, str>>) -> String {
    if rate == 0 {
        return "—".into();
    }
    let khz = format!("{:.1}", f64::from(rate) / 1000.);
    match depth {
        Some(depth) => format!("{khz} kHz / {depth}"),
        None => format!("{khz} kHz"),
    }
}

pub fn phase_text(snapshot: &AppSnapshot) -> Cow<'static, str> {
    match snapshot.phase {
        SessionPhase::Idle => t!("Idle"),
        SessionPhase::Starting => t!("Starting"),
        SessionPhase::Running => t!("Running"),
        SessionPhase::Stopping => t!("Stopping"),
        SessionPhase::Failed => t!("Failed"),
        SessionPhase::Exiting => t!("Exiting"),
        SessionPhase::Exited => t!("Exited"),
    }
}

/// Settings page used to render this directly; the live control-feedback path
/// (`control_feedback`) owns the production wording now, so this stays only as
/// the observation helper for the latency-projection state tests.
#[cfg(test)]
pub fn latency_apply_text(snapshot: &AppSnapshot) -> Cow<'static, str> {
    match snapshot.active_latency_profile {
        Some(active) if active == snapshot.latency_profile => {
            t!("Active profile matches selection")
        }
        _ => t!("Selected profile activates on the next audio start"),
    }
}

/// One live stream as the clock overview and the device monitor render it:
/// identity in the title fields, detail rows in the format the clock tab has
/// always shown (design 23 §3.3/§3.5).
pub struct StreamTimingView {
    pub node: moiren_core::graph::NodeId,
    pub input: bool,
    pub master: bool,
    /// The actual-stream line on its own — the row summary and the monitor
    /// section reuse it without re-parsing the detail rows.
    pub summary: String,
    pub device_title: String,
    pub node_label: String,
    pub rows: Vec<(String, String)>,
}

/// The clock tab's projection: the processing parameters once, then one
/// group per live stream. Only the currently running owner can supply
/// timing — a stopped or replaced owner leaves nothing here rather than
/// plausible-looking stale values.
#[derive(Default)]
pub struct NativeTimingViews {
    pub processing: Option<(String, String)>,
    pub streams: Vec<StreamTimingView>,
}

pub fn native_timing_views(snapshot: &AppSnapshot) -> NativeTimingViews {
    #[cfg(not(windows))]
    {
        let _ = snapshot;
        NativeTimingViews::default()
    }
    #[cfg(windows)]
    {
        let Some(observations) = professional_observations(snapshot) else {
            return NativeTimingViews::default();
        };
        let processing = snapshot.active_audio_settings.as_ref().map(|settings| {
            (
                t!("Processing").to_string(),
                format!(
                    "{} Hz; {}; {} frames",
                    settings.processing.sample_rate,
                    t!(settings.processing.precision.title()),
                    settings.processing.block_frames,
                ),
            )
        });
        let streams = observations
            .streams
            .iter()
            .map(|stream| stream_timing_view(snapshot, stream))
            .collect();
        NativeTimingViews {
            processing,
            streams,
        }
    }
}

#[cfg(windows)]
fn stream_timing_view(
    snapshot: &AppSnapshot,
    stream: &moiren_windows_audio::engine_backend::ProfessionalStreamObservation,
) -> StreamTimingView {
    let summary = format!(
        "{} Hz; {}; {} channels; {} frames ({:.2} ms)",
        stream.actual.sample_rate,
        t!(stream.actual.format.title()),
        stream.actual.channels,
        stream.actual.buffer_frames,
        f64::from(stream.actual.buffer_frames) * 1000.0 / f64::from(stream.actual.sample_rate),
    );
    let mut rows = vec![
        (t!("Actual stream").to_string(), summary.clone()),
        (
            t!("Backend / mode").to_string(),
            match stream.requested.backend {
                AudioBackend::Asio => t!("ASIO driver (driver owns mode)").to_string(),
                AudioBackend::Wasapi => {
                    format!("WASAPI / {}", t!(stream.requested.mode.title()))
                }
            },
        ),
        (
            t!("Requested buffer").to_string(),
            stream
                .requested
                .period_frames
                .map(|frames| format!("{frames} frames"))
                .unwrap_or_else(|| t!("Driver default").to_string()),
        ),
        (
            t!("Bridge counters").to_string(),
            t!(
                "Dropped %{dropped} / silence %{silent} frames",
                dropped = stream.dropped_frames,
                silent = stream.silent_frames,
            )
            .to_string(),
        ),
    ];
    if stream.requested.backend == AudioBackend::Asio {
        rows.push((
            t!("ASIO callback counters").to_string(),
            t!(
                "%{callbacks} callbacks; capture dropped %{dropped} / render silence %{silent} frames (includes startup)",
                callbacks = stream.native_callbacks,
                dropped = stream.native_dropped_frames,
                silent = stream.native_silent_frames,
            )
            .to_string(),
        ));
        rows.push((
            t!("Driver-reported latency").to_string(),
            stream
                .driver_latency
                .map(|latency| {
                    t!(
                        "Input %{input} / output %{output} frames; excludes application queues",
                        input = latency.input_frames,
                        output = latency.output_frames,
                    )
                    .to_string()
                })
                .unwrap_or_else(|| t!("Unavailable (not measured round-trip latency)").to_string()),
        ));
    }
    StreamTimingView {
        node: stream.node,
        input: stream.input,
        master: stream.master,
        device_title: stream_device_title(snapshot, stream),
        node_label: snapshot
            .graph
            .node(stream.node)
            .map(display_label)
            .unwrap_or_else(|| format!("{:?}", stream.node)),
        summary,
        rows,
    }
}

impl StreamTimingView {
    /// The stream's clock role, localized: input, master output, or
    /// follower output — the third title segment of a stream group.
    pub fn role(&self) -> Cow<'static, str> {
        if self.input {
            t!("Input")
        } else if self.master {
            t!("Master output")
        } else {
            t!("Follower output")
        }
    }
}

/// The display name of a stream's endpoint: the catalog row's name for a
/// physical device, the process name for a loopback, the virtual device's
/// own name. Falls back to the raw identity when the row has since left
/// the catalog.
#[cfg(windows)]
fn stream_device_title(
    snapshot: &AppSnapshot,
    stream: &moiren_windows_audio::engine_backend::ProfessionalStreamObservation,
) -> String {
    use moiren_windows_audio::engine_backend::StreamEndpoint;
    match &stream.endpoint {
        StreamEndpoint::Physical(id) => {
            let catalog = &snapshot.catalog;
            catalog
                .inputs
                .iter()
                .chain(catalog.outputs.iter())
                .chain(catalog.asio_drivers.iter())
                .find(|row| &row.endpoint_id == id)
                .map(|row| row.name.clone())
                .unwrap_or_else(|| id.clone())
        }
        StreamEndpoint::Application {
            pid,
            creation_time_100ns,
        } => snapshot
            .catalog
            .processes
            .iter()
            .find(|row| row.pid == *pid && row.creation_time_100ns == *creation_time_100ns)
            .map(|row| row.name.clone())
            .unwrap_or_else(|| format!("PID {pid}")),
        StreamEndpoint::Virtual(endpoint_id) => snapshot
            .exports
            .iter()
            .find(|status| status.intent.endpoint_id == *endpoint_id)
            .map(|status| status.intent.name.clone())
            .unwrap_or_else(|| endpoint_id.to_string()),
    }
}

/// A linear gain as a signed dB readout; exact digital silence is −∞ dB.
/// Shared by every fader/route/inspector readout so one spelling of silence
/// and one decimal shape appear everywhere.
pub fn db_text(gain: f64) -> String {
    if gain == 0. {
        "−∞ dB".into()
    } else {
        format!("{:+.1} dB", 20. * gain.log10())
    }
}

/// The unit-less form of [`db_text`] for tight inline slots that carry their
/// unit elsewhere (the mixer's send rows): −∞ or a signed one-decimal dB
/// value with no trailing unit.
pub fn db_text_bare(gain: f64) -> String {
    if gain == 0. {
        "−∞".into()
    } else {
        format!("{:+.1}", 20. * gain.log10())
    }
}

/// A level amplitude as a signed dBFS readout; exact digital silence is
/// −∞ dBFS, spelled with the same U+2212 minus as [`db_text`].
pub fn dbfs_text(amplitude: f32) -> String {
    if amplitude <= 0. {
        "−∞ dBFS".into()
    } else {
        format!("{:+.1} dBFS", 20. * f64::from(amplitude).log10())
    }
}

/// One multiplicative ±`delta_db` step of a linear gain, clamped to
/// `min..=max`. Exact digital silence has no dB domain: multiplying zero
/// keeps it exactly silent, so a surface that wants silence to become
/// audible seeds a nonzero value itself (the Gain inspector does).
pub fn step_db(gain: f64, delta_db: f64, min: f64, max: f64) -> f64 {
    if gain <= 0. {
        return gain;
    }
    (gain * 10f64.powf(delta_db / 20.)).clamp(min, max)
}

pub fn control_feedback(snapshot: &AppSnapshot, pending_request: bool) -> String {
    let control = &snapshot.control;
    if pending_request {
        return t!("Applying changes...").into();
    }
    if control.accepted_pending > 0 {
        return t!("Applying changes...").into();
    }
    if let Some(reply) = control.last_result {
        use moiren_core::protocol::ReplyCode;
        return match reply.code {
            ReplyCode::Applied | ReplyCode::AppliedLate => t!("Changes applied").into(),
            ReplyCode::Accepted => t!("Applying changes...").into(),
            ReplyCode::StaleRevision | ReplyCode::StaleEpoch => {
                t!("Audio routing changed. Try again.").into()
            }
            ReplyCode::UnknownParameter => t!("This control is no longer available.").into(),
            ReplyCode::InvalidValue => t!("Choose a valid parameter value.").into(),
            ReplyCode::QueueFull => t!("Audio controls are busy. Try again.").into(),
            ReplyCode::OutOfOrder | ReplyCode::InvalidTime => {
                t!("Unable to apply this change. Try again.").into()
            }
        };
    }
    if snapshot.phase == SessionPhase::Running {
        t!("No parameter changes requested").into()
    } else {
        t!("Next start · engine stopped").into()
    }
}

/// The one paint-time translation of a stored [`UiFailure`] — the same job
/// [`control_feedback`] does for reply codes. Storage keeps typed payloads
/// and stable English diagnostics; the current locale is applied here, so a
/// stored failure re-renders on a locale switch through the normal rebuild
/// paths. Free-form producer prose ([`UiFailure::ExportInvalid`]) is
/// diagnostic data and passes through untranslated by design.
pub(crate) fn failure_text(failure: &UiFailure) -> String {
    failure_text_in(failure, &rust_i18n::locale())
}

/// [`failure_text`] under an explicit locale — the locale-flip tests' seam
/// (the settings draft feedback composes it into its own sentences), so
/// parallel tests never touch the process-global locale.
pub(crate) fn failure_text_in(failure: &UiFailure, locale: &str) -> String {
    match failure {
        UiFailure::Dispatch(error) => match error {
            DispatchError::Busy => {
                t!(
                    "The audio service is busy; try again in a moment.",
                    locale = locale
                )
            }
            DispatchError::Exiting => {
                t!("The audio service is shutting down.", locale = locale)
            }
            DispatchError::Disconnected => {
                t!("The audio service is disconnected.", locale = locale)
            }
            DispatchError::InvalidConfig => {
                t!(
                    "The audio service rejected the request as invalid.",
                    locale = locale
                )
            }
            DispatchError::GenerationExhausted => t!(
                "Too many audio sessions have been started. Restart the application to continue.",
                locale = locale
            ),
            DispatchError::RequestIdExhausted => t!(
                "Too many pending requests. Restart the application to continue.",
                locale = locale
            ),
        }
        .into_owned(),
        UiFailure::InvalidAudioSettings(error) => {
            t!(error.to_string(), locale = locale).into_owned()
        }
        UiFailure::ExportInvalid { reason } => reason.clone(),
    }
}

pub fn output_name(snapshot: &AppSnapshot, id: &str) -> String {
    output_rows(&snapshot.catalog, snapshot.audio_settings.backend)
        .into_iter()
        .find(|row| row.endpoint_id == id)
        .map(|row| row.name.clone())
        .unwrap_or_else(|| t!("Unavailable output").into())
}

/// Presentation label for a node: exactly the node's own label. The old
/// default-label substitution (an export's name, the bound device name) is
/// gone — the node card surfaces its binding as a chip, so repeating the
/// binding as the name read as a duplicate. What the node is, it shows.
pub fn display_label(entry: &crate::service::GraphNodeState) -> String {
    entry.label.clone()
}

pub fn source_name(snapshot: &AppSnapshot, source: &InputSelection) -> String {
    match source {
        InputSelection::Tone { frequency_hz } => t!(
            "Test tone · %{frequency} Hz",
            frequency = format!("{frequency_hz:.0}")
        )
        .to_string(),
        InputSelection::Physical { endpoint_id } => {
            input_rows(&snapshot.catalog, snapshot.audio_settings.backend)
                .into_iter()
                .find(|row| row.endpoint_id == *endpoint_id)
                .map(|row| row.name.clone())
                .unwrap_or_else(|| t!("Unavailable input").into())
        }
        InputSelection::Process {
            pid,
            creation_time_100ns,
        } => snapshot
            .catalog
            .processes
            .iter()
            .find(|row| row.pid == *pid && row.creation_time_100ns == *creation_time_100ns)
            .map(|row| t!("%{name} (PID %{pid})", name = row.name, pid = pid).to_string())
            .unwrap_or_else(|| t!("Unavailable application (PID %{pid})", pid = pid).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{db_text, db_text_bare, dbfs_text, step_db};
    use crate::i18n::{ENGLISH, zh};
    use moiren_engine::node::DeviceMode;

    #[test]
    fn db_text_reads_signed_db_and_one_infinity_for_silence() {
        assert_eq!(db_text(0.), "−∞ dB");
        assert_eq!(db_text(1.), "+0.0 dB");
        assert_eq!(db_text(10f64.powf(-6. / 20.)), "-6.0 dB");
        assert_eq!(db_text(16.), "+24.1 dB");
    }

    #[test]
    fn dbfs_text_matches_db_text_but_in_the_meter_unit() {
        assert_eq!(dbfs_text(0.), "−∞ dBFS");
        assert_eq!(dbfs_text(1.), "+0.0 dBFS");
        assert_eq!(dbfs_text(10f32.powf(-6. / 20.)), "-6.0 dBFS");
        // Negative amplitudes are as absent as silence: ballistics never
        // produce them, and silence is the honest rendering if they appear.
        assert_eq!(dbfs_text(-0.5), "−∞ dBFS");
    }

    #[test]
    fn bare_readouts_keep_their_slot_formats() {
        // The send rows' unit-less label: signed one-decimal dB, −∞ spelled
        // with the same U+2212 minus as db_text.
        assert_eq!(db_text_bare(0.), "−∞");
        assert_eq!(db_text_bare(1.), "+0.0");
        assert_eq!(db_text_bare(10f64.powf(-6. / 20.)), "-6.0");
    }

    #[test]
    fn step_db_moves_one_db_and_clamps_silence_is_sticky() {
        let one_db = 10f64.powf(1. / 20.);
        assert!((step_db(1., 1., 0., 16.) - one_db).abs() < 1e-12);
        assert!((step_db(one_db, -1., 0., 16.) - 1.).abs() < 1e-12);
        // Silence has no dB domain and stays exactly silent.
        assert_eq!(step_db(0., 1., 0., 16.), 0.);
        assert_eq!(step_db(0., -1., 0., 16.), 0.);
        // The Gain node domain clamps at amplitude 16.
        assert_eq!(step_db(16., 1., 0., 16.), 16.);
        // Send gains have no upper limit by design.
        assert!((step_db(16., 1., 0., f64::INFINITY) - 16. * one_db).abs() < 1e-9);
    }

    /// The diagnostics "backend / mode" row paints `t!(mode.title())`; every
    /// variant must resolve in both shipped locales, replacing the retired
    /// `format!("WASAPI / {:?}", ..)` Debug blob.
    #[test]
    fn stream_mode_titles_render_in_both_locales() {
        for (mode, zh_cn) in [
            (DeviceMode::Shared, "共享"),
            (DeviceMode::Exclusive, "独占"),
        ] {
            let title = mode.title();
            assert!(!title.is_empty());
            assert_eq!(rust_i18n::t!(title, locale = ENGLISH), title);
            assert_eq!(zh(title), zh_cn, "zh-CN value for {title:?}");
        }
    }

    // The status bar's text shapers. Expected strings assert on the
    // interpolated facts (digits, units) so they hold under any locale
    // fallback; the zh-CN entries are covered by the ledger guards.

    #[test]
    fn percent_text_never_claims_an_exact_zero_it_cannot_know() {
        assert_eq!(super::percent_text(0.), "0.00%");
        assert_eq!(super::percent_text(0.001), "<0.01%");
        assert_eq!(super::percent_text(0.02), "0.02%");
        assert_eq!(super::percent_text(3.5), "3.50%");
    }

    #[test]
    fn time_texts_spell_seconds_and_minutes() {
        use std::time::Duration;
        assert_eq!(super::ago_text(Duration::from_secs(42)), "42 s ago");
        assert_eq!(super::ago_text(Duration::from_secs(75)), "1 min 15 s ago");
        assert_eq!(super::span_text(Duration::from_secs(7)), "7 s");
        assert_eq!(super::span_text(Duration::from_secs(686)), "11 min 26 s");
    }

    #[test]
    fn buffer_text_shows_frames_and_duration_or_a_dash() {
        assert_eq!(super::buffer_text(256, 48_000), "256 smp / 5.3 ms");
        assert_eq!(super::buffer_text(512, 48_000), "512 smp / 10.7 ms");
        assert_eq!(super::buffer_text(0, 48_000), "—", "no invented period");
        assert_eq!(super::buffer_text(256, 0), "—");
    }

    #[test]
    fn format_short_text_pairs_rate_with_known_depth() {
        assert_eq!(
            super::format_short_text(48_000, Some("24-bit PCM".into())),
            "48.0 kHz / 24-bit PCM"
        );
        assert_eq!(super::format_short_text(96_000, None), "96.0 kHz");
        assert_eq!(super::format_short_text(0, None), "—", "no invented rate");
    }
}
