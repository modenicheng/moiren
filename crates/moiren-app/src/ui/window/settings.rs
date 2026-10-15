use super::*;
use controls::*;

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
        let executor = cx.background_executor().clone();
        self.preference_save = Some(executor.spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            if let Err(error) = crate::project::save_preferences_at(path, &preferences) {
                eprintln!("Moiren preferences: {error}");
            }
        }));
        self.project_message = Some("Preferences save queued".into());
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
                    })
                }),
        );
    }
    div().id("settings-page").flex().flex_col().flex_1().min_h_0().overflow_hidden()
        .child(div().flex().flex_none().h(px(52.)).px(px(24.)).items_center().justify_between().child(heading("Settings"))
            .child(button("close-settings","Back to workspace").on_click({let this=cx.entity();move|_,_,cx|this.update(cx,|s,cx|{s.settings_open=false;cx.notify();})})))
        .child(div().id("settings-scroll").flex_1().overflow_y_scroll().p(px(24.)).child(div().max_w(px(760.)).flex().flex_col().gap(px(if shell.density==Density::Compact{8.}else{16.}))
            .child(heading("Appearance")).child(detail("Theme","Dark workstation")).child(label("Density")).child(density)
            .when_some(shell.project_message.clone(),|v,message|v.child(label(message)))
            .child(div().mt(px(16.)).child(heading("Audio engine")))
            .child(detail("State",state.phase_text())).child(detail("Session",state.snapshot.generation.0.to_string()))
            .child(detail("Native backend","WASAPI shared mode")).child(detail("Channels","Stereo"))
            .child(label("Choose devices in Devices and edit routing in Graph or Matrix. Applying a topology restarts its audio session."))
            .child(div().mt(px(16.)).child(heading("Keyboard")))
            .child(detail("Next control","Tab")).child(detail("Previous control","Shift + Tab")).child(detail("Activate focused action","Enter / Space"))
            .child(div().mt(px(16.)).child(heading("Diagnostics")))
            .child(detail("Candidate revision",state.snapshot.graph.candidate_revision.to_string()))
            .child(detail("Active revision",state.snapshot.graph.active_revision.map(|r|r.to_string()).unwrap_or_else(||"—".into())))
            .child(label(state.control_feedback()))
            .child(detail("Catalog",format!("{} inputs / {} outputs / {} applications",state.snapshot.catalog.inputs.len(),state.snapshot.catalog.outputs.len(),state.snapshot.catalog.processes.len())))
            .child(detail("CPU / latency / XRUN","— / — / —"))
            .child(label("These measurements are unavailable until their publishers are connected."))
            .child(button("quit-application","Quit Moiren").variant(ButtonVariant::Danger).on_click(|_,_,cx|cx.quit()))))
        .gpui_test_observe().into_any_element()
}
