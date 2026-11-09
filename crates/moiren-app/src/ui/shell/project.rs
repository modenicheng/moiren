//! Native file prompts; parsing and filesystem writes run off the UI thread.
use super::*;
use crate::project::{ProjectDocument, WorkspaceDocument};
use crate::ui::widgets::occluding_page::{claim_presses, occluding_page};
use controls::*;
use rust_i18n::t;

/// The session popover: an occluding shell-root overlay (same dismissal
/// chrome as the settings menu and the fault popover) with the project
/// identity, the save/open actions, and the recent-projects quick switch.
/// The popover claims presses inside itself by stopping propagation, so
/// the overlay's outside handler closes it without any bounds math.
pub(super) fn menu(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let this = cx.entity();
    let close = this.clone();
    let save_pending = shell.project_io.busy || shell.state.read(cx).pending_request;
    let current = shell.project_io.path.clone();
    let recents = shell.preferences.values.recent_projects.clone();
    let mut popover = claim_presses(
        div()
            .id("project-popover")
            .absolute()
            .top(px(theme::TOOLBAR_HEIGHT))
            .left(px(32.))
            .w(px(280.))
            .p(px(theme::SPACE_3))
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_2))
            .bg(theme::SURFACE)
            .border_1()
            .border_color(theme::BORDER)
            .rounded(px(theme::RADIUS_MD))
            .occlude()
            .gpui_test_observe(),
    )
    .child(heading(shell.project_io.name.clone()))
    .child(label(
        shell
            .project_io
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| t!("Not saved to disk").into()),
    ))
    .child(
        button("open-project", t!("Open project…"))
            .disabled(shell.project_io.busy)
            .on_click({
                let this = this.clone();
                move |_, _, cx| this.update(cx, |s, cx| s.open_project(cx))
            }),
    )
    .child(
        button("save-project", t!("Save project"))
            .disabled(save_pending)
            .on_click({
                let this = this.clone();
                move |_, _, cx| this.update(cx, |s, cx| s.save_project(false, cx))
            }),
    )
    .child(
        button("save-project-as", t!("Save project as…"))
            .disabled(save_pending)
            .on_click({
                let this = this.clone();
                move |_, _, cx| this.update(cx, |s, cx| s.save_project(true, cx))
            }),
    );
    if !recents.is_empty() {
        popover = popover.child(label(t!("Recent projects")));
        for (index, path) in recents.iter().enumerate() {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let is_current = current.as_ref() == Some(path);
            popover = popover.child(
                button(SharedString::from(format!("recent-project-{index}")), name)
                    .full_width()
                    .justify_start()
                    .variant(ButtonVariant::Ghost)
                    .disabled(save_pending || is_current)
                    .on_click({
                        let this = this.clone();
                        let path = path.clone();
                        move |_, _, cx| {
                            this.update(cx, |s, cx| s.open_recent_project(path.clone(), cx))
                        }
                    }),
            );
        }
    }
    let popover = popover
        .when_some(shell.project_io.message.clone(), |v, message| {
            v.child(label(message_text(&message)))
        })
        .child(label(t!("Opening a project keeps audio stopped.")));
    occluding_page(
        "project-menu-page",
        true,
        move |cx| {
            close.update(cx, |shell, cx| {
                shell.chrome.project_menu = false;
                cx.notify();
            })
        },
        popover,
    )
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests {
    use super::{Duration, PersistedFailure, ProjectDocument, ProjectMessage, message_text_in};
    use crate::i18n::{ENGLISH, SIMPLIFIED_CHINESE, zh};
    use crate::ui::UiState;
    use crate::ui::shell::test_harness::{
        finish_service, initial_projection, shell_with_state, spawn_service, test_shell, wait_until,
    };
    use gpui_kit::AppContext;

    #[test]
    fn project_message_text_flips_with_the_locale() {
        // Static sentences relabel; en is the literal fallback.
        assert_eq!(
            message_text_in(&ProjectMessage::Saved, ENGLISH),
            "Project saved"
        );
        assert_eq!(
            message_text_in(&ProjectMessage::Saved, SIMPLIFIED_CHINESE),
            zh("Project saved")
        );
        assert_eq!(
            message_text_in(&ProjectMessage::LoadRequested, SIMPLIFIED_CHINESE),
            zh("Project load requested")
        );
        // Interpolated payloads keep their data across the flip; only the
        // wrapper sentence relabels.
        let failed = ProjectMessage::NotLoaded {
            cause: "stale revision".into(),
        };
        assert_eq!(
            message_text_in(&failed, ENGLISH),
            "Project was not loaded: stale revision"
        );
        assert_eq!(
            message_text_in(&failed, SIMPLIFIED_CHINESE),
            zh("Project was not loaded: %{error}").replace("%{error}", "stale revision")
        );
        // A persisted failure: the closed variant drives the sentence.
        let too_large = ProjectMessage::Persisted(PersistedFailure::TooLarge {
            path: std::path::PathBuf::from("Z:/sets/live.moiren.json"),
        });
        assert_eq!(
            message_text_in(&too_large, ENGLISH),
            "Document Z:/sets/live.moiren.json exceeds the 8 MiB limit"
        );
        assert_eq!(
            message_text_in(&too_large, SIMPLIFIED_CHINESE),
            zh("Document %{path} exceeds the 8 MiB limit")
                .replace("%{path}", "Z:/sets/live.moiren.json")
        );
    }

    #[gpui_kit::test]
    fn project_presentation_waits_for_service_and_maps_sparse_document_ids(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let runtime = spawn_service();
        let handle = runtime.handle();
        let snapshot = initial_projection(&handle);
        let mut document = ProjectDocument::from_graph("Sparse template", &snapshot.graph).unwrap();
        let graph = document.graph.as_mut().unwrap();
        for node in &mut graph.nodes {
            node.id += 42;
        }
        for edge in &mut graph.edges {
            edge.source_node += 42;
            edge.destination_node += 42;
        }
        document.layout.nodes.push(crate::project::NodeLayout {
            node_id: 42,
            x: -80.,
            y: 120.,
        });
        let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
        let (view, cx) = shell_with_state(cx, state);
        view.update(cx, |shell, cx| {
            shell
                .stage_project(document, "Sparse.moiren.json".into(), cx)
                .unwrap();
            assert_eq!(shell.project_io.name, "Untitled session");
            assert!(shell.project_io.pending.is_some());
        });
        wait_until(Duration::from_secs(2), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.finish_project_load(cx);
                shell.project_io.pending.is_none()
            })
        });
        view.update(cx, |shell, cx| {
            assert_eq!(shell.project_io.name, "Sparse template");
            let source = shell.state.read(cx).snapshot.graph.graph.nodes()[0].id();
            assert_eq!(
                shell.graph_view.node_positions.get(&source),
                Some(&(-80., 120.))
            );
            assert!(!shell.project_io.busy);
            assert!(shell.state.read(cx).selection.output_id.is_none());
        });
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn project_load_completion_cancels_canvas_gestures(cx: &mut gpui_kit::TestAppContext) {
        let (view, cx, runtime) = test_shell(cx);
        let handle = runtime.handle();
        let snapshot = initial_projection(&handle);
        let document = ProjectDocument::from_graph("Loaded", &snapshot.graph).unwrap();
        view.update(cx, |shell, cx| {
            // A pan grab in flight when the load lands: the rubber-band
            // trigger is a drag running while the async load completes and
            // `apply_project_presentation` switches pages.
            shell.graph_gestures.pan_grab = Some((100., 100.));
            shell.sync_drag_demand();
            assert!(shell.drag_active());
            assert!(
                shell
                    .stage_project(document, "Loaded.moiren.json".into(), cx)
                    .is_ok()
            );
        });
        wait_until(Duration::from_secs(2), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.finish_project_load(cx);
                shell.project_io.pending.is_none()
            })
        });
        view.update(cx, |shell, cx| {
            assert_eq!(
                shell.project_io.name, "Loaded",
                "the load must have applied"
            );
            assert!(
                shell.graph_gestures.pan_grab.is_none() && !shell.drag_active(),
                "the project apply path must cancel the in-flight gesture"
            );
            assert!(
                !shell.frames_wanted(cx),
                "the DRAG demand must not outlive the load"
            );
        });
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn rejected_project_enqueue_preserves_current_name_and_layout(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (view, cx, runtime) = test_shell(cx);
        let handle = runtime.handle();
        let snapshot = initial_projection(&handle);
        let document = ProjectDocument::from_graph("Rejected", &snapshot.graph).unwrap();
        finish_service(runtime);
        view.update(cx, |shell, cx| {
            let original = shell.state.read(cx).snapshot.graph.graph.nodes()[0].id();
            shell.graph_view.node_positions.insert(original, (10., 20.));
            assert!(
                shell
                    .stage_project(document, "Rejected.moiren.json".into(), cx)
                    .is_err()
            );
            assert_eq!(shell.project_io.name, "Untitled session");
            assert_eq!(
                shell.graph_view.node_positions.get(&original),
                Some(&(10., 20.))
            );
            assert!(shell.project_io.pending.is_none());
        });
    }

    #[gpui_kit::test]
    fn project_processing_restores_without_starting_or_replacing_device_defaults(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use moiren_engine::audio_config::{ProcessingPrecision, ProcessingSettings, SampleFormat};

        let runtime = spawn_service();
        let handle = runtime.handle();
        let mut snapshot = (*initial_projection(&handle)).clone();
        snapshot.audio_settings.output.format = SampleFormat::Pcm24;
        snapshot.audio_settings.input.buffer_frames = Some(512);
        let hardware_defaults = snapshot.audio_settings.clone();
        let mut document = ProjectDocument::from_graph("96k template", &snapshot.graph).unwrap();
        let processing = ProcessingSettings {
            sample_rate: 96_000,
            precision: ProcessingPrecision::F64,
            block_frames: 1024,
        };
        document.processing = Some(processing);
        let state = cx.new(|_| UiState::new(handle.clone(), std::sync::Arc::new(snapshot)));
        let (view, cx) = shell_with_state(cx, state);
        view.update(cx, |shell, cx| {
            shell.apply_project_presentation(&document, cx).unwrap();
        });
        wait_until(Duration::from_secs(2), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.state.read(cx).snapshot.audio_settings.processing == processing
            })
        });
        view.update(cx, |shell, cx| {
            let state = shell.state.read(cx);
            assert_eq!(state.max_block_frames, 1024);
            assert_eq!(
                state.snapshot.audio_settings.output,
                hardware_defaults.output
            );
            assert_eq!(state.snapshot.audio_settings.input, hardware_defaults.input);
            assert!(state.snapshot.active_audio_settings.is_none());
            assert_eq!(state.snapshot.phase, crate::service::SessionPhase::Idle);
            assert_eq!(
                shell.project_document(cx).unwrap().processing,
                Some(processing)
            );
        });
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn graph_edits_settle_into_the_autosave_file(cx: &mut gpui_kit::TestAppContext) {
        let (view, cx, runtime) = test_shell(cx);
        let autosave_file = std::env::temp_dir().join(format!(
            "moiren-autosave-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        view.update(cx, |shell, _| {
            shell.autosave.path = Some(autosave_file.clone());
        });
        let renamed = view.update(cx, |shell, cx| {
            let id = shell.state.read(cx).snapshot.graph.graph.nodes()[0].id();
            shell.state.update(cx, |state, _| {
                state.dispatch(crate::service::AppRequest::EditGraph(
                    crate::service::GraphEdit::SetLabel {
                        node: id,
                        label: "Renamed by edit".into(),
                    },
                ));
            });
            id
        });
        wait_until(Duration::from_secs(2), || {
            view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell
                    .state
                    .read(cx)
                    .snapshot
                    .graph
                    .node(renamed)
                    .is_some_and(|node| node.label == "Renamed by edit")
            })
        });
        view.update(cx, |shell, cx| {
            shell.poll_autosave(cx);
            assert!(
                shell.autosave.timer.is_some(),
                "a settled graph change must arm the debounce"
            );
            // The test executor does not run the debounce timer, so run its
            // settle step directly; the poll loops drive the timer in
            // production.
            shell.autosave.timer = None;
            shell.autosave.deadline = None;
            shell.autosave_write_now(cx);
        });
        // The test executor runs spawned work only when driven.
        cx.run_until_parked();
        let document = crate::project::load_autosave_at(&autosave_file)
            .expect("the debounce settle step wrote the autosave");
        let graph = document.graph.expect("the autosave carries the graph");
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.label == "Renamed by edit")
        );
        let _ = std::fs::remove_file(&autosave_file);
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn quitting_flushes_a_dirty_document_into_the_autosave_file(cx: &mut gpui_kit::TestAppContext) {
        let (view, cx, runtime) = test_shell(cx);
        let autosave_file = std::env::temp_dir().join(format!(
            "moiren-autosave-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        view.update(cx, |shell, _| {
            shell.autosave.path = Some(autosave_file.clone());
        });
        let mut flush = None;
        view.update(cx, |shell, cx| {
            // A presentation-only change (camera pan) still dirties the
            // document, so the quit path must flush it.
            shell.graph_view.camera.pan_x = 128.0;
            flush = shell.autosave_flush(cx);
            assert!(flush.is_some(), "a dirty document must flush on quit");
        });
        cx.run_until_parked();
        let document = crate::project::load_autosave_at(&autosave_file)
            .expect("the quit flush wrote the autosave");
        assert_eq!(document.layout.camera_x, 128.0);
        drop(flush);
        let _ = std::fs::remove_file(&autosave_file);
        finish_service(runtime);
    }

    #[gpui_kit::test]
    fn project_menu_lists_recents_and_closes_on_outside_press_and_escape(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{
            Modifiers, MouseButton, VisualTestContext, point, px, size, test::TestWindowExt,
        };

        fn render(cx: &mut VisualTestContext) {
            cx.update(|window, cx| window.render_frame(cx));
        }

        let (view, cx, runtime) = test_shell(cx);
        cx.update(|window, _| window.resize(size(px(1200.), px(800.))));
        view.update(cx, |shell, cx| {
            // Seeded preferences, no disk round trip: the menu lists the
            // quick-switch targets it was given.
            shell.preferences.values.recent_projects = vec![
                std::path::PathBuf::from("Z:/sets/live.moiren.json"),
                std::path::PathBuf::from("Z:/sets/rehearsal.moiren.json"),
            ];
            shell.chrome.project_menu = true;
            cx.notify();
        });
        render(cx);
        cx.update(|window, _| {
            assert!(window.find("project-popover").visible());
            assert!(window.find("recent-project-0").visible());
            assert!(window.find("recent-project-1").visible());
        });
        // A press on the workspace far from the popover closes the menu.
        let outside = point(px(1000.), px(700.));
        cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
        view.update(cx, |shell, _| assert!(!shell.chrome.project_menu));
        render(cx);
        cx.update(|window, _| assert!(window.try_find("project-popover").is_none()));
        // Reopened, Escape closes it too.
        view.update(cx, |shell, cx| {
            shell.chrome.project_menu = true;
            cx.notify();
        });
        render(cx);
        cx.update(|window, _| assert!(window.find("project-popover").visible()));
        cx.simulate_keystrokes("escape");
        view.update(cx, |shell, _| assert!(!shell.chrome.project_menu));
        render(cx);
        cx.update(|window, _| assert!(window.try_find("project-popover").is_none()));
        finish_service(runtime);
    }
}

