//! Native file prompts; parsing and filesystem writes run off the UI thread.
use super::*;
use crate::project::{ProjectDocument, WorkspaceDocument};
use controls::*;

pub(super) fn menu(shell: &StudioShell, cx: &Context<StudioShell>) -> impl IntoElement {
    let save_pending = shell.project_busy || shell.state.read(cx).pending_request;
    div()
        .id("project-popover")
        .absolute()
        .top(px(44.))
        .left(px(32.))
        .w(px(280.))
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .bg(theme::SURFACE)
        .border_1()
        .border_color(theme::BORDER)
        .rounded(px(theme::RADIUS_MD))
        .occlude()
        .child(heading(shell.project_name.clone()))
        .child(label(
            shell
                .project_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Not saved to disk".into()),
        ))
        .child(
            button("open-project", "Open project…")
                .disabled(shell.project_busy)
                .on_click({
                    let this = cx.entity();
                    move |_, _, cx| this.update(cx, |s, cx| s.open_project(cx))
                }),
        )
        .child(
            button("save-project", "Save project")
                .disabled(save_pending)
                .on_click({
                    let this = cx.entity();
                    move |_, _, cx| this.update(cx, |s, cx| s.save_project(false, cx))
                }),
        )
        .child(
            button("save-project-as", "Save project as…")
                .disabled(save_pending)
                .on_click({
                    let this = cx.entity();
                    move |_, _, cx| this.update(cx, |s, cx| s.save_project(true, cx))
                }),
        )
        .when_some(shell.project_message.clone(), |v, message| {
            v.child(label(message))
        })
        .child(label("Opening a project keeps audio stopped."))
}

#[cfg(all(test, feature = "gpui-test"))]
mod tests {
    use super::{Duration, ProjectDocument, ServiceRuntime, StudioShell, UiState};
    use gpui_kit::AppContext;

