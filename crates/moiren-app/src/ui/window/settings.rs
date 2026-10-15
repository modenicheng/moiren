use super::*;
use controls::*;

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
                    this.update(cx, |s, cx| {
                        s.density = value;
                        let mut preferences =
                            crate::project::load_preferences().unwrap_or_default();
                        preferences.density = if value == Density::Compact {
                            crate::project::UiDensity::Compact
                        } else {
                            crate::project::UiDensity::Comfortable
                        };
                        cx.spawn(async move |this, cx| {
                            let result = cx
                                .background_executor()
                                .spawn(
                                    async move { crate::project::save_preferences(&preferences) },
                                )
                                .await;
                            let _ = this.update(cx, |s, cx| {
                                s.project_message = Some(
                                    result
                                        .map(|_| "Preferences saved".to_string())
                                        .unwrap_or_else(|e| e.to_string()),
                                );
                                cx.notify();
                            });
                        })
                        .detach();
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