/// The project status line's typed payload (design 21 §4): [`ProjectIo`]
/// stores what happened, and the paint sites translate it through
/// [`message_text`]. No prose is frozen into state, so a locale switch
/// relabels the stored event on the next repaint.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProjectMessage {
    Saved,
    Opened,
    LoadRequested,
    /// The service declined the load; `cause` is the service's stable
    /// cause string (`snapshot.graph.last_error`).
    NotLoaded {
        cause: String,
    },
    /// A persistence-layer failure, stored as [`PersistedFailure`] (the
    /// cloneable mirror of the closed `PersistenceError`); the sentence is
    /// chosen at paint by matching the variant.
    Persisted(PersistedFailure),
    /// The native save dialog failed to open; `detail` is the dialog
    /// layer's Display prose (infrastructure diagnostics).
    SaveDialogFailed {
        detail: String,
    },
    /// The native open dialog failed to open; `detail` as above.
    LoadDialogFailed {
        detail: String,
    },
    /// The background autosave write failed; `cause` is the persistence prose.
    AutoSaveFailed {
        cause: String,
    },
    PreferencesSaved,
    PreferencesSaveQueued,
    /// The preferences write failed; `cause` is the persistence prose.
    PreferencesSaveFailed {
        cause: String,
    },
}

/// The UI-side, cloneable mirror of the closed
/// [`crate::project::PersistenceError`]: one variant each, with the
/// `io::Error`/`serde_json::Error` sources degraded to their Display prose
/// (diagnostic data; the sentence around it is translated at paint). The
/// mirror exists because the status message rides `Clone`/`PartialEq`
/// plumbing (settings key, tray snapshot) the raw errors cannot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PersistedFailure {
    Io {
        operation: &'static str,
        path: std::path::PathBuf,
        source: String,
    },
    Json {
        path: std::path::PathBuf,
        source: String,
    },
    Invalid {
        field: String,
        reason: String,
    },
    UnsupportedVersion {
        document: &'static str,
        version: u32,
    },
    TooLarge {
        path: std::path::PathBuf,
    },
    ConfigurationDirectoryUnavailable,
}

