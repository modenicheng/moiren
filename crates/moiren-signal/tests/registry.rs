use moiren_signal::{
    BlockInit, Bus, BusConfig, LatestBlock, LatestValue, LookupError, RegisterError, StreamBlock,
    StreamValue,
};
use std::num::NonZeroU32;

mod allocations {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };

    #[derive(Clone, Copy, Debug)]
    pub struct Event {
        pub size: usize,
        pub align: usize,
        pub allocated: bool,
    }

    #[derive(Clone, Copy)]
    pub struct Probe {
        active: bool,
        pub bytes: isize,
        pub events: [Event; 128],
        pub count: usize,
    }

    impl Probe {
        const EMPTY: Self = Self {
            active: false,
            bytes: 0,
            count: 0,
            events: [Event {
                size: 0,
                align: 0,
                allocated: false,
            }; 128],
        };
    }

    thread_local! { static PROBE: Cell<Probe> = const { Cell::new(Probe::EMPTY) }; }

    pub struct Allocator;
    fn record(layout: Layout, allocated: bool) {
        let _ = PROBE.try_with(|state| {
            let mut probe = state.get();
            if probe.active {
                probe.bytes += if allocated {
                    layout.size() as isize
                } else {
                    -(layout.size() as isize)
                };
                if probe.count < probe.events.len() {
                    probe.events[probe.count] = Event {
                        size: layout.size(),
                        align: layout.align(),
                        allocated,
                    };
                }
                probe.count += 1;
                state.set(probe);
            }
        });
    }

    // SAFETY: Every allocation is delegated unchanged to System; bookkeeping
    // uses only preinitialized thread-local Cells and never allocates itself.
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: The caller supplies the allocator's valid Layout.
            let pointer = unsafe { System.alloc(layout) };
            if !pointer.is_null() {
                record(layout, true);
            }
            pointer
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            record(layout, false);
            // SAFETY: Pointer/layout are passed through to their allocator.
            unsafe { System.dealloc(pointer, layout) };
        }
    }

    pub fn measure<T>(operation: impl FnOnce() -> T) -> (T, Probe) {
        struct Stop;
        impl Drop for Stop {
            fn drop(&mut self) {
                PROBE.with(|state| {
                    let mut probe = state.get();
                    probe.active = false;
                    state.set(probe);
                });
            }
        }
        PROBE.with(|state| {
            state.set(Probe {
                active: true,
                ..Probe::EMPTY
            })
        });
        let stop = Stop;
        let value = operation();
        drop(stop);
        (value, PROBE.with(Cell::get))
    }
}

#[global_allocator]
static ALLOCATOR: allocations::Allocator = allocations::Allocator;

#[test]
fn valid_nonzero_payload_can_register_without_default() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus
        .register_latest_block(
            "custom/status",
            BlockInit {
                header: NonZeroU32::new(7).unwrap(),
                element: NonZeroU32::new(3).unwrap(),
                capacity: 4,
            },
        )
        .unwrap();
    let found = bus
        .resolve::<LatestBlock<NonZeroU32, NonZeroU32>>("custom/status")
        .unwrap();
    assert_eq!(found.id(), topic.id());
    assert!(matches!(
        bus.resolve::<LatestBlock<u32, u32>>("custom/status"),
        Err(LookupError::TypeMismatch)
    ));
    assert!(bus.budget_used() > 0);
}

#[test]
fn four_shapes_resolve_and_modes_are_distinct_from_types() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let value = NonZeroU32::new(1).unwrap();
    bus.register_latest_value("latest", value).unwrap();
    bus.register_stream_value("stream", value, 8).unwrap();
    bus.register_latest_block(
        "block",
        BlockInit {
            header: value,
            element: value,
            capacity: 2,
        },
    )
    .unwrap();
    bus.register_stream_block(
        "stream-block",
        BlockInit {
            header: value,
            element: value,
            capacity: 2,
        },
        8,
    )
    .unwrap();
    assert!(bus.resolve::<LatestValue<NonZeroU32>>("latest").is_ok());
    assert!(bus.resolve::<StreamValue<NonZeroU32>>("stream").is_ok());
    assert!(
        bus.resolve::<StreamBlock<NonZeroU32, NonZeroU32>>("stream-block")
            .is_ok()
    );
    assert!(matches!(
        bus.resolve::<StreamValue<NonZeroU32>>("latest"),
        Err(LookupError::ModeMismatch)
    ));
    assert!(matches!(
        bus.resolve::<StreamValue<u32>>("latest"),
        Err(LookupError::TypeMismatch)
    ));
    assert!(matches!(
        bus.resolve::<LatestValue<NonZeroU32>>("block"),
        Err(LookupError::TypeMismatch)
    ));
    assert!(matches!(
        bus.resolve::<LatestValue<u32>>("absent"),
        Err(LookupError::NotFound)
    ));
}

