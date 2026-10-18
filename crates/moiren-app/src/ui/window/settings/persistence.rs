use super::*;

#[derive(Default)]
struct PreferenceWriteCompletion {
    result: Option<Result<(), String>>,
    waiter: Option<std::task::Waker>,
}

impl StudioShell {
    /// Serialize preference writes so rapid profile/density actions cannot let
    /// an older background write overwrite the user's latest choice.
    pub(in super::super) fn save_preferences(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preferences_path.clone() else {
            self.project_message = Some("User configuration directory is unavailable".into());
            return;
        };
        let previous = self.preference_save.take();
        let preferences = self.preferences.clone();
        self.preference_save_revision = self
            .preference_save_revision
            .checked_add(1)
            .expect("preference save revision exhausted");
        let revision = self.preference_save_revision;
        let completion =
            std::sync::Arc::new(std::sync::Mutex::new(PreferenceWriteCompletion::default()));
        let publish = completion.clone();
        let executor = cx.background_executor().clone();
        self.preference_save = Some(executor.spawn(async move {
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
        let quitting = self.preferences_quitting.clone();
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
                if shell.preference_save_revision == revision {
                    if let Err(error) = &result
                        && let Some((saved_revision, editor)) = shell.settings.save_revision
                        && saved_revision == revision
                    {
                        let draft = shell.settings.draft_mut(editor);
                        draft.error = Some(format!("Could not save preferences: {error}"));
                        draft.message = None;
                    }
                    shell.project_message = Some(
                        result
                            .map(|_| "Preferences saved".into())
                            .unwrap_or_else(|error| format!("Could not save preferences: {error}")),
                    );
                    cx.notify();
                }
            });
        })
        .detach();
        self.project_message = Some("Preferences save queued".into());
    }
}
