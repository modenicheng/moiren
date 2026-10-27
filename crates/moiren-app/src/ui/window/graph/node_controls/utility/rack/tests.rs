use super::*;
use moiren_engine::processor::ChannelMapperSettings;
use moiren_engine::rack::RackRoute;

fn gain_slot(id: u32) -> RackSlot {
    RackSlot {
        id,
        bypass: false,
        processor: RackProcessor::Gain { initial: 1. },
    }
}

fn routed_rack() -> RackSettings {
    RackSettings {
        slots: vec![gain_slot(4), gain_slot(8), gain_slot(12)],
        routes: vec![
            RackRoute {
                input_port: 0,
                output_port: 0,
                // Membership is not execution order.
                slots: vec![12, 4],
            },
            RackRoute {
                input_port: 1,
                output_port: 1,
                slots: vec![8],
            },
        ],
    }
}

#[::core::prelude::v1::test]
fn rack_reorders_execution_order_within_the_actual_route() {
    let settings = routed_rack();
    let next = edited_rack(&settings, RackChange::Move(12, -1), &[2, 2], &[2, 2]).unwrap();
    assert_eq!(
        next.slots.iter().map(|slot| slot.id).collect::<Vec<_>>(),
        [12, 8, 4]
    );
    assert_eq!(next.routes, settings.routes);
    assert_eq!(
        next.resolve_layout(&[2, 2], &[2, 2]).unwrap().routes[0].slots,
        [12, 4]
    );
    assert!(edited_rack(&settings, RackChange::Move(4, -1), &[2, 2], &[2, 2]).is_none());
    assert!(edited_rack(&settings, RackChange::Move(8, 1), &[2, 2], &[2, 2]).is_none());
    assert_eq!(settings.slots[0].id, 4);
}

#[::core::prelude::v1::test]
fn rack_removal_and_bypass_preserve_ids_processors_and_route_membership() {
    let settings = routed_rack();
    let toggled = edited_rack(&settings, RackChange::Bypass(8), &[2, 2], &[2, 2]).unwrap();
    assert!(toggled.slots[1].bypass);
    assert_eq!(toggled.slots[1].processor, settings.slots[1].processor);
    assert_eq!(toggled.routes, settings.routes);
    assert!(!settings.slots[1].bypass);
    let removed = edited_rack(&settings, RackChange::Remove(4), &[2, 2], &[2, 2]).unwrap();
    assert_eq!(
        removed.slots.iter().map(|slot| slot.id).collect::<Vec<_>>(),
        [8, 12]
    );
    assert_eq!(removed.routes[0].slots, [12]);
    assert_eq!(removed.routes[1], settings.routes[1]);
    assert!(edited_rack(&settings, RackChange::Remove(99), &[2, 2], &[2, 2]).is_none());
}

#[::core::prelude::v1::test]
fn rack_add_assigns_selected_route_and_never_duplicates_maximum_id() {
    let settings = RackSettings {
        slots: vec![gain_slot(u32::MAX)],
        ..Default::default()
    };
    let next = edited_rack(
        &settings,
        RackChange::Add(0, RackProcessor::Eq(Default::default())),
        &[2],
        &[2],
    )
    .unwrap();
    assert_ne!(next.slots[1].id, u32::MAX);
    assert!(next.valid_for(2));
    assert!(next.routes.is_empty());
    let routed = routed_rack();
    let next = edited_rack(
        &routed,
        RackChange::Add(1, RackProcessor::Gain { initial: 1. }),
        &[2, 2],
        &[2, 2],
    )
    .unwrap();
    assert_eq!(next.routes[0], routed.routes[0]);
    assert_eq!(next.routes[1].slots, [8, next.slots[3].id]);
    assert_eq!(routed.slots.len(), 3);
}

#[::core::prelude::v1::test]
fn rack_rejects_width_breaking_edits_and_incompatible_presets() {
    let settings = RackSettings {
        slots: vec![RackSlot {
            id: 1,
            bypass: false,
            processor: RackProcessor::ChannelMapper(ChannelMapperSettings {
                mapping: vec![0, 0],
                matrix: Vec::new(),
            }),
        }],
        ..Default::default()
    };
    assert!(settings.valid_layout(&[1], &[2]));
    assert!(edited_rack(&settings, RackChange::Bypass(1), &[1], &[2]).is_none());
    assert!(edited_rack(&settings, RackChange::Remove(1), &[1], &[2]).is_none());
    let mono = RackSettings::default();
    assert!(
        edited_rack(
            &mono,
            RackChange::Add(
                0,
                RackProcessor::Pan {
                    initial: 0.,
                    mono: false,
                    swap: false
                }
            ),
            &[1],
            &[1],
        )
        .is_none()
    );
}

#[::core::prelude::v1::test]
fn rack_capacity_and_unknown_route_disable_add_without_dropping_stages() {
    let full = RackSettings {
        slots: (0..RackSettings::MAX_SLOTS as u32).map(gain_slot).collect(),
        ..Default::default()
    };
    assert!(
        edited_rack(
            &full,
            RackChange::Add(0, RackProcessor::Gain { initial: 1. }),
            &[2],
            &[2]
        )
        .is_none()
    );
    assert!(
        edited_rack(
            &RackSettings::default(),
            RackChange::Add(1, RackProcessor::Gain { initial: 1. }),
            &[2],
            &[2]
        )
        .is_none()
    );
    assert_eq!(full.slots.len(), RackSettings::MAX_SLOTS);
}
