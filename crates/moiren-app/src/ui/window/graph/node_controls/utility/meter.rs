//! Live per-channel display readings remain separate from candidate settings.
use super::*;
use crate::ui::monitoring::MeterStatus;
use crate::ui::presentation::dbfs_text;
use moiren_engine::meter::MeterSettings;

pub(super) fn content(
    shell: &StudioShell,
    node: &GraphNodeState,
    settings: MeterSettings,
    cx: &Context<StudioShell>,
) -> Option<AnyElement> {
    let snapshot = &shell.state.read(cx).snapshot;
    let channels = snapshot.graph.graph.get_node(node.id).ok()?.channels();
    let managed = snapshot.signal_topics.node_levels.contains_key(&node.id);
    let status = shell.node_meters.status(node.id);
    let live = meter_is_live(snapshot.phase, managed, &status);
    // These are the existing display ballistics, not an invented measurement.
    // A held marker is labelled as such; stale retained values stay unknown.
    let levels = live
        .then(|| shell.node_meters.channel_levels(node.id))
        .flatten();
    let unknown = t!("Unknown");
    let mut windows = div().flex().items_center().gap(px(4.));
    for frames in [128, 480, 2048, 8192] {
        windows = windows.child(choice(
            format!("card-meter-window-{:?}-{frames}", node.id),
            frames.to_string(),
            frames == settings.window_frames,
            Some(builtin_edit(
                node.id,
                BuiltinState::Meter(MeterSettings {
                    window_frames: frames,
                }),
            )),
            cx,
        ));
    }
    let mut readings = div()
        .id(format!("card-meter-readings-{:?}", node.id))
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .max_h(px(320.))
        .overflow_y_scroll();
    for channel in 0..channels {
        let level = levels.and_then(|levels| levels.get(channel));
        readings = readings.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .w(px(136.))
                .flex_none()
                .child(note(
                    t!("Ch %{channel} / held peak", channel = channel + 1).to_string(),
                ))
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(17.))
                        .text_color(if level.is_some_and(|level| level.marker >= 1.) {
                            theme::DANGER
                        } else {
                            theme::TEXT
                        })
                        .child(level_text(level.map(|level| level.marker), &unknown)),
                )
                .child(note(
                    t!(
                        "RMS %{value}",
                        value = level_text(level.map(|level| level.rms), &unknown)
                    )
                    .to_string(),
                )),
        );
    }
    let state = if snapshot.phase != SessionPhase::Running {
        "Start audio to observe this node"
    } else if !managed {
        "No level publisher for this node"
    } else {
        status.label()
    };
    let meter_width = crate::ui::widgets::meters::node_meter_width(channels);
    let readout = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(readings)
        .child(note(state.to_owned()).text_color(if live {
            theme::SUCCESS
        } else {
            theme::TEXT_MUTED
        }))
        .child(note("Sample peak / RMS (display ballistics)"));
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .child(detail_line(
                "Window",
                &t!("%{frames} frames", frames = settings.window_frames),
            ))
            .child(windows)
            .child(
                div()
                    .relative()
                    .w_full()
                    .pr(px(meter_width + 16.))
                    .child(readout)
                    .child(
                        div()
                            .absolute()
                            .right_0()
                            .top_0()
                            .bottom_0()
                            .w(px(meter_width))
                            .child(vertical_meter(shell, node, cx)),
                    ),
            )
            .into_any_element(),
    )
}

fn meter_is_live(phase: SessionPhase, managed: bool, status: &MeterStatus) -> bool {
    phase == SessionPhase::Running && managed && *status == MeterStatus::Live
}

fn level_text(value: Option<f32>, unknown: &str) -> String {
    value
        .filter(|value| value.is_finite() && *value >= 0.)
        .map_or_else(|| unknown.to_owned(), dbfs_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn meter_never_reports_retained_stale_or_unmanaged_values_as_live() {
        assert!(meter_is_live(
            SessionPhase::Running,
            true,
            &MeterStatus::Live
        ));
        for phase in [
            SessionPhase::Idle,
            SessionPhase::Starting,
            SessionPhase::Stopping,
            SessionPhase::Failed,
        ] {
            assert!(!meter_is_live(phase, true, &MeterStatus::Live));
        }
        assert!(!meter_is_live(
            SessionPhase::Running,
            false,
            &MeterStatus::Live
        ));
        for status in [
            MeterStatus::Waiting,
            MeterStatus::Stale,
            MeterStatus::NoPublisher,
            MeterStatus::Hidden,
        ] {
            assert!(!meter_is_live(SessionPhase::Running, true, &status));
        }
        assert_eq!(level_text(None, "Unknown"), "Unknown");
        assert_eq!(level_text(Some(f32::NAN), "Unknown"), "Unknown");
        assert_eq!(level_text(Some(-0.1), "Unknown"), "Unknown");
        assert_ne!(level_text(Some(0.), "Unknown"), "Unknown");
        assert_eq!(level_text(Some(1.), "Unknown"), "+0.0 dBFS");
        assert_eq!(level_text(None, "localized unknown"), "localized unknown");
    }
}