#[test]
fn rejects_names_without_consuming_resources() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let initial = bus.budget_used();
    for name in ["", "nul\0name", &"é".repeat(129)] {
        assert!(matches!(
            bus.register_latest_value(name, 1_u32),
            Err(RegisterError::InvalidName)
        ));
        assert_eq!(bus.budget_used(), initial);
    }
    bus.register_latest_value(&"é".repeat(128), 1_u32).unwrap();
    let used = bus.budget_used();
    assert!(matches!(
        bus.register_latest_value(&"é".repeat(128), 2_u32),
        Err(RegisterError::DuplicateName)
    ));
    assert_eq!(bus.budget_used(), used);
}

#[test]
fn invalid_capacity_and_overflow_do_not_reserve_name() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let initial = bus.budget_used();
    assert!(matches!(
        bus.register_latest_block(
            "retry",
            BlockInit {
                header: 1_u32,
                element: 1_u64,
                capacity: 0
            }
        ),
        Err(RegisterError::InvalidCapacity)
    ));
    assert!(matches!(
        bus.register_stream_value("retry", 1_u32, 0),
        Err(RegisterError::InvalidCapacity)
    ));
    assert!(matches!(
        bus.register_latest_block(
            "retry",
            BlockInit {
                header: 1_u32,
                element: 1_u64,
                capacity: usize::MAX
            }
        ),
        Err(RegisterError::SizeOverflow)
    ));
    assert!(matches!(
        bus.register_stream_value("retry", 1_u32, usize::MAX),
        Err(RegisterError::SizeOverflow)
    ));
    assert_eq!(bus.budget_used(), initial);
    bus.register_latest_value("retry", 1_u32).unwrap();
}

#[test]
fn budget_failure_is_atomic_and_boundary_is_exact() {
    let mut measure = Bus::new(BusConfig::default()).unwrap();
    let base = measure.budget_used();
    measure.register_latest_value("one", 1_u64).unwrap();
    let required = measure.budget_used();
    assert!(required > base);
    let mut exact = Bus::new(BusConfig {
        byte_budget: required,
        ..BusConfig::default()
    })
    .unwrap();
    exact.register_latest_value("one", 1_u64).unwrap();
    assert_eq!(exact.budget_used(), required);
    let before = exact.budget_used();
    assert!(matches!(
        exact.register_latest_value("two", 1_u64),
        Err(RegisterError::BudgetExceeded)
    ));
    assert_eq!(exact.budget_used(), before);
    assert!(matches!(
        exact.resolve::<LatestValue<u64>>("two"),
        Err(LookupError::NotFound)
    ));
    let mut short = Bus::new(BusConfig {
        byte_budget: required - 1,
        ..BusConfig::default()
    })
    .unwrap();
    assert!(matches!(
        short.register_latest_value("one", 1_u64),
        Err(RegisterError::BudgetExceeded)
    ));
    assert_eq!(short.budget_used(), base);
    short.register_latest_value("one", ()).unwrap();
}

#[test]
fn payload_limit_includes_metadata_alignment_and_precedes_factories() {
    let mut bus = Bus::new(BusConfig {
        max_payload_bytes: 8,
        ..BusConfig::default()
    })
    .unwrap();
    let initial = bus.budget_used();
    assert!(matches!(
        bus.register_latest_value("large", 1_u64),
        Err(RegisterError::PayloadTooLarge)
    ));
    assert!(matches!(
        bus.register_latest_block_with(
            "large",
            8,
            || panic!("header factory must not run"),
            || 1_u64
        ),
        Err(RegisterError::PayloadTooLarge)
    ));
    assert_eq!(bus.budget_used(), initial);
}

#[test]
fn factories_build_template_once_and_panic_does_not_commit() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let mut headers = 0;
    let mut elements = 0;
    bus.register_latest_block_with(
        "factory",
        5,
        || {
            headers += 1;
            NonZeroU32::new(7).unwrap()
        },
        || {
            elements += 1;
            NonZeroU32::new(elements).unwrap()
        },
    )
    .unwrap();
    assert_eq!((headers, elements), (1, 5));
    let used = bus.budget_used();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = bus.register_stream_block_with(
                "retry",
                3,
                || 1_u32,
                || -> u32 { panic!("factory failure") },
                8,
            );
        }))
        .is_err()
    );
    assert_eq!(bus.budget_used(), used);
    assert!(matches!(
        bus.resolve::<StreamBlock<u32, u32>>("retry"),
        Err(LookupError::NotFound)
    ));
    bus.register_stream_block_with("retry", 3, || 1_u32, || 2_u32, 8)
        .unwrap();
}

