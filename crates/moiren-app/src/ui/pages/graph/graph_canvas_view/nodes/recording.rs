use gpui_kit::{
    App, BorderStyle, Bounds, FontWeight, Hsla, PaintQuad, TransformationMatrix, point, px, size,
};

use crate::ui::pages::graph::graph_canvas::{
    CanvasFrame, CardRecording, RecordAction, RecordingLayout,
};
use crate::ui::theme;

use super::shared::{Label, Paint, paint_label, rounded};

/// The recorder card's session strip: status dot and phase word, the live
/// clock slot, the output file name, and the transport bar the phase fills.
/// The ticking duration shares the status line's right half but is painted
/// by the hot layer, so the cached card never repaints per second.
pub(super) fn paint_recording_strip(
    frame: &CanvasFrame,
    recording: &CardRecording,
    detail: &str,
    layout: &RecordingLayout,
    scale: f32,
    paint: Paint<'_>,
    cx: &mut App,
) {
    use crate::service::RecordingPhase;
    let Paint { shapes, window } = paint;
    let active = recording.phase == Some(RecordingPhase::Recording);
    let dot = frame.window_rect(layout.dot);
    window.paint_quad(PaintQuad {
        bounds: dot,
        corner_radii: (f32::from(dot.size.width) / 2.).into(),
        background: if active {
            theme::DANGER
        } else if recording.phase.is_some_and(|phase| !phase.is_terminal()) {
            theme::WARNING
        } else {
            theme::TEXT_FAINT
        }
        .into(),
        border_widths: 0f32.into(),
        border_color: theme::controls::TRANSPARENT,
        border_style: BorderStyle::default(),
    });
    // The phase word is the strip's state readout: red while tape rolls,
    // amber while held, muted otherwise, faint before the first session.
    let word_color = match recording.phase {
        Some(RecordingPhase::Recording) => theme::DANGER,
        Some(RecordingPhase::Paused) => theme::WARNING,
        Some(_) => theme::TEXT_MUTED,
        None => theme::TEXT_FAINT,
    };
    paint_label(
        frame,
        &layout.status,
        Label {
            text: &crate::ui::presentation::recording_phase_text(recording.phase),
            family: theme::FONT_UI,
            color: word_color,
            right_aligned: false,
            weight: FontWeight::SEMIBOLD,
        },
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
    if let Some((text, color)) = strip_file_label(recording, detail) {
        paint_label(
            frame,
            &layout.file_name,
            Label {
                text,
                family: theme::FONT_UI,
                color,
                right_aligned: false,
                weight: FontWeight::NORMAL,
            },
            Paint {
                shapes: &mut *shapes,
                window: &mut *window,
            },
            cx,
        );
    }
    // Transport bar: one rounded container under the session rows, its three
    // slots inset by a hairline so the row reads as a single control. The
    // container and the slots each project exactly one rect from the shared
    // layout — face, stroke, and glyphs all hang off those rects, so no
    // layer re-derives an offset and every layer scales with the camera.
    // Every slot is a real button in every phase (see [`transport_look`]);
    // which of them answer a press follows the same availability the hit
    // test consults, so paint and pointer never drift.
    let bar = frame.window_rect(layout.bar);
    let radius = theme::RADIUS_SM * scale;
    window.paint_quad(PaintQuad {
        bounds: bar,
        corner_radii: radius.into(),
        background: theme::SURFACE_4.into(),
        border_widths: theme::HAIRLINE_W.into(),
        border_color: theme::GRIP,
        border_style: BorderStyle::default(),
    });
    let slots = [
        (&layout.record, RecordAction::Record),
        (&layout.pause, RecordAction::Pause),
        (&layout.stop, RecordAction::Stop),
    ];
    for (bounds, action) in slots {
        let bounds = frame.window_rect(*bounds);
        let look = transport_look(action, recording.phase);
        rounded(bounds, look.background, radius, window);
        let glyph_px = px(10. * scale);
        let inset = (bounds.size.width - glyph_px) / 2.;
        let _ = window.paint_svg(
            Bounds {
                origin: bounds.origin + point(inset, inset),
                size: size(glyph_px, glyph_px),
            },
            format!("icons/{}.svg", look.glyph).into(),
            None,
            TransformationMatrix::unit(),
            look.glyph_color,
            cx,
        );
    }
}

/// One transport slot's resting look: the face it fills and the glyph it
/// wears. The single decision behind the strip's paint, so the layout tests
/// can pin the phase semantics without a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::ui) struct TransportLook {
    pub(in crate::ui) background: Hsla,
    pub(in crate::ui) glyph: &'static str,
    pub(in crate::ui) glyph_color: Hsla,
}

/// The transport look for one slot in one phase. No slot ever turns
/// hollow: the record key keeps its red wash and red glyph from idle
/// through the terminal phases (its press is gated by the hit test while a
/// session is live), and a pause or stop slot the phase does not offer
/// dims to a raised face with a muted glyph instead of an outline.
pub(in crate::ui) fn transport_look(
    action: RecordAction,
    phase: Option<crate::service::RecordingPhase>,
) -> TransportLook {
    let affordance = action.available(phase);
    // A disabled slot keeps its own glyph: the pause slot rests as ⏸, and
    // only a paused session flips it to the ▶ resume.
    let glyph = match affordance.unwrap_or(action) {
        RecordAction::Record => "player-record",
        RecordAction::Pause => "player-pause",
        RecordAction::Resume => "player-play",
        RecordAction::Stop => "player-stop",
    };
    let record = action == RecordAction::Record;
    TransportLook {
        // The record affordance is the one loud element on the strip: a red
        // wash under the red glyph marks the card's armed state. The other
        // slots sit a step raised, at full strength while enabled and
        // muted once the phase takes them away.
        background: if record {
            theme::DANGER_WASH
        } else {
            theme::controls::RAISED
        },
        glyph,
        glyph_color: if record {
            theme::DANGER
        } else if affordance.is_some() {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        },
    }
}

/// The strip's file row: the session's output name while one is set, or —
/// only on an idle card that has never named a file — the node's static
/// detail ("48 kHz L/R"), metadata the strip's presence used to swallow.
/// Terminal phases without a name paint nothing, as before. Returns the text
/// with its color.
pub(in crate::ui) fn strip_file_label<'a>(
    recording: &'a CardRecording,
    detail: &'a str,
) -> Option<(&'a str, Hsla)> {
    if !recording.file_name.is_empty() {
        Some((&recording.file_name, theme::TEXT_MUTED))
    } else if recording.phase.is_none() {
        Some((detail, theme::TEXT_FAINT))
    } else {
        None
    }
}

/// The live duration over a recorder card, painted by the hot layer at the
/// strip's reserved clock slot. Text only — geometry comes from the shared
/// frame, translated for dragged cards by the caller.
pub(in crate::ui) fn paint_recording_duration(
    frame: &CanvasFrame,
    layout: &RecordingLayout,
    secs: u64,
    paint: Paint<'_>,
    cx: &mut App,
) {
    let Paint { shapes, window } = paint;
    paint_label(
        frame,
        &layout.duration,
        Label {
            text: &crate::ui::presentation::recording_duration_text(secs),
            family: theme::FONT_MONO,
            color: theme::TEXT,
            right_aligned: true,
            weight: FontWeight::SEMIBOLD,
        },
        Paint {
            shapes: &mut *shapes,
            window: &mut *window,
        },
        cx,
    );
}
