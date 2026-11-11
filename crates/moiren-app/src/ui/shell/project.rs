//! Native file prompts; parsing and filesystem writes run off the UI thread.
//!
//! The session popover lives in `menu`, and the typed status payload with its
//! paint-time translation in `messages`; this module keeps the [`ProjectIo`]
//! state group and the save/open/stage/apply paths.
use super::*;
use crate::project::{ProjectDocument, WorkspaceDocument};
use rust_i18n::t;

mod menu;
mod messages;
#[cfg(all(test, feature = "gpui-test"))]
mod tests;

pub(super) use menu::menu;
pub(crate) use messages::{ProjectMessage, message_text};

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
        // Design 23 §4.3: no `session` section anymore — the graph carries
        // every binding, and processing settings travel in their own field.
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
                    state.try_dispatch(AppRequest::SetAudioSettings(audio))
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
