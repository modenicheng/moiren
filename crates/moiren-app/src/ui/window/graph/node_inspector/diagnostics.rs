//! Observation is distinct from configuration: only fresh node data is live.
use super::*;
use crate::ui::monitoring::MeterStatus;

pub(super) fn node_observation(
    shell: &StudioShell,
    node: &moiren_core::graph::Node,
    cx: &Context<StudioShell>,
) -> AnyElement {
    let state = shell.state.read(cx);
    let snapshot = &state.snapshot;
    let managed = snapshot.signal_topics.node_levels.contains_key(&node.id());
    let meter_status = shell.node_meters.status(node.id());
    let status = observation_status(snapshot.phase, managed, &meter_status);
    // Subscription grace can retain display values after a publisher stops.
    // Retention is useful for animation, not evidence of a current reading.
    let levels = has_live_reading(snapshot.phase, managed, &meter_status)
        .then(|| shell.node_meters.channel_levels(node.id()))
        .flatten();
    let mut view = div()
        .id("node-observation")
        .flex()
        .flex_col()
        .flex_none()
        .gap(px(theme::SPACE_1))
        .pt(px(theme::SPACE_2))
        .border_t_1()
        .border_color(theme::HAIRLINE)
        .child(
            div()
                .id("node-observation-status")
                .flex()
                .items_center()
                .justify_between()
                .gap(px(theme::SPACE_2))
                .child(label(t!("Runtime observation")).text_size(px(theme::TEXT_XS)))
                .child(
                    label(t!(status))
                        .min_w_0()
                        .text_right()
                        .text_size(px(theme::TEXT_XS))
                        .text_color(if levels.is_some() {
                            theme::SUCCESS
                        } else {
                            theme::TEXT_MUTED
                        }),
                )
                .gpui_test_observe(),
        )
        .child(compact_horizontal_meter(
            levels,
            node.channels(),
            if levels.is_some() {
                shell.node_meters.channel_clips(node.id())
            } else {
                &[]
            },
        ));
    if let Some(levels) = levels {
        let mut readings = div().id("node-observation-levels").flex().flex_col();
        for (channel, level) in levels.iter().enumerate() {
            readings = readings.child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(theme::SPACE_2))
                    .py(px(theme::SPACE_1))
                    .child(label(t!("Channel %{index}", index = channel + 1)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .child(
                                label(format!("{} {}", t!("Peak"), dbfs_text(level.marker)))
                                    .text_color(theme::TEXT),
                            )
                            .child(label(format!("{} {}", t!("RMS"), dbfs_text(level.rms)))),
                    ),
            );
        }
        view = view.child(Disclosure::new(
            Section::Readings,
            node.id(),
            readings.gpui_test_observe(),
        ));
    } else {
        view = view.child(
            div()
                .id("node-observation-unknown")
                .child(
                    label(format!("{}: {}", t!("Peak / RMS"), t!("Unknown")))
                        .text_size(px(theme::TEXT_XS)),
                )
                .gpui_test_observe(),
        );
    }
    for (index, fault) in snapshot
        .graph
        .faults
        .iter()
        .filter(|fault| fault.subject == crate::service::GraphSubject::Node(node.id()))
        .enumerate()
    {
        let mut diagnostic = div()
            .id(format!("inspector-node-fault-{index}"))
            .flex()
            .flex_col()
            .gap(px(theme::SPACE_1))
            .child(label(fault.cause.clone()).text_color(theme::WARNING));
        if let Some(hint) = &fault.hint {
            diagnostic = diagnostic.child(label(hint.clone()));
        }
        view = view.child(diagnostic.gpui_test_observe());
    }
    view.into_any_element()
}

fn has_live_reading(
    phase: crate::service::SessionPhase,
    managed: bool,
    status: &MeterStatus,
) -> bool {
    phase == crate::service::SessionPhase::Running && managed && *status == MeterStatus::Live
}

fn observation_status(
    phase: crate::service::SessionPhase,
    managed: bool,
    status: &MeterStatus,
) -> &'static str {
    use crate::service::SessionPhase;
    match phase {
        SessionPhase::Idle | SessionPhase::Exited => "Not running",
        SessionPhase::Starting => "Starting",
        SessionPhase::Stopping | SessionPhase::Exiting => "Stopping",
        SessionPhase::Failed => "Failed",
        SessionPhase::Running if !managed => "Unavailable for this node",
        SessionPhase::Running => match status {
            MeterStatus::Live => "Live",
            MeterStatus::Waiting => "Waiting for a complete window",
            MeterStatus::Stale => "No recent audio window",
            MeterStatus::NoPublisher => "No publisher",
            MeterStatus::Stopped => "Not running",
            MeterStatus::Hidden | MeterStatus::NotAvailable | MeterStatus::Error(_) => {
                "Unavailable for this node"
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{has_live_reading, observation_status};
    use crate::service::SessionPhase;
    use crate::ui::monitoring::MeterStatus;

    #[::core::prelude::v1::test]
    fn only_a_running_managed_live_subscription_may_show_readings() {
        assert!(has_live_reading(
            SessionPhase::Running,
            true,
            &MeterStatus::Live
        ));
        assert!(!has_live_reading(
            SessionPhase::Running,
            false,
            &MeterStatus::Live
        ));
        for phase in [
            SessionPhase::Idle,
            SessionPhase::Starting,
            SessionPhase::Stopping,
            SessionPhase::Failed,
            SessionPhase::Exiting,
            SessionPhase::Exited,
        ] {
            assert!(!has_live_reading(phase, true, &MeterStatus::Live));
        }
        for status in [
            MeterStatus::Hidden,
            MeterStatus::Stopped,
            MeterStatus::NotAvailable,
            MeterStatus::Waiting,
            MeterStatus::Stale,
            MeterStatus::NoPublisher,
            MeterStatus::Error("private transport context".into()),
        ] {
            assert!(!has_live_reading(SessionPhase::Running, true, &status));
        }
    }

    #[::core::prelude::v1::test]
    fn observation_copy_distinguishes_stopped_missing_and_stale_without_transport_details() {
        assert_eq!(
            observation_status(SessionPhase::Idle, false, &MeterStatus::Waiting),
            "Not running"
        );
        assert_eq!(
            observation_status(SessionPhase::Running, false, &MeterStatus::Live),
            "Unavailable for this node"
        );
        assert_eq!(
            observation_status(SessionPhase::Running, true, &MeterStatus::Stale),
            "No recent audio window"
        );
        assert_eq!(
            observation_status(
                SessionPhase::Running,
                true,
                &MeterStatus::Error("SessionId(7)".into())
            ),
            "Unavailable for this node"
        );
    }
}
