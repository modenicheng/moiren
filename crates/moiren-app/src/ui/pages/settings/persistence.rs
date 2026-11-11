use super::*;

/// The unset-preference recording directory: `<home>/Music/Moiren`, falling
/// back to the temp directory when no home can be resolved.
pub(crate) fn default_recording_directory() -> std::path::PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    home.map(|home| std::path::PathBuf::from(home).join("Music").join("Moiren"))
        .unwrap_or_else(std::env::temp_dir)
}

#[derive(Default)]
struct PreferenceWriteCompletion {
    result: Option<Result<(), String>>,
    waiter: Option<std::task::Waker>,
}

impl StudioShell {
    /// Where recordings land when no directory preference is set: the user's
    /// Music folder under a Moiren subdirectory. Resolved at click time so a
    /// preference change applies to the very next start without any sync.
    /// The format applied to the next recording start, mirroring the
    /// directory preference above.
    pub(crate) fn recording_format(&self) -> crate::service::RecordingFormat {
        self.preferences.values.recording_format
    }

    pub(crate) fn recording_output_directory(&self) -> std::path::PathBuf {
        self.preferences
            .values
            .recording_directory
            .clone()
            .unwrap_or_else(default_recording_directory)
    }

    /// Pick a new recording output directory and persist it quietly. The
    /// dialog is the platform folder picker; cancellation changes nothing.
    pub(crate) fn change_recording_directory(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("Choose the recording output directory").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |shell, cx| {
                    shell.preferences.values.recording_directory = Some(path);
                    shell.save_preferences_quiet(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Serialize preference writes so rapid profile/density actions cannot let
    /// an older background write overwrite the user's latest choice.
    pub(crate) fn save_preferences(&mut self, cx: &mut Context<Self>) {
        self.save_preferences_inner(cx, true);
    }

    /// Persists preferences without displacing user-facing feedback: for
    /// bookkeeping writes such as remembering the last project file.
    pub(crate) fn save_preferences_quiet(&mut self, cx: &mut Context<Self>) {
        self.save_preferences_inner(cx, false);
    }

    fn save_preferences_inner(&mut self, cx: &mut Context<Self>, report: bool) {
        let Some(path) = self.preferences.path.clone() else {
            if report {
                self.project_io.message = Some(project::ProjectMessage::Persisted(
                    crate::project::PersistenceError::ConfigurationDirectoryUnavailable.into(),
                ));
            }
            return;
        };
        let previous = self.preferences.save.take();
        let preferences = self.preferences.values.clone();
        self.preferences.save_revision = self
            .preferences
            .save_revision
            .checked_add(1)
            .expect("preference save revision exhausted");
        let revision = self.preferences.save_revision;
        let completion =
            std::sync::Arc::new(std::sync::Mutex::new(PreferenceWriteCompletion::default()));
        let publish = completion.clone();
        let executor = cx.background_executor().clone();
        self.preferences.save = Some(executor.spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = crate::project::save_preferences_at(path, &preferences)
                .map_err(|error| error.to_string());
            if let Err(error) = &result {
                eprintln!("Moiren preferences: {error}");
            }
            let waiter = {
                let mut completion = publish.lock().unwrap_or_else(|p| p.into_inner());
                completion.result = Some(result);
                completion.waiter.take()
            };
            if let Some(waiter) = waiter {
                waiter.wake();
            }
        }));
        // Presentation watches completion independently. Shutdown only owns the
        // background chain and can finish it after foreground tasks stop running.
        let quitting = self.preferences.quitting.clone();
        cx.spawn(async move |this, cx| {
            let result = std::future::poll_fn(move |cx| {
                let mut completion = completion.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(result) = completion.result.take() {
                    std::task::Poll::Ready(result)
                } else {
                    completion.waiter = Some(cx.waker().clone());
                    std::task::Poll::Pending
                }
            })
            .await;
            if quitting.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let _ = this.update(cx, |shell, cx| {
                if shell.preferences.save_revision == revision {
                    if report
                        && let Err(error) = &result
                        && let Some((saved_revision, editor)) = shell.settings.editor.save_revision
                        && saved_revision == revision
                    {
                        let draft = shell.settings.editor.draft_mut(editor);
                        draft.feedback = Some(DraftFeedback::PreferencesSaveFailed {
                            cause: error.clone(),
                        });
                    }
                    if report {
                        shell.project_io.message = Some(match result {
                            Ok(()) => project::ProjectMessage::PreferencesSaved,
                            Err(cause) => project::ProjectMessage::PreferencesSaveFailed { cause },
                        });
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        if report {
            self.project_io.message = Some(project::ProjectMessage::PreferencesSaveQueued);
        }
    }
}
