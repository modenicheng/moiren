use super::*;

#[test]
fn empty_rack_accepts_supported_channel_counts() {
    let rack = RackSettings::default();
    assert!(rack.slots.is_empty());
    for channels in [1, 2, 8, 256] {
        assert!(rack.valid_for(channels), "{channels} channels");
    }
    for channels in [0, 257, usize::MAX] {
        assert!(!rack.valid_for(channels), "{channels} channels");
    }
}

#[test]
fn slot_limit_and_stable_ids_include_bypassed_slots() {
    let mut rack = RackSettings {
        slots: (0..RackSettings::MAX_SLOTS as u32)
            .map(|id| slot(id, RackProcessor::Gain { initial: 1.0 }))
            .collect(),
    };
    assert!(rack.valid_for(2));
    rack.slots[1].id = u32::MAX;
    rack.slots.reverse();
    assert!(rack.valid_for(2));
    rack.slots
        .push(slot(65, RackProcessor::Gain { initial: 1.0 }));
    assert!(!rack.valid_for(2));
    rack.slots.pop();
    rack.slots[0].id = rack.slots[1].id;
    rack.slots[0].bypass = true;
    assert!(!rack.valid_for(2));
}

#[test]
fn all_processor_defaults_validate_and_pan_requires_stereo() {
    for processor in processors() {
        assert!(rack(processor.clone()).valid_for(2), "{processor:?}");
        if matches!(processor, RackProcessor::Pan { .. }) {
            for channels in [1, 3, 256] {
                assert!(!rack(processor.clone()).valid_for(channels));
            }
        } else {
            for channels in [1, 8, 256] {
                assert!(rack(processor.clone()).valid_for(channels));
            }
        }
    }
}

#[test]
fn gain_and_pan_reject_nonfinite_or_out_of_range_values() {
    for initial in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.1, 16.1] {
        assert!(!rack(RackProcessor::Gain { initial }).valid_for(2));
    }
    for initial in [0.0, 16.0] {
        assert!(rack(RackProcessor::Gain { initial }).valid_for(2));
    }
    for initial in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.1, 1.1] {
        assert!(
            !rack(RackProcessor::Pan {
                initial,
                mono: false,
                swap: false,
            })
            .valid_for(2)
        );
    }
    for initial in [-1.0, 1.0] {
        assert!(
            rack(RackProcessor::Pan {
                initial,
                mono: true,
                swap: true,
            })
            .valid_for(2)
        );
    }
}

#[test]
fn every_dynamics_and_eq_field_uses_its_kernel_domain() {
    // Replacing each field catches accidentally omitted configuration values.
    macro_rules! check_fields {
        ($variant:ident, $settings:ident, $( $field:ident: ($min:expr, $max:expr) ),+ $(,)?) => {
            $(
                for invalid in [
                    f64::NAN,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    $min - 0.01,
                    $max + 0.01,
                ] {
                    let settings = $settings {
                        $field: invalid,
                        ..$settings::default()
                    };
                    assert!(
                        !rack(RackProcessor::$variant(settings)).valid_for(2),
                        "{}={invalid}", stringify!($field)
                    );
                }
                for boundary in [$min, $max] {
                    let settings = $settings {
                        $field: boundary,
                        ..$settings::default()
                    };
                    assert!(
                        rack(RackProcessor::$variant(settings)).valid_for(2),
                        "{}={boundary}", stringify!($field)
                    );
                }
            )+
        };
    }
    check_fields!(
        Compressor, CompressorSettings,
        input_gain_db: (-60.0, 60.0),
        threshold_db: (-120.0, 24.0),
        ratio: (1.0, 100.0),
        attack_ms: (0.0, 60_000.0),
        release_ms: (0.0, 60_000.0),
        hold_ms: (0.0, 60_000.0),
        knee_db: (0.0, 60.0),
        makeup_gain_db: (-60.0, 60.0),
        output_gain_db: (-60.0, 60.0),
        mix: (0.0, 1.0),
    );
    check_fields!(
        Eq, EqSettings,
        frequency_hz: (20.0, 20_000.0),
        gain_db: (-24.0, 24.0),
        q: (0.1, 20.0),
    );
    check_fields!(
        Gate, GateSettings,
        threshold_db: (-120.0, 0.0),
        attack_ms: (0.0, 5_000.0),
        release_ms: (0.0, 5_000.0),
        hold_ms: (0.0, 5_000.0),
        range_db: (-120.0, 0.0),
    );
    check_fields!(
        Limiter, LimiterSettings,
        threshold_db: (-60.0, 0.0),
        release_ms: (0.0, 5_000.0),
        input_gain_db: (-60.0, 60.0),
    );
    check_fields!(
        Delay, DelaySettings,
        delay_ms: (0.0, 2_000.0),
        feedback: (-0.999, 0.999),
        mix: (0.0, 1.0),
    );
}