#[test]
fn topic_limits_and_bus_identities_are_checked() {
    let config = BusConfig {
        max_topics: 1,
        ..BusConfig::default()
    };
    let mut first = Bus::new(config).unwrap();
    let mut second = Bus::new(config).unwrap();
    let one = first.register_latest_value("one", ()).unwrap();
    let two = second.register_latest_value("one", ()).unwrap();
    assert_ne!(one.id(), two.id());
    let used = first.budget_used();
    assert!(matches!(
        first.register_latest_value("two", ()),
        Err(RegisterError::TopicLimit)
    ));
    assert_eq!(first.budget_used(), used);
}

#[test]
fn validates_config_and_default_contract() {
    let config = BusConfig::default();
    assert_eq!(
        (
            config.byte_budget,
            config.max_topics,
            config.max_subscribers_per_topic,
            config.default_stream_depth,
            config.max_payload_bytes
        ),
        (32 * 1024 * 1024, 128, 64, 8, 1024 * 1024)
    );
    for subscribers in [0, 65, usize::MAX] {
        assert!(matches!(
            Bus::new(BusConfig {
                max_subscribers_per_topic: subscribers,
                ..config
            }),
            Err(RegisterError::InvalidCapacity)
        ));
    }
    assert!(matches!(
        Bus::new(BusConfig {
            default_stream_depth: 0,
            ..config
        }),
        Err(RegisterError::InvalidCapacity)
    ));
    assert!(matches!(
        Bus::new(BusConfig {
            max_topics: usize::MAX,
            ..config
        }),
        Err(RegisterError::SizeOverflow)
    ));
    assert!(matches!(
        Bus::new(BusConfig {
            byte_budget: 0,
            ..config
        }),
        Err(RegisterError::BudgetExceeded)
    ));
}

#[test]
fn zst_still_owns_metadata_and_stream_depth_costs_resources() {
    let mut latest = Bus::new(BusConfig::default()).unwrap();
    let base = latest.budget_used();
    latest.register_latest_value("topic", ()).unwrap();
    assert!(latest.budget_used() > base);
    let mut stream = Bus::new(BusConfig::default()).unwrap();
    stream.register_stream_value("topic", (), 1).unwrap();
    let mut deeper = Bus::new(BusConfig::default()).unwrap();
    deeper.register_stream_value("topic", (), 8).unwrap();
    // Latest now reserves its business sender directory, so cross-mode totals
    // differ in metadata too. Compare Stream depths to isolate queue costs.
    assert!(stream.budget_used() > base);
    assert!(deeper.budget_used() > stream.budget_used());
}

#[test]
fn payload_copy_layout_pads_only_after_all_fields() {
    let mut bus = Bus::new(BusConfig {
        max_payload_bytes: 32,
        ..BusConfig::default()
    })
    .unwrap();
    // PublicationMeta (24) + header (4) + element (4), already 8-byte aligned.
    bus.register_latest_block(
        "exact-copy",
        BlockInit {
            header: 1_u32,
            element: 2_u32,
            capacity: 1,
        },
    )
    .unwrap();
}

#[repr(align(512))]
#[derive(Clone, Copy)]
struct Aligned(u8);

#[repr(C)]
struct ValueSlotMirror<T> {
    meta: moiren_signal::PublicationMeta,
    value: T,
}

#[repr(C)]
struct BlockSlotMirror<H, E> {
    meta: moiren_signal::PublicationMeta,
    header: H,
    elements: Box<[E]>,
}

fn shared_layout<S>() -> std::alloc::Layout {
    use crossbeam_utils::CachePadded;
    use std::{
        alloc::Layout,
        cell::UnsafeCell,
        sync::atomic::{AtomicU8, AtomicUsize},
    };
    let buffers = Layout::array::<CachePadded<UnsafeCell<S>>>(3).unwrap();
    let shared = buffers
        .extend(Layout::new::<CachePadded<AtomicU8>>())
        .unwrap()
        .0
        .pad_to_align();
    Layout::array::<AtomicUsize>(2)
        .unwrap()
        .extend(shared)
        .unwrap()
        .0
        .pad_to_align()
}