impl From<crate::project::PersistenceError> for PersistedFailure {
    fn from(error: crate::project::PersistenceError) -> Self {
        match error {
            crate::project::PersistenceError::Io {
                operation,
                path,
                source,
            } => Self::Io {
                operation,
                path,
                source: source.to_string(),
            },
            crate::project::PersistenceError::Json { path, source } => Self::Json {
                path,
                source: source.to_string(),
            },
            crate::project::PersistenceError::Invalid { field, reason } => {
                Self::Invalid { field, reason }
            }
            crate::project::PersistenceError::UnsupportedVersion { document, version } => {
                Self::UnsupportedVersion { document, version }
            }
            crate::project::PersistenceError::TooLarge { path } => Self::TooLarge { path },
            crate::project::PersistenceError::ConfigurationDirectoryUnavailable => {
                Self::ConfigurationDirectoryUnavailable
            }
        }
    }
}

/// Paint-time translation of the stored status message: one literal `t!`
/// sentence per case, payloads interpolated as data. Reads the process
/// locale, exactly like the other `t!` paint sites.
pub(crate) fn message_text(message: &ProjectMessage) -> String {
    message_text_in(message, &rust_i18n::locale())
}

/// [`message_text`] under an explicit locale — the seam the locale-flip
/// test uses so parallel tests never touch the process-global locale.
fn message_text_in(message: &ProjectMessage, locale: &str) -> String {
    match message {
        ProjectMessage::Saved => t!("Project saved", locale = locale).to_string(),
        ProjectMessage::Opened => t!("Project opened", locale = locale).to_string(),
        ProjectMessage::LoadRequested => t!("Project load requested", locale = locale).to_string(),
        ProjectMessage::NotLoaded { cause } => t!(
            "Project was not loaded: %{error}",
            locale = locale,
            error = cause
        )
        .to_string(),
        ProjectMessage::SaveDialogFailed { detail } => t!(
            "Could not open save dialog: %{error}",
            locale = locale,
            error = detail
        )
        .to_string(),
        ProjectMessage::LoadDialogFailed { detail } => t!(
            "Could not open file dialog: %{error}",
            locale = locale,
            error = detail
        )
        .to_string(),
        ProjectMessage::AutoSaveFailed { cause } => t!(
            "Could not auto-save: %{error}",
            locale = locale,
            error = cause
        )
        .to_string(),
        ProjectMessage::PreferencesSaved => t!("Preferences saved", locale = locale).to_string(),
        ProjectMessage::PreferencesSaveQueued => {
            t!("Preferences save queued", locale = locale).to_string()
        }
        ProjectMessage::PreferencesSaveFailed { cause } => t!(
            "Could not save preferences: %{error}",
            locale = locale,
            error = cause
        )
        .to_string(),
        ProjectMessage::Persisted(error) => persistence_error_text(error, locale),
    }
}

