use super::*;
use crate::service::{DeliveryProvenance, ExpertInputBuffer};

#[derive(Clone, Copy)]
enum Input {
    Process,
    Vad,
}

impl Input {
    fn get(self, draft: &Draft) -> Option<ExpertInputBuffer> {
        match self {
            Self::Process => draft.transport.process,
            Self::Vad => draft.transport.vad,
        }
    }

    fn edit(self, draft: &mut Draft) -> &mut Option<ExpertInputBuffer> {
        match self {
            Self::Process => &mut draft.transport.process,
            Self::Vad => &mut draft.transport.vad,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Vad => "vad",
        }
    }
}

#[derive(Clone, Copy)]
enum Frames {
    Capacity,
    Target,
    Jitter,
    Packet,
    Gap,
}

impl Frames {
    fn title(self) -> &'static str {
        match self {
            Self::Capacity => "Ring capacity",
            Self::Target => "Normal input target",
            Self::Jitter => "Jitter reserve",
            Self::Packet => "Maximum input packet",
            Self::Gap => "Maximum delivery gap",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Capacity => "capacity",
            Self::Target => "target",
            Self::Jitter => "jitter",
            Self::Packet => "packet",
            Self::Gap => "gap",
        }
    }

    fn value(self, input: &mut ExpertInputBuffer) -> &mut u32 {
        match self {
            Self::Capacity => &mut input.capacity_frames,
            Self::Target => &mut input.target_frames,
            Self::Jitter => &mut input.jitter_frames,
            Self::Packet => &mut input.max_packet_frames,
            Self::Gap => &mut input.max_gap_frames,
        }
    }
}

pub(super) fn editor(shell: &StudioShell, cx: &Context<StudioShell>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(heading(t!("Expert input buffers")))
        .child(
            div()
                .id("transport-latency-unqualified")
                .child(label(t!("Input targets are estimates. Physical end-to-end latency is unqualified.")))
                .gpui_test_observe(),
        )
        .child(label(t!("All input bounds use 48 kHz frames. Capacity is spare storage; target is normal residency. The native owner validates the full output demand before activation.")))
        .child(input_editor(shell, Input::Process, cx))
        .child(input_editor(shell, Input::Vad, cx))
}

fn input_editor(shell: &StudioShell, input: Input, cx: &Context<StudioShell>) -> Div {
    let selected = input.get(&shell.settings.global);
    let prefix = input.prefix();
    let mut view = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .pt(px(12.))
        .border_t_1()
        .border_color(theme::BORDER)
        .child(heading(match input {
            Input::Process => t!("Process input buffer"),
            Input::Vad => t!("VAD input buffer"),
        }))
        .child(choices(
            shell,
            Editor::Global,
            &format!("{prefix}-policy"),
            selected.is_some(),
            [
                (false, t!("Compatible").into()),
                (true, t!("Expert estimate").into()),
            ],
            move |draft, expert| {
                let value = input.edit(draft);
                *value = expert.then(|| value.unwrap_or_default());
            },
            cx,
        ));
    if let Some(mut selected) = selected {
        let mut fields = div()
            .id(SharedString::from(format!("{prefix}-expert-fields")))
            .flex()
            .flex_col()
            .gap(px(10.));
        for frames in [
            Frames::Capacity,
            Frames::Target,
            Frames::Jitter,
            Frames::Packet,
            Frames::Gap,
        ] {
            let current = *frames.value(&mut selected);
            let mut values = match frames {
                Frames::Capacity => vec![512, 1024, 2048, 4096, 8192, 16384, 32768],
                Frames::Target => vec![64, 96, 128, 256, 512, 1024, 2048, 4096],
                Frames::Jitter => vec![0, 32, 64, 96, 128, 256, 480],
                Frames::Packet | Frames::Gap => vec![32, 64, 128, 240, 256, 480, 960, 2048],
            };
            if !values.contains(&current) {
                values.push(current);
                values.sort_unstable();
            }
            fields = fields.child(field(
                &t!(frames.title()),
                choices(
                    shell,
                    Editor::Global,
                    &format!("{prefix}-{}", frames.key()),
                    current,
                    values.into_iter().map(|value| (value, value.to_string())),
                    move |draft, value| {
                        if let Some(settings) = input.edit(draft) {
                            *frames.value(settings) = value;
                        }
                    },
                    cx,
                ),
            ));
        }
        fields = fields.child(field(
            &t!("Delivery evidence"),
            choices(
                shell,
                Editor::Global,
                &format!("{prefix}-provenance"),
                selected.provenance,
                [
                    (
                        DeliveryProvenance::Declared,
                        t!("Declared assumption").into(),
                    ),
                    (
                        DeliveryProvenance::Empirical,
                        t!("Empirical estimate").into(),
                    ),
                ],
                move |draft, value| {
                    if let Some(settings) = input.edit(draft) {
                        settings.provenance = value;
                    }
                },
                cx,
            ),
        ));
        view = view.child(fields.gpui_test_observe());
    }
    view
}

pub(super) fn summary(input: Option<ExpertInputBuffer>) -> String {
    match input {
        None => t!("Compatible").into(),
        Some(input) => t!(
            "Capacity %{capacity}; target %{target}; jitter %{jitter}; packet %{packet}; gap %{gap} frames at 48 kHz (%{evidence})",
            capacity = input.capacity_frames,
            target = input.target_frames,
            jitter = input.jitter_frames,
            packet = input.max_packet_frames,
            gap = input.max_gap_frames,
            evidence = match input.provenance {
                DeliveryProvenance::Declared => t!("Declared assumption"),
                DeliveryProvenance::Empirical => t!("Empirical estimate"),
            }
        ).into(),
    }
}
