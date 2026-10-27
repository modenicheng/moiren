//! Builtin field metadata, bounds, and single-field updates.
use super::*;

pub(super) fn gate_fields(settings: GateSettings) -> [BuiltinFieldSpec<GateSettings>; 5] {
    [
        BuiltinFieldSpec {
            key: "threshold",
            label_text: "Threshold",
            value: settings.threshold_db,
            suffix: "dB",
            min: -120.,
            max: 0.,
            step: 1.,
            set: |next, value| next.threshold_db = value,
        },
        BuiltinFieldSpec {
            key: "attack",
            label_text: "Attack",
            value: settings.attack_ms,
            suffix: "ms",
            min: 0.,
            max: 5_000.,
            step: 5.,
            set: |next, value| next.attack_ms = value,
        },
        BuiltinFieldSpec {
            key: "release",
            label_text: "Release",
            value: settings.release_ms,
            suffix: "ms",
            min: 0.,
            max: 5_000.,
            step: 10.,
            set: |next, value| next.release_ms = value,
        },
        BuiltinFieldSpec {
            key: "hold",
            label_text: "Hold",
            value: settings.hold_ms,
            suffix: "ms",
            min: 0.,
            max: 5_000.,
            step: 10.,
            set: |next, value| next.hold_ms = value,
        },
        BuiltinFieldSpec {
            key: "range",
            label_text: "Range",
            value: settings.range_db,
            suffix: "dB",
            min: -120.,
            max: 0.,
            step: 1.,
            set: |next, value| next.range_db = value,
        },
    ]
}

pub(super) fn limiter_fields(settings: LimiterSettings) -> [BuiltinFieldSpec<LimiterSettings>; 3] {
    [
        BuiltinFieldSpec {
            key: "threshold",
            label_text: "Threshold",
            value: settings.threshold_db,
            suffix: "dB",
            min: -60.,
            max: 0.,
            step: 1.,
            set: |next, value| next.threshold_db = value,
        },
        BuiltinFieldSpec {
            key: "release",
            label_text: "Release",
            value: settings.release_ms,
            suffix: "ms",
            min: 0.,
            max: 5_000.,
            step: 10.,
            set: |next, value| next.release_ms = value,
        },
        BuiltinFieldSpec {
            key: "input-gain",
            label_text: "Input gain",
            value: settings.input_gain_db,
            suffix: "dB",
            min: -60.,
            max: 60.,
            step: 1.,
            set: |next, value| next.input_gain_db = value,
        },
    ]
}

pub(super) fn delay_fields(settings: DelaySettings) -> [BuiltinFieldSpec<DelaySettings>; 3] {
    [
        BuiltinFieldSpec {
            key: "time",
            label_text: "Time",
            value: settings.delay_ms,
            suffix: "ms",
            min: 0.,
            max: settings.max_delay_ms,
            step: 10.,
            set: |next, value| next.delay_ms = value,
        },
        BuiltinFieldSpec {
            key: "feedback",
            label_text: "Feedback",
            value: settings.feedback,
            suffix: "%",
            min: -0.999,
            max: 0.999,
            step: 0.05,
            set: |next, value| next.feedback = value,
        },
        BuiltinFieldSpec {
            key: "mix",
            label_text: "Wet mix",
            value: settings.mix,
            suffix: "%",
            min: 0.,
            max: 1.,
            step: 0.05,
            set: |next, value| next.mix = value,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{delay_fields, gate_fields, limiter_fields};
    use moiren_engine::processor::{DelaySettings, GateSettings, LimiterSettings};

    #[::core::prelude::v1::test]
    fn field_units_describe_engine_values_instead_of_unlabelled_numbers() {
        assert_eq!(
            gate_fields(GateSettings::default()).map(|field| field.suffix),
            ["dB", "ms", "ms", "ms", "dB"]
        );
        assert_eq!(
            limiter_fields(LimiterSettings::default()).map(|field| field.suffix),
            ["dB", "ms", "dB"]
        );
        assert_eq!(
            delay_fields(DelaySettings::default()).map(|field| field.suffix),
            ["ms", "%", "%"]
        );
    }

    #[::core::prelude::v1::test]
    fn each_gate_setter_changes_only_its_declared_field() {
        let original = GateSettings::default();
        for field in gate_fields(original) {
            let mut next = original;
            let value = (field.value + field.step).clamp(field.min, field.max);
            (field.set)(&mut next, value);
            for other in gate_fields(next) {
                let expected = if other.key == field.key {
                    value
                } else {
                    gate_fields(original)
                        .into_iter()
                        .find(|old| old.key == other.key)
                        .unwrap()
                        .value
                };
                assert_eq!(other.value, expected, "{} changed {}", field.key, other.key);
            }
        }
    }

    #[::core::prelude::v1::test]
    fn delay_feedback_stays_signed_and_time_respects_prepared_capacity() {
        let settings = DelaySettings {
            max_delay_ms: 753.,
            ..DelaySettings::default()
        };
        let [time, feedback, mix] = delay_fields(settings);
        assert_eq!(time.max, 753.);
        assert_eq!((feedback.min, feedback.max), (-0.999, 0.999));
        assert_eq!((mix.min, mix.max), (0., 1.));
        let mut next = settings;
        (feedback.set)(&mut next, -0.5);
        assert_eq!(next.feedback, -0.5);
        assert_eq!(next.delay_ms, settings.delay_ms);
        assert_eq!(next.max_delay_ms, settings.max_delay_ms);
    }
}