/// One literal sentence per closed [`PersistedFailure`] variant; each
/// variant's own fields ride along as interpolated diagnostic detail.
fn persistence_error_text(error: &PersistedFailure, locale: &str) -> String {
    match error {
        PersistedFailure::Io {
            operation,
            path,
            source,
        } => t!(
            "Could not %{operation} %{path}: %{source}",
            locale = locale,
            operation = operation,
            path = path.display().to_string(),
            source = source,
        )
        .to_string(),
        PersistedFailure::Json { path, source } => t!(
            "Invalid JSON in %{path}: %{source}",
            locale = locale,
            path = path.display().to_string(),
            source = source,
        )
        .to_string(),
        PersistedFailure::Invalid { field, reason } => t!(
            "Invalid %{field}: %{reason}",
            locale = locale,
            field = field,
            reason = reason,
        )
        .to_string(),
        PersistedFailure::UnsupportedVersion { document, version } => t!(
            "Unsupported %{document} document version %{version}",
            locale = locale,
            document = document,
            version = version,
        )
        .to_string(),
        PersistedFailure::TooLarge { path } => t!(
            "Document %{path} exceeds the 8 MiB limit",
            locale = locale,
            path = path.display().to_string(),
        )
        .to_string(),
        PersistedFailure::ConfigurationDirectoryUnavailable => t!(
            "User configuration directory is unavailable",
            locale = locale
        )
        .to_string(),
    }
}