fn assert_registration_allocations(
    bus: &mut Bus,
    shared: Option<std::alloc::Layout>,
    register: impl FnOnce(&mut Bus),
) {
    let before = bus.budget_used();
    let (_, probe) = allocations::measure(|| register(bus));
    assert!(probe.count < probe.events.len(), "probe event capacity");
    assert_eq!(
        bus.budget_used() - before,
        usize::try_from(probe.bytes).unwrap(),
        "every owned heap allocation must be charged exactly"
    );
    if let Some(shared) = shared {
        assert!(
            probe.events[..probe.count]
                .iter()
                .any(|event| event.allocated
                    && event.size == shared.size()
                    && event.align == shared.align()),
            "actual upstream Arc allocation must match the pinned layout model"
        );
    }
}

#[test]
fn actual_allocator_layouts_match_directory_and_all_transport_budgets() {
    let (mut bus, probe) = allocations::measure(|| Bus::new(BusConfig::default()).unwrap());
    assert_eq!(
        bus.budget_used(),
        usize::try_from(probe.bytes).unwrap() + std::mem::size_of::<Bus>(),
        "reserved directory and bus metadata must be charged"
    );
    assert_registration_allocations(
        &mut bus,
        Some(shared_layout::<ValueSlotMirror<u64>>()),
        |bus| {
            bus.register_latest_value("value", 1_u64).unwrap();
        },
    );
    assert_registration_allocations(
        &mut bus,
        Some(shared_layout::<ValueSlotMirror<()>>()),
        |bus| {
            bus.register_latest_value("zst", ()).unwrap();
        },
    );
    assert_registration_allocations(
        &mut bus,
        Some(shared_layout::<ValueSlotMirror<Aligned>>()),
        |bus| {
            bus.register_latest_value("aligned", Aligned(5)).unwrap();
        },
    );
    assert_registration_allocations(
        &mut bus,
        Some(shared_layout::<BlockSlotMirror<u32, u64>>()),
        |bus| {
            bus.register_latest_block(
                "block",
                BlockInit {
                    header: 1_u32,
                    element: 2_u64,
                    capacity: 3,
                },
            )
            .unwrap();
        },
    );
    assert_registration_allocations(
        &mut bus,
        Some(shared_layout::<BlockSlotMirror<(), ()>>()),
        |bus| {
            bus.register_latest_block(
                "zst-block",
                BlockInit {
                    header: (),
                    element: (),
                    capacity: 3,
                },
            )
            .unwrap();
        },
    );
    assert_registration_allocations(
        &mut bus,
        Some(shared_layout::<BlockSlotMirror<Aligned, Aligned>>()),
        |bus| {
            bus.register_latest_block(
                "aligned-block",
                BlockInit {
                    header: Aligned(7),
                    element: Aligned(9),
                    capacity: 3,
                },
            )
            .unwrap();
        },
    );
    for depth in [1, 8] {
        assert_registration_allocations(&mut bus, None, |bus| {
            bus.register_stream_value(&format!("stream-{depth}"), 1_u64, depth)
                .unwrap();
        });
        assert_registration_allocations(&mut bus, None, |bus| {
            bus.register_stream_block(
                &format!("stream-block-{depth}"),
                BlockInit {
                    header: Aligned(7),
                    element: Aligned(9),
                    capacity: 3,
                },
                depth,
            )
            .unwrap();
        });
        assert_registration_allocations(&mut bus, None, |bus| {
            bus.register_stream_block(
                &format!("stream-zst-{depth}"),
                BlockInit {
                    header: (),
                    element: (),
                    capacity: 3,
                },
                depth,
            )
            .unwrap();
        });
    }
    // Read fields once to ensure these over-aligned test values remain ordinary
    // Copy payloads and never rely on Default or invalid bit patterns.
    assert_eq!(Aligned(5).0, 5);
}

fn assert_subscription_allocations<P: moiren_signal::Shape>(
    bus: &mut Bus,
    topic: &moiren_signal::Topic<P>,
    shared: std::alloc::Layout,
) {
    let before = bus.budget_used();
    let (subscriber, probe) =
        allocations::measure(|| bus.subscribe(topic, Default::default()).unwrap());
    assert_eq!(
        bus.budget_used() - before,
        usize::try_from(probe.bytes).unwrap(),
        "receiver metadata, lease, three slots and element regions must all be charged"
    );
    assert!(probe.count < probe.events.len());
    assert!(
        probe.events[..probe.count]
            .iter()
            .any(|event| event.allocated
                && event.size == shared.size()
                && event.align == shared.align())
    );
    drop(subscriber);
}

