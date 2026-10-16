use super::*;
use controls::*;

#[derive(Default)]
struct PreferenceWriteCompletion {
    result: Option<Result<(), String>>,
    waiter: Option<std::task::Waker>,
}

impl StudioShell {
    /// Serialize preference writes so rapid profile/density actions cannot let
    /// an older background write overwrite the user's latest choice.
    pub(super) fn save_preferences(&mut self, cx: &mut Context<Self>) {
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

    fn select_latency_profile(
        &mut self,
        profile: crate::service::AudioLatencyProfile,
        cx: &mut Context<Self>,
    ) {
        let queued = self.state.update(cx, |state, _| {
            state
                .try_dispatch(AppRequest::SetLatencyProfile(profile))
                .is_ok()
        });
        if queued {
            self.preferences.latency_profile = profile;
            self.save_preferences(cx);
        }
        cx.notify();
    }
}

pub(super) fn page(shell: &StudioShell, cx: &Context<StudioShell>) -> AnyElement {
    let state = shell.state.read(cx);
    let mut density = div().flex().gap(px(8.));
    for (id, title, value) in [
        ("density-comfortable", "Comfortable", Density::Comfortable),
        ("density-compact", "Compact", Density::Compact),
    ] {
        let this = cx.entity();
        density = density.child(
            button(id, title)
                .variant(if shell.density == value {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| {
                        shell.density = value;
                        shell.preferences.density = if value == Density::Compact {
                            crate::project::UiDensity::Compact
                        } else {
                            crate::project::UiDensity::Comfortable
                        };
                        shell.save_preferences(cx);
                        cx.notify();
                    });
                }),
        );
    }
    let mut profiles = div().flex().gap(px(8.));
    for (id, profile) in [
        (
            "latency-compatible",
            crate::service::AudioLatencyProfile::Compatible,
        ),
        (
            "latency-balanced",
            crate::service::AudioLatencyProfile::Balanced,
        ),
        ("latency-low", crate::service::AudioLatencyProfile::Low),
    ] {
        let this = cx.entity();
        profiles = profiles.child(
            button(id, profile.title())
                .variant(if shell.preferences.latency_profile == profile {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |shell, cx| shell.select_latency_profile(profile, cx));
                }),
        );
    }
    let mut native_timing = div().id("native-audio-timing").flex().flex_col();
    for (key, value) in state.native_timing_rows() {
        native_timing = native_timing.child(detail(&key, value));
    }
    div().id("settings-page").flex().flex_col().flex_1().min_h_0().overflow_hidden()
        .child(div().flex().flex_none().h(px(52.)).px(px(24.)).items_center().justify_between().child(heading("Settings"))
            .child(button("close-settings","Back to workspace").on_click({let this=cx.entity();move|_,_,cx|this.update(cx,|s,cx|{s.settings_open=false;cx.notify();})})))
        .child(div().id("settings-scroll").flex_1().overflow_y_scroll().p(px(24.)).child(div().max_w(px(760.)).flex().flex_col().gap(px(if shell.density==Density::Compact{8.}else{16.}))
            .child(heading("Audio engine"))
            .child(label("Latency profile")).child(profiles)
            .child(detail("Requested profile", shell.preferences.latency_profile.title()))
            .child(label(shell.preferences.latency_profile.description()))
            .child(detail("Active profile", state.snapshot.active_latency_profile.map(|profile| profile.title()).unwrap_or("—")))
            .child(label(if shell.preferences.latency_profile != state.snapshot.latency_profile {
                "Requested profile is waiting for the service"
            } else { state.latency_apply_text() }))
            .child(label("Device periods and queue budgets are validated on Apply / Start. Unsupported budgets report an error."))
            .when_some(state.dispatch_error.clone().or_else(|| state.snapshot.error.clone()).or_else(|| state.snapshot.graph.last_error.clone()), |v, error| v.child(label(error)))
            .when_some(shell.project_message.clone(),|v,message|v.child(label(message)))
            .child(detail("State",state.phase_text()))
            .child(detail("Native backend","WASAPI shared mode")).child(detail("Channels","Stereo"))
            .child(label("Choose devices in Devices and edit routing in Graph or Matrix. Compatible graph edits keep audio running; device or profile changes prepare a new audio session."))
            .child(div().mt(px(16.)).child(heading("Appearance"))).child(detail("Theme","Dark workstation")).child(label("Density")).child(density)
            .child(div().mt(px(16.)).child(heading("Keyboard")))
            .child(detail("Next control","Tab")).child(detail("Previous control","Shift + Tab")).child(detail("Activate focused action","Enter / Space"))
            .child(div().mt(px(16.)).child(heading("Diagnostics")))
            .when(state.pending_request || state.snapshot.control.accepted_pending > 0 || state.snapshot.control.last_result.is_some(), |view| view.child(label(state.control_feedback())))
            .child(native_timing)
            .child(detail("Catalog",format!("{} inputs / {} outputs / {} applications",state.snapshot.catalog.inputs.len(),state.snapshot.catalog.outputs.len(),state.snapshot.catalog.processes.len())))
            .child(detail("CPU / physical latency / XRUN","— / — / —"))
            .child(label("Physical end-to-end latency requires a measurement. Queue fill and callback timing do not establish CPU load or XRUN counts."))
            .child(button("quit-application","Quit Moiren").variant(ButtonVariant::Danger).on_click(|_,_,cx|cx.quit()))))
        .gpui_test_observe().into_any_element()
}