/// Flattens a file-dialog outcome into the chosen path or the failure
/// prose. Both fallible layers — the dialog itself and the answer channel —
/// carry Display errors, so the payload never needs a Debug blob.
fn dialog_outcome<T>(
    outcome: Result<Result<Option<T>, anyhow::Error>, impl std::fmt::Display>,
) -> Result<Option<T>, String> {
    match outcome {
        Ok(Ok(path)) => Ok(path),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// Short cause prose for the closed [`crate::service::DispatchError`] set.
/// These ride inside `PersistedFailure::Invalid` as diagnostic detail
/// (data); the sentence around them is translated at paint. The Debug blob
/// they replace said even less than these one-liners.
fn dispatch_error_cause(error: crate::service::DispatchError) -> &'static str {
    use crate::service::DispatchError as E;
    match error {
        E::Busy => "the service is busy",
        E::Exiting => "the service is exiting",
        E::Disconnected => "the service is disconnected",
        E::InvalidConfig => "the request configuration is invalid",
        E::GenerationExhausted => "the graph revision space is exhausted",
        E::RequestIdExhausted => "the request id space is exhausted",
    }
}

/// The ProjectIo group of StudioShell state (design 20 §4): the open
/// project's file identity and status line, and the in-flight load staged
/// while the service confirms.
pub(crate) struct ProjectIo {
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) name: String,
    pub(crate) message: Option<ProjectMessage>,
    pub(crate) busy: bool,
    pub(crate) pending: Option<(
        crate::service::RequestId,
        crate::project::ProjectDocument,
        std::path::PathBuf,
    )>,
}