    #[gpui_kit::test]
    fn project_presentation_waits_for_service_and_maps_sparse_document_ids(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
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
        let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
        view.update(cx, |shell, cx| {
            shell
                .stage_project(document, "Sparse.moiren.json".into(), cx)
                .unwrap();
            assert_eq!(shell.project_name, "Untitled session");
            assert!(shell.pending_project.is_some());
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let done = view.update(cx, |shell, cx| {
                shell.state.update(cx, |state, _| state.poll());
                shell.finish_project_load(cx);
                shell.pending_project.is_none()
            });
            if done {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        view.update(cx, |shell, cx| {
            assert_eq!(shell.project_name, "Sparse template");
            let source = shell.state.read(cx).snapshot.graph.graph.nodes()[0].id();
            assert_eq!(
                shell.node_positions.get(&format!("{source:?}")),
                Some(&(-80., 120.))
            );
            assert!(!shell.project_busy);
            assert!(shell.state.read(cx).selection.output_id.is_none());
        });
        handle.request_exit();
        runtime.join().unwrap();
    }

    #[gpui_kit::test]
    fn rejected_project_enqueue_preserves_current_name_and_layout(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let runtime = ServiceRuntime::spawn().unwrap();
        let handle = runtime.handle();
        let snapshot = crate::ui::state::initial_snapshot(&handle).unwrap();
        let document = ProjectDocument::from_graph("Rejected", &snapshot.graph).unwrap();
        let state = cx.new(|_| UiState::new(handle.clone(), snapshot));
        let (view, cx) = cx.add_window_view(|window, cx| StudioShell::new(state, window, cx, None));
        handle.request_exit();
        runtime.join().unwrap();
        view.update(cx, |shell, cx| {
            shell.node_positions.insert("original".into(), (10., 20.));
            assert!(
                shell
                    .stage_project(document, "Rejected.moiren.json".into(), cx)
                    .is_err()
            );
            assert_eq!(shell.project_name, "Untitled session");
            assert_eq!(shell.node_positions.get("original"), Some(&(10., 20.)));
            assert!(shell.pending_project.is_none());
        });
    }
}

impl StudioShell {
    pub(super) fn project_document(
        &self,
        cx: &App,
    ) -> Result<ProjectDocument, crate::project::PersistenceError> {
        let state = self.state.read(cx);
        let mut document =
            ProjectDocument::from_graph(self.project_name.clone(), &state.snapshot.graph)?;
        document.session = state.candidate_config().as_ref().map(Into::into);
        document.layout.camera_x = self.camera.pan_x;
        document.layout.camera_y = self.camera.pan_y;
        document.layout.zoom = self.camera.zoom;
        document.layout.mixer_width = self.mixer.width;
        document.layout.mixer_dock_width = self.mixer.dock_width;
        document.layout.mixer_order = self
            .mixer
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
        document.layout.workspace = match self.presentation.page {
            Page::Graph => WorkspaceDocument::Graph,
            Page::Mixer => WorkspaceDocument::Mixer,
            Page::Matrix => WorkspaceDocument::Matrix,
            Page::Devices => WorkspaceDocument::Devices,
        };
        for (index, node) in state.snapshot.graph.graph.nodes().iter().enumerate() {
            if let Some(&(x, y)) = self.node_positions.get(&format!("{:?}", node.id())) {
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
                self.project_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let known = self.project_path.clone().filter(|_| !save_as);
        let prompt = known.is_none().then(|| {
            cx.prompt_for_new_path(
                &std::env::current_dir().unwrap_or_default(),
                Some("Session.moiren.json"),
            )
        });
        self.project_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let path = if let Some(path) = known {
                Some(path)
            } else {
                match prompt.expect("save prompt").await {
                    Ok(Ok(path)) => path,
                    other => {
                        let message = format!("Could not open save dialog: {other:?}");
                        let _ = this.update(cx, |s, cx| {
                            s.project_busy = false;
                            s.project_message = Some(message);
                            cx.notify();
                        });
                        return;
                    }
                }
            };
            let Some(path) = path else {
                let _ = this.update(cx, |s, cx| {
                    s.project_busy = false;
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
                s.project_busy = false;
                match result {
                    Ok(()) => {
                        s.project_path = Some(path);
                        s.project_message = Some("Project saved".into());
                    }
                    Err(error) => s.project_message = Some(error.to_string()),
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
            prompt: Some("Open Moiren project".into()),
        });
        self.project_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let path = match prompt.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                other => {
                    let _ = this.update(cx, |s, cx| {
                        s.project_busy = false;
                        s.project_message = Some(format!("Could not open file dialog: {other:?}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let Some(path) = path else {
                let _ = this.update(cx, |s, cx| {
                    s.project_busy = false;
                    cx.notify();
                });
                return;
            };
            let target = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::project::load_project(&target) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.project_busy = false;
                match result {
                    Ok(document) => match s.stage_project(document, path, cx) {
                        Ok(()) => {}
                        Err(error) => s.project_message = Some(error.to_string()),
                    },
                    Err(error) => s.project_message = Some(error.to_string()),
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
        if let Some(graph) = &document.graph {
            let (logical, nodes) = graph.restore()?;
            let request = self
                .state
                .update(cx, |state, _| {
                    state.try_dispatch(AppRequest::ReplaceGraph {
                        graph: logical,
                        nodes,
                    })
                })
                .map_err(|error| crate::project::PersistenceError::Invalid {
                    field: "service request".into(),
                    reason: format!("{error:?}"),
                })?;
            self.project_busy = true;
            self.project_message = Some("Project load requested".into());
            self.pending_project = Some((request, document, path));
        } else {
            self.apply_project_presentation(&document, cx)?;
            self.project_path = Some(path);
            self.project_message = Some("Session settings loaded for next start".into());
        }
        Ok(())
    }

    pub(super) fn finish_project_load(&mut self, cx: &mut Context<Self>) {
        let Some((request, _, _)) = &self.pending_project else {
            return;
        };
        let snapshot = self.state.read(cx).snapshot.clone();
        if snapshot.consumed_request_id < *request {
            return;
        }
        let (_, document, path) = self.pending_project.take().expect("pending project");
        self.project_busy = false;
        if let Some(error) = &snapshot.graph.last_error {
            self.project_message = Some(format!("Project was not loaded: {error}"));
        } else {
            match self.apply_project_presentation(&document, cx) {
                Ok(()) => {
                    self.project_path = Some(path);
                    self.project_message = Some("Project opened".into());
                }
                Err(error) => self.project_message = Some(error.to_string()),
            }
        }
        cx.notify();
    }

    pub(super) fn apply_project_presentation(
        &mut self,
        document: &ProjectDocument,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::project::PersistenceError> {
        self.node_positions.clear();
        self.mixer.order.clear();
        self.mixer.width = document.layout.mixer_width;
        self.mixer.dock_width = document.layout.mixer_dock_width;
        if let Some(graph) = &document.graph {
            let restored = graph.to_graph()?;
            self.mixer.order = document
                .layout
                .mixer_order
                .iter()
                .filter_map(|id| restored.node_ids.get(id).copied())
                .collect();
            for position in &document.layout.nodes {
                if let Some(id) = restored.node_ids.get(&position.node_id) {
                    self.node_positions
                        .insert(format!("{id:?}"), (position.x, position.y));
                }
            }
        }
        self.observation_nodes = None;
        if let Some(spec) = document.session_spec()? {
            self.state
                .update(cx, |state, _| state.restore_candidate(&spec));
        } else {
            self.state
                .update(cx, |state, _| state.restore_graph_candidate());
        }
        self.project_name = document.name.clone();
        self.camera = Camera {
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
        self.selected_node = None;
        self.selected_route = None;
        Ok(())
    }
}
