use super::{checks_frame, fields::parameters, integer::verify_integer, verify_dial, verify_rows};
use crate::service::BuiltinState;
use gpui_kit::{Bounds, point, size};
use moiren_engine::processor::DelaySettings;

fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<f32> {
    Bounds {
        origin: point(x, y),
        size: size(w, h),
    }
}

#[::core::prelude::v1::test]
fn scalar_capture_filters_keep_four_frames_and_integer_checks_inspector_only() {
    for density in ["comfortable", "compact"] {
        for surface in ["card", "inspector"] {
            for slug in ["eq", "gate", "limiter", "delay"] {
                assert!(checks_frame(&format!("nodes-{density}-{surface}-{slug}")));
                assert_eq!(super::super::single_node_frame_count(Some(slug)), 4);
            }
            for slug in ["meter", "recorder"] {
                assert_eq!(
                    checks_frame(&format!("nodes-{density}-{surface}-{slug}")),
                    surface == "inspector"
                );
                assert_eq!(super::super::single_node_frame_count(Some(slug)), 4);
            }
        }
    }
    for name in [
        "nodes-comfortable-01-input-eq",
        "nodes-compact-inspector-meter-stopped",
        "nodes-compact-inspector-meter32",
        "nodes-compact-card-mapper-editing",
        "nodes-wide-card-eq",
    ] {
        assert!(!checks_frame(name), "{name}");
    }
}

#[::core::prelude::v1::test]
fn scalar_capture_descriptors_match_fields_and_signed_percentage_units() {
    for (kind, count) in [
        (moiren_core::graph::NodeKind::Eq, 3),
        (moiren_core::graph::NodeKind::Gate, 5),
        (moiren_core::graph::NodeKind::Limiter, 3),
        (moiren_core::graph::NodeKind::Delay, 3),
    ] {
        let state = BuiltinState::for_kind(kind);
        let (_, fields) = parameters(&state).unwrap();
        assert_eq!(fields.len(), count);
    }
    let delay = BuiltinState::Delay(DelaySettings {
        feedback: -0.25,
        mix: 0.375,
        ..Default::default()
    });
    let (slug, fields) = parameters(&delay).unwrap();
    assert_eq!(slug, "delay");
    assert_eq!(fields[1], ("DelayFeedback", "%", -25.));
    assert_eq!(fields[2], ("DelayMix", "%", 37.5));
}

#[::core::prelude::v1::test]
fn scalar_capture_gate_aligns_each_row_without_requiring_one_global_baseline() {
    let mut cells = (0..5)
        .map(|index| {
            let x = (index % 3) as f32 * 94.;
            let y = (index / 3) as f32 * 120.;
            (bounds(x, y, 88., 104.), bounds(x + 12., y + 82., 64., 22.))
        })
        .collect::<Vec<_>>();
    assert_eq!(verify_rows(&cells).unwrap(), 2);
    cells[4].1.origin.y += 5.;
    assert!(verify_rows(&cells).is_err());
    cells[4].1.origin.y -= 5.;
    cells[3].0.origin.y = 90.;
    assert!(verify_rows(&cells).is_err());
    cells[3].0.origin.y = 120.;
    cells[1].0.origin.x = 80.;
    assert!(verify_rows(&cells).is_err());
}

#[::core::prelude::v1::test]
fn scalar_capture_dials_reject_overlapping_units_and_shifted_numeric_baselines() {
    let caption = bounds(0., 0., 88., 28.);
    let knob = bounds(20., 30., 48., 48.);
    let input = bounds(12., 82., 64., 22.);
    let unit = bounds(76., 82., 12., 22.);
    assert!(verify_dial(caption, knob, input, unit).is_ok());
    assert!(verify_dial(caption, knob, input, bounds(76., 90., 12., 22.)).is_err());
    assert!(verify_dial(caption, knob, input, bounds(70., 82., 12., 22.)).is_err());
    assert!(verify_dial(caption, knob, bounds(0., 82., 64., 22.), unit).is_err());
}

#[::core::prelude::v1::test]
fn scalar_capture_integer_evidence_uses_exact_frames_and_real_channel_limit() {
    assert!(verify_integer("48000", 48_000, 1_048_576).is_ok());
    for value in ["48000.0", "4.8e4", "NaN", "0", "48001"] {
        assert!(verify_integer(value, 48_000, 1_048_576).is_err(), "{value}");
    }
    let maximum = moiren_engine::recorder::RecorderSettings::max_capacity_frames(256).unwrap();
    assert_eq!(maximum, 65_536);
    assert!(verify_integer("65536", 65_536, maximum).is_ok());
    assert!(verify_integer("65537", 65_537, maximum).is_err());
}