impl StudioShell {
    /// Presentation page as its persisted form, shared by the document
    /// builder and the autosave fingerprint.
    pub(super) fn workspace_document(page: Page) -> WorkspaceDocument {
        match page {
            Page::Graph => WorkspaceDocument::Graph,
            Page::Mixer => WorkspaceDocument::Mixer,
            Page::Matrix => WorkspaceDocument::Matrix,
            Page::Devices => WorkspaceDocument::Devices,
        }
    }

    pub(crate) fn project_document(
        &self,
        cx: &App,
    ) -> Result<ProjectDocument, crate::project::PersistenceError> {
        let state = self.state.read(cx);
        let mut document =
            ProjectDocument::from_graph(self.project_io.name.clone(), &state.snapshot.graph)?;
        document.session = state.candidate_config().as_ref().map(Into::into);
        document.processing = Some(state.snapshot.audio_settings.processing);
        document.layout.camera_x = self.graph_view.camera.pan_x;
        document.layout.camera_y = self.graph_view.camera.pan_y;
        document.layout.zoom = self.graph_view.camera.zoom;
        document.layout.mixer_order = self
            .mixer
            .presentation
            .order
            .iter()
            .filter_map(|id| {
                state
                    .snapshot
                    .graph
                    .graph
                    .nodes()
                    .iter()
                    .position(|node| node.id() == *id)
                    .map(|index| index as u64)
            })
            .collect();
        document.layout.workspace = Self::workspace_document(self.presentation.page());
        for (index, node) in state.snapshot.graph.graph.nodes().iter().enumerate() {
            if let Some(&(x, y)) = self.graph_view.node_positions.get(&node.id()) {
                document.layout.nodes.push(crate::project::NodeLayout {
                    node_id: index as u64,
                    x,
                    y,
                });
            }
        }
        Ok(document)
    }