#[test]
fn delay_length_is_finite_bounded_and_contains_the_initial_delay() {
    for max_delay_ms in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 60_001.0] {
        assert!(
            !rack(RackProcessor::Delay(DelaySettings {
                max_delay_ms,
                ..DelaySettings::default()
            }))
            .valid_for(2)
        );
    }
    for max_delay_ms in [0.0, 60_000.0] {
        assert!(
            rack(RackProcessor::Delay(DelaySettings {
                delay_ms: max_delay_ms,
                max_delay_ms,
                ..DelaySettings::default()
            }))
            .valid_for(2)
        );
    }
    assert!(
        !rack(RackProcessor::Delay(DelaySettings {
            delay_ms: 11.0,
            max_delay_ms: 10.0,
            ..DelaySettings::default()
        }))
        .valid_for(2)
    );
}

#[test]
fn channel_mapping_accepts_identity_reorder_and_copy_but_rejects_invalid_widths() {
    for mapping in [vec![], vec![0, 1], vec![1, 0], vec![0, 0]] {
        assert!(
            rack(RackProcessor::ChannelMapper(ChannelMapperSettings {
                mapping,
                ..ChannelMapperSettings::default()
            }))
            .valid_for(2)
        );
    }
    for mapping in [vec![0], vec![0, 1, 0], vec![0, 2], vec![usize::MAX, 0]] {
        assert!(
            !rack(RackProcessor::ChannelMapper(ChannelMapperSettings {
                mapping,
                ..ChannelMapperSettings::default()
            }))
            .valid_for(2)
        );
    }
}

#[test]
fn weighted_mapping_requires_finite_square_rows_and_excludes_index_mapping() {
    let valid = ChannelMapperSettings {
        matrix: vec![vec![0.5, 0.5], vec![1.0, -1.0]],
        ..ChannelMapperSettings::default()
    };
    assert!(rack(RackProcessor::ChannelMapper(valid.clone())).valid_for(2));
    for matrix in [
        vec![vec![1.0, 0.0]],
        vec![vec![1.0], vec![0.0]],
        vec![vec![1.0, 0.0], vec![]],
        vec![vec![f64::NAN, 0.0], vec![0.0, 1.0]],
        vec![vec![1.0, f64::INFINITY], vec![0.0, 1.0]],
        vec![vec![1.0, 0.0], vec![f64::NEG_INFINITY, 1.0]],
    ] {
        assert!(
            !rack(RackProcessor::ChannelMapper(ChannelMapperSettings {
                matrix,
                ..ChannelMapperSettings::default()
            }))
            .valid_for(2)
        );
    }
    assert!(
        !rack(RackProcessor::ChannelMapper(ChannelMapperSettings {
            mapping: vec![0, 1],
            ..valid
        }))
        .valid_for(2)
    );
}

#[test]
fn bypassed_settings_still_require_valid_configuration() {
    let mut rack = rack(RackProcessor::Gain { initial: f64::NAN });
    rack.slots[0].bypass = true;
    assert!(!rack.valid_for(2));
    rack.slots[0].processor = RackProcessor::Pan {
        initial: 0.0,
        mono: false,
        swap: false,
    };
    assert!(!rack.valid_for(1));
}

#[test]
fn presets_validate_version_name_and_rack() {
    let mut preset = RackPreset {
        version: RackPreset::VERSION,
        name: "Vocal rack".to_owned(),
        rack: RackSettings::default(),
    };
    assert!(preset.valid_for(2));
    assert!(!preset.valid_for(0));
    for version in [0, RackPreset::VERSION + 1, u32::MAX] {
        preset.version = version;
        assert!(!preset.valid_for(2));
    }
    preset.version = RackPreset::VERSION;
    for name in ["", " \t ", "Bad\0name", "Bad\nname"] {
        preset.name = name.to_owned();
        assert!(!preset.valid_for(2));
    }
    preset.name = "x".repeat(RackPreset::MAX_NAME_BYTES + 1);
    assert!(!preset.valid_for(2));
    preset.name = "x".repeat(RackPreset::MAX_NAME_BYTES);
    assert!(preset.valid_for(2));
    preset.rack = rack(RackProcessor::Gain { initial: -1.0 });
    assert!(!preset.valid_for(2));
}

#[test]
fn config_types_support_serde_clone_debug_and_partial_eq() {
    fn assert_traits<T>()
    where
        T: Serialize + for<'de> Deserialize<'de> + Clone + std::fmt::Debug + PartialEq,
    {
    }
    assert_traits::<RackSettings>();
    assert_traits::<RackSlot>();
    assert_traits::<RackProcessor>();
    assert_traits::<RackPreset>();
    let rack = RackSettings {
        slots: processors()
            .into_iter()
            .enumerate()
            .map(|(id, processor)| slot(id as u32, processor))
            .collect(),
    };
    assert_eq!(rack.clone(), rack);
}