#[test]
fn actual_subscriber_allocator_layouts_match_budget_deltas() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let value = bus.register_latest_value("value", 1_u64).unwrap();
    assert_subscription_allocations(&mut bus, &value, shared_layout::<ValueSlotMirror<u64>>());
    let zst = bus.register_latest_value("zst", ()).unwrap();
    assert_subscription_allocations(&mut bus, &zst, shared_layout::<ValueSlotMirror<()>>());
    let aligned = bus.register_latest_value("aligned", Aligned(5)).unwrap();
    assert_subscription_allocations(
        &mut bus,
        &aligned,
        shared_layout::<ValueSlotMirror<Aligned>>(),
    );
    let block = bus
        .register_latest_block(
            "block",
            BlockInit {
                header: 1_u32,
                element: 2_u64,
                capacity: 3,
            },
        )
        .unwrap();
    assert_subscription_allocations(
        &mut bus,
        &block,
        shared_layout::<BlockSlotMirror<u32, u64>>(),
    );
    let zst_block = bus
        .register_latest_block(
            "zst-block",
            BlockInit {
                header: (),
                element: (),
                capacity: 3,
            },
        )
        .unwrap();
    assert_subscription_allocations(
        &mut bus,
        &zst_block,
        shared_layout::<BlockSlotMirror<(), ()>>(),
    );
    let aligned_block = bus
        .register_latest_block(
            "aligned-block",
            BlockInit {
                header: Aligned(7),
                element: Aligned(9),
                capacity: 3,
            },
        )
        .unwrap();
    assert_subscription_allocations(
        &mut bus,
        &aligned_block,
        shared_layout::<BlockSlotMirror<Aligned, Aligned>>(),
    );
}

#[test]
fn rejected_subscriptions_allocate_nothing() {
    let mut bus = Bus::new(BusConfig {
        max_subscribers_per_topic: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_latest_value("latest", 99_u32).unwrap();
    let _subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let before = bus.budget_used();
    let (_, probe) = allocations::measure(|| {
        assert!(matches!(
            bus.subscribe(&topic, Default::default()),
            Err(moiren_signal::SubscribeError::SubscriberLimit)
        ));
        assert!(matches!(
            bus.subscribe(
                &topic,
                moiren_signal::SubscriptionOptions {
                    stream_depth: Some(8)
                }
            ),
            Err(moiren_signal::SubscribeError::InvalidOptions)
        ));
    });
    assert_eq!(probe.count, 0);
    assert_eq!(bus.budget_used(), before);
}

#[test]
fn checked_failures_allocate_nothing_and_do_not_advance_identity() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let before = bus.budget_used();
    let (result, probe) = allocations::measure(|| {
        bus.register_latest_block_with("retry", usize::MAX, || 1_u64, || 2_u64)
    });
    assert!(matches!(result, Err(RegisterError::SizeOverflow)));
    assert_eq!(probe.count, 0);
    assert_eq!(bus.budget_used(), before);
    let topic = bus.register_latest_value("retry", 1_u64).unwrap();
    assert_eq!(
        topic.id(),
        bus.resolve::<LatestValue<u64>>("retry").unwrap().id()
    );
    let (_, probe) = allocations::measure(|| {
        assert!(matches!(
            bus.register_latest_value("retry", 2_u64),
            Err(RegisterError::DuplicateName)
        ));
        assert!(matches!(
            bus.register_latest_value("bad\0name", 2_u64),
            Err(RegisterError::InvalidName)
        ));
        assert!(matches!(
            bus.register_stream_value("zero", 2_u64, 0),
            Err(RegisterError::InvalidCapacity)
        ));
    });
    assert_eq!(probe.count, 0);
    let (_, probe) = allocations::measure(|| {
        assert!(matches!(
            Bus::new(BusConfig {
                byte_budget: 0,
                ..BusConfig::default()
            }),
            Err(RegisterError::BudgetExceeded)
        ));
    });
    assert_eq!(probe.count, 0);
}

#[test]
fn zst_capacity_multiplication_is_checked_before_factories() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let before = bus.budget_used();
    let (result, probe) = allocations::measure(|| {
        bus.register_latest_block_with(
            "zst-overflow",
            usize::MAX,
            || -> () { panic!("header must not run") },
            || -> () { panic!("element must not run") },
        )
    });
    assert!(matches!(result, Err(RegisterError::SizeOverflow)));
    assert_eq!(probe.count, 0);
    assert_eq!(bus.budget_used(), before);
    bus.register_latest_block(
        "zst-overflow",
        BlockInit {
            header: (),
            element: (),
            capacity: 4,
        },
    )
    .unwrap();
}