    fn save_project(&mut self, save_as: bool, cx: &mut Context<Self>) {
        let document = match self.project_document(cx) {
            Ok(document) => document,
            Err(error) => {
                self.project_io.message = Some(ProjectMessage::Persisted(error.into()));
                cx.notify();
                return;
            }
        };
        let known = self.project_io.path.clone().filter(|_| !save_as);
        let prompt = known.is_none().then(|| {
            cx.prompt_for_new_path(
                &std::env::current_dir().unwrap_or_default(),
                Some("Session.moiren.json"),
            )
        });
        self.project_io.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let path = if let Some(path) = known {
                Some(path)
            } else {
                match dialog_outcome(prompt.expect("save prompt").await) {
                    Ok(path) => path,
                    Err(detail) => {
                        let _ = this.update(cx, |s, cx| {
                            s.project_io.busy = false;
                            s.project_io.message =
                                Some(ProjectMessage::SaveDialogFailed { detail });
                            cx.notify();
                        });
                        return;
                    }
                }
            };
            let Some(path) = path else {
                let _ = this.update(cx, |s, cx| {
                    s.project_io.busy = false;
                    cx.notify();
                });
                return;
            };
            let target = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::project::save_project(&target, &document) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.project_io.busy = false;
                match result {
                    Ok(()) => {
                        s.project_io.path = Some(path);
                        s.project_io.message = Some(ProjectMessage::Saved);
                        s.remember_project_file(cx);
                    }
                    Err(error) => {
                        s.project_io.message = Some(ProjectMessage::Persisted(error.into()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_project(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("Open Moiren project").into()),
        });
        self.project_io.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let path = match dialog_outcome(prompt.await) {
                Ok(Some(paths)) => paths.into_iter().next(),
                Ok(None) => None,
                Err(detail) => {
                    let _ = this.update(cx, |s, cx| {
                        s.project_io.busy = false;
                        s.project_io.message = Some(ProjectMessage::LoadDialogFailed { detail });
                        cx.notify();
                    });
                    return;
                }
            };
            let Some(path) = path else {
                let _ = this.update(cx, |s, cx| {
                    s.project_io.busy = false;
                    cx.notify();
                });
                return;
            };
            let _ = this.update(cx, |s, cx| s.spawn_project_load(path, cx));
        })
        .detach();
    }

    /// Opens a project straight from the session menu's quick switch: the
    /// same load path as the file-dialog prompt, minus the prompt.
    pub(super) fn open_recent_project(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        self.chrome.project_menu = false;
        self.spawn_project_load(path, cx);
    }

