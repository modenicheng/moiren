//! Recovery autosave: the working document is rewritten to a user-owned
//! autosave file once edits settle, and restored on the next launch. Manual
//! "Save project" keeps writing the external file the user chose.

use std::collections::BTreeMap;
use std::future::poll_fn;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use rust_i18n::t;

use super::*;

/// How long the working document must stay unchanged before the autosave
/// file is rewritten; continuing edits keep extending the wait.
pub(super) const AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(1);

/// Everything the saved document captures, fingerprinted so the poll can
/// tell "the document would change" from meter churn that never reaches disk.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct AutosaveStamp {
    revision: u64,
    processing: moiren_engine::audio_config::ProcessingSettings,
    session: Option<crate::service::SessionSpec>,
    camera: (f32, f32, f32),
    workspace: crate::project::WorkspaceDocument,
    mixer_order: Vec<moiren_core::graph::NodeId>,
    node_positions: BTreeMap<moiren_core::graph::NodeId, (f32, f32)>,
    name: String,
}

impl AutosaveStamp {
    pub(super) fn capture(shell: &StudioShell, cx: &App) -> Self {
        let state = shell.state.read(cx);
        Self {
            revision: state.snapshot.graph.candidate_revision,
            processing: state.snapshot.audio_settings.processing,
            session: state.candidate_config(),
            camera: (shell.camera.pan_x, shell.camera.pan_y, shell.camera.zoom),
            workspace: StudioShell::workspace_document(shell.presentation.page),
            mixer_order: shell.mixer.order.clone(),
            node_positions: shell
                .node_positions
                .iter()
                .map(|(id, position)| (*id, *position))
                .collect(),
            name: shell.project_name.clone(),
        }
    }
}

/// Serialized hand-off between background writes and their UI-thread
/// completion watcher, so writes chain and shutdown can await the newest.
#[derive(Default)]
pub(crate) struct AutosaveWriteCompletion {
    result: Option<Result<(), String>>,
    waiter: Option<Waker>,
}

async fn await_completion(completion: Arc<Mutex<AutosaveWriteCompletion>>) -> Result<(), String> {
    poll_fn(move |cx| {
        let mut completion = completion.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(result) = completion.result.take() {
            Poll::Ready(result)
        } else {
            completion.waiter = Some(cx.waker().clone());
            Poll::Pending
        }
    })
    .await
}

impl StudioShell {
    /// True when the working document is still the one the autosave file
    /// (or an explicit save) last captured.
    pub(super) fn autosave_matches(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        self.autosave_stamp.revision == state.snapshot.graph.candidate_revision
            && self.autosave_stamp.processing == state.snapshot.audio_settings.processing
            && self.autosave_stamp.session == state.candidate_config()
            && self.autosave_stamp.camera
                == (self.camera.pan_x, self.camera.pan_y, self.camera.zoom)
            && self.autosave_stamp.workspace
                == StudioShell::workspace_document(self.presentation.page)
            && self.autosave_stamp.mixer_order == self.mixer.order
            && self.node_positions.len() == self.autosave_stamp.node_positions.len()
            && self
                .node_positions
                .iter()
                .all(|(id, position)| self.autosave_stamp.node_positions.get(id) == Some(position))
            && self.autosave_stamp.name == self.project_name
    }

    /// Called from the control-side poll: arms the debounce whenever the
    /// working document has moved past the last saved state.
    pub(super) fn poll_autosave(&mut self, cx: &mut Context<Self>) {
        if self.autosave_path.is_none() || self.autosave_matches(cx) {
            return;
        }
        self.autosave_deadline = Some(Instant::now() + AUTOSAVE_DEBOUNCE);
        if self.autosave_timer.is_some() {
            return;
        }
        self.autosave_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                let sleep = match this.update(cx, |shell, _| shell.autosave_deadline) {
                    Ok(Some(deadline)) => deadline.saturating_duration_since(Instant::now()),
                    // Whatever cleared the deadline owns the write now.
                    Ok(None) => return,
                    Err(_) => return,
                };
                if sleep.is_zero() {
                    break;
                }
                cx.background_executor().timer(sleep).await;
            }
            let _ = this.update(cx, |shell, cx| {
                shell.autosave_timer = None;
                shell.autosave_deadline = None;
                if !shell.autosave_matches(cx) {
                    shell.autosave_write_now(cx);
                }
            });
        }));
    }

    /// Builds the working document and writes it off the UI thread, chained
    /// after any earlier write so an older document can never land on top of
    /// a newer one. This is the debounce's settle step; the gpui tests also
    /// call it directly because the test executor does not run timers.
    pub(super) fn autosave_write_now(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.autosave_path.clone() else {
            return;
        };
        let document = match self.project_document(cx) {
            Ok(document) => document,
            Err(error) => {
                self.project_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let stamp = AutosaveStamp::capture(self, cx);
        let previous = self.autosave_write.clone();
        let completion = Arc::new(Mutex::new(AutosaveWriteCompletion::default()));
        let publish = completion.clone();
        cx.background_executor()
            .spawn(async move {
                if let Some(previous) = previous {
                    await_completion(previous).await.ok();
                }
                let result = crate::project::save_autosave_at(&path, &document)
                    .map_err(|error| error.to_string());
                if let Err(error) = &result {
                    eprintln!("Moiren autosave: {error}");
                }
                let waiter = {
                    let mut completion = publish.lock().unwrap_or_else(|p| p.into_inner());
                    completion.result = Some(result);
                    completion.waiter.take()
                };
                if let Some(waiter) = waiter {
                    waiter.wake();
                }
            })
            .detach();
        self.autosave_write = Some(completion.clone());
        cx.spawn(async move |this, cx| {
            let result = await_completion(completion).await;
            let _ = this.update(cx, |shell, cx| {
                match result {
                    Ok(()) => {
                        // Only record the save while the state is still the
                        // one that was written; the poll re-arms otherwise.
                        if AutosaveStamp::capture(shell, cx) == stamp {
                            shell.autosave_stamp = stamp;
                        }
                    }
                    Err(error) => {
                        shell.project_message =
                            Some(t!("Could not auto-save: %{error}", error = error).to_string());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Synchronous quit-time write of unsaved changes; the returned task is
    /// awaited before shutdown finishes, like the pending preference write.
    pub(super) fn autosave_flush(&mut self, cx: &App) -> Option<Task<()>> {
        let path = self.autosave_path.clone()?;
        if self.autosave_matches(cx) {
            return None;
        }
        let document = self.project_document(cx).ok()?;
        let previous = self.autosave_write.clone();
        // The armed debounce would only rewrite the same document.
        self.autosave_deadline = None;
        Some(cx.background_executor().spawn(async move {
            if let Some(previous) = previous {
                await_completion(previous).await.ok();
            }
            if let Err(error) = crate::project::save_autosave_at(&path, &document) {
                eprintln!("Moiren autosave: {error}");
            }
        }))
    }
}