    /// The one background load-and-stage path, shared by the file-dialog
    /// prompt and the recent-projects quick switch. Parsing runs off the
    /// UI thread; staging hands the document to the service.
    fn spawn_project_load(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        self.project_io.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let target = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::project::load_project(&target) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.project_io.busy = false;
                match result {
                    Ok(document) => match s.stage_project(document, path, cx) {
                        Ok(()) => {}
                        Err(error) => {
                            s.project_io.message = Some(ProjectMessage::Persisted(error.into()))
                        }
                    },
                    Err(error) => {
                        s.project_io.message = Some(ProjectMessage::Persisted(error.into()))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stage_project(
        &mut self,
        document: ProjectDocument,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::project::PersistenceError> {
        document.validate()?;
        let request = self
            .state
            .update(cx, |state, _| {
                state.try_dispatch(AppRequest::LoadProject(document.clone()))
            })
            .map_err(|error| crate::project::PersistenceError::Invalid {
                field: "service request".into(),
                reason: dispatch_error_cause(error).into(),
            })?;
        self.project_io.busy = true;
        self.project_io.message = Some(ProjectMessage::LoadRequested);
        self.project_io.pending = Some((request, document, path));
        Ok(())
    }

    pub(super) fn finish_project_load(&mut self, cx: &mut Context<Self>) {
        let Some((request, _, _)) = &self.project_io.pending else {
            return;
        };
        let snapshot = self.state.read(cx).snapshot.clone();
        if snapshot.consumed_request_id < *request {
            return;
        }
        let (_, document, path) = self.project_io.pending.take().expect("pending project");
        self.project_io.busy = false;
        if let Some(error) = &snapshot.graph.last_error {
            self.project_io.message = Some(ProjectMessage::NotLoaded {
                cause: error.clone(),
            });
        } else {
            match self.apply_project_presentation(&document, cx) {
                Ok(()) => {
                    self.project_io.path = Some(path);
                    self.project_io.message = Some(ProjectMessage::Opened);
                    self.remember_project_file(cx);
                }
                Err(error) => {
                    self.project_io.message = Some(ProjectMessage::Persisted(error.into()))
                }
            }
        }
        cx.notify();
    }

    /// The project file the working state came from becomes the restore
    /// target and the top of the quick-switch list: preferences remember it
    /// (for `restore_last_project` and the session menu's recents), and the
    /// autosave fingerprint restarts because the state is already on disk.
    fn remember_project_file(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        if let Some(path) = self.project_io.path.clone() {
            if self.preferences.values.last_project_path.as_ref() != Some(&path) {
                self.preferences.values.last_project_path = Some(path.clone());
                changed = true;
            }
            changed |= self.preferences.values.remember_recent_project(path);
        }
        if changed {
            self.save_preferences_quiet(cx);
        }
        self.autosave.stamp = autosave::AutosaveStamp::capture(self, cx);
    }

    pub(crate) fn apply_project_presentation(
        &mut self,
        document: &ProjectDocument,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::project::PersistenceError> {
        if let Some(processing) = document.processing {
            processing
                .validate()
                .map_err(|error| crate::project::PersistenceError::Invalid {
                    field: "processing".into(),
                    reason: error.to_string(),
                })?;
            let mut audio = self.state.read(cx).snapshot.audio_settings.clone();
            audio.processing = processing;
            self.state
                .update(cx, |state, _| {
                    state
                        .try_dispatch(AppRequest::SetAudioSettings(audio))
                        .map(|_| state.max_block_frames = processing.block_frames as usize)
                })
                .map_err(|error| crate::project::PersistenceError::Invalid {
                    field: "service request".into(),
                    reason: dispatch_error_cause(error).into(),
                })?;
        }
        self.graph_view.node_positions.clear();
        self.graph_view.pending_node_spawn = None;
        self.inspector.node_name_editing = None;
        self.inspector.compressor = graph::CompressorControls::default();
        self.mixer.presentation.order.clear();
        if let Some(graph) = &document.graph {
            let restored = graph.to_graph()?;
            self.mixer.presentation.order = document
                .layout
                .mixer_order
                .iter()
                .filter_map(|id| restored.node_ids.get(id).copied())
                .collect();
            for position in &document.layout.nodes {
                if let Some(id) = restored.node_ids.get(&position.node_id) {
                    self.graph_view
                        .node_positions
                        .insert(*id, (position.x, position.y));
                }
            }
        }
        self.metering.observation_nodes = None;
        if let Some(spec) = document.session_spec()? {
            self.state
                .update(cx, |state, _| state.restore_candidate(&spec));
        } else {
            self.state
                .update(cx, |state, _| state.restore_graph_candidate());
        }
        self.project_io.name = document.name.clone();
        self.graph_view.camera = Camera {
            pan_x: document.layout.camera_x,
            pan_y: document.layout.camera_y,
            zoom: document.layout.zoom,
            adjusted: true,
        };
        self.set_page(
            match document.layout.workspace {
                WorkspaceDocument::Graph => Page::Graph,
                WorkspaceDocument::Mixer => Page::Mixer,
                WorkspaceDocument::Matrix => Page::Matrix,
                WorkspaceDocument::Devices => Page::Devices,
            },
            cx,
        );
        self.selection.node = None;
        self.selection.route = None;
        Ok(())
    }
}
