use moiren_signal::{
    BlockInit, Bus, BusConfig, LatestBlock, LatestValue, LookupError, RegisterError, StreamBlock,
    StreamValue,
};
use std::{
    num::NonZeroU32,
    sync::{Arc, atomic::Ordering},
};

mod allocations {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::{Cell, RefCell},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    #[derive(Clone, Copy, Debug)]
    pub struct Event {
        pub pointer: usize,
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
                pointer: 0,
                size: 0,
                align: 0,
                allocated: false,
            }; 128],
        };
    }

    thread_local! { static PROBE: Cell<Probe> = const { Cell::new(Probe::EMPTY) }; }
    thread_local! { static PAUSE: RefCell<Option<Arc<DeallocationGate>>> = const { RefCell::new(None) }; }

    pub struct DeallocationGate {
        target: usize,
        pub reached: AtomicBool,
        pub resume: AtomicBool,
        pub freed_bytes: AtomicUsize,
    }

    impl DeallocationGate {
        pub fn new(target: usize) -> Self {
            Self {
                target,
                reached: AtomicBool::new(false),
                resume: AtomicBool::new(false),
                freed_bytes: AtomicUsize::new(0),
            }
        }
    }

    pub fn pause_deallocation<T>(gate: Arc<DeallocationGate>, operation: impl FnOnce() -> T) -> T {
        struct Stop;
        impl Drop for Stop {
            fn drop(&mut self) {
                // Drop the gate outside the TLS borrow: its own deallocation
                // also passes through this allocator.
                let gate = PAUSE.with(|state| state.borrow_mut().take());
                drop(gate);
            }
        }
        PAUSE.with(|state| *state.borrow_mut() = Some(gate));
        let stop = Stop;
        let result = operation();
        drop(stop);
        result
    }

    pub struct Allocator;
    fn record(pointer: *mut u8, layout: Layout, allocated: bool) {
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
                        pointer: pointer as usize,
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
    // uses preinitialized TLS cells/atomics and never allocates itself.
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: The caller supplies the allocator's valid Layout.
            let pointer = unsafe { System.alloc(layout) };
            if !pointer.is_null() {
                record(pointer, layout, true);
            }
            pointer
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            let _ = PAUSE.try_with(|state| {
                if let Some(gate) = state.borrow().as_ref()
                    && pointer as usize == gate.target
                {
                    gate.reached.store(true, Ordering::Release);
                    while !gate.resume.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                }
            });
            // SAFETY: Pointer/layout are passed through to their allocator.
            unsafe { System.dealloc(pointer, layout) };
            record(pointer, layout, false);
            let _ = PAUSE.try_with(|state| {
                if let Some(gate) = state.borrow().as_ref() {
                    gate.freed_bytes.fetch_add(layout.size(), Ordering::Relaxed);
                }
            });
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

/// A worker thread parked inside the allocator on a gate's target
/// deallocation. Dropping the guard (including during a panic unwind) resumes
/// the worker so `thread::scope` can join it.
struct ParkedDrop<'scope, T> {
    gate: Arc<allocations::DeallocationGate>,
    worker: Option<std::thread::ScopedJoinHandle<'scope, T>>,
}

impl<T> ParkedDrop<'_, T> {
    fn resume_and_join(mut self) -> T {
        self.gate.resume.store(true, Ordering::Release);
        let worker = self.worker.take().expect("worker handle consumed once");
        worker.join().expect("gated drop worker panicked")
    }
}

impl<T> Drop for ParkedDrop<'_, T> {
    fn drop(&mut self) {
        // Unblock the worker even if a regression assertion panics before
        // `resume_and_join`; `thread::scope` joins it during unwind.
        self.gate.resume.store(true, Ordering::Release);
    }
}

/// Spawns `operation` on `scope` with the gate's target deallocation parked
/// inside the allocator, then waits (deadline-bounded) until the worker is
/// parked on it. Regression assertions may run between this call and
/// [`ParkedDrop::resume_and_join`]: the observed state is exactly the
/// mid-`Drop` state the gate freezes.
fn spawn_parked_drop<'scope, 'env: 'scope, T>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    gate: &Arc<allocations::DeallocationGate>,
    stuck: &str,
    operation: impl FnOnce() -> T + Send + 'scope,
) -> ParkedDrop<'scope, T>
where
    T: Send + 'scope,
{
    let worker_gate = Arc::clone(gate);
    let worker = scope.spawn(move || allocations::pause_deallocation(worker_gate, operation));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !gate.reached.load(Ordering::Acquire) {
        assert!(std::time::Instant::now() < deadline, "{stuck}");
        std::thread::yield_now();
    }
    ParkedDrop {
        gate: Arc::clone(gate),
        worker: Some(worker),
    }
}

fn assert_concurrent_receiver_drop_stays_budgeted<P: moiren_signal::Shape>(
    register: impl Fn(&mut Bus) -> moiren_signal::Topic<P>,
) {
    use std::sync::atomic::Ordering;

    let mut measure = Bus::new(BusConfig::default()).unwrap();
    let topic = register(&mut measure);
    let baseline = measure.budget_used();
    let subscriber = measure.subscribe(&topic, Default::default()).unwrap();
    let charge = measure.budget_used() - baseline;
    drop(subscriber);
    drop(measure);

    let limit = baseline + 2 * charge;
    let mut bus = Bus::new(BusConfig {
        byte_budget: limit,
        ..Default::default()
    })
    .unwrap();
    let topic = register(&mut bus);
    let (subscriber, probe) =
        allocations::measure(|| bus.subscribe(&topic, Default::default()).unwrap());
    assert_eq!(probe.bytes, charge as isize);
    let before = bus.budget_used();
    // The receiver box is the final preparation allocation. Pause its actual
    // deallocation after all fields (including the final strong lease) dropped.
    let receiver = probe.events[..probe.count]
        .iter()
        .rfind(|event| event.allocated)
        .unwrap();
    let gate = Arc::new(allocations::DeallocationGate::new(receiver.pointer));

    let replacement = std::thread::scope(|scope| {
        let worker = spawn_parked_drop(scope, &gate, "receiver Drop never reached dealloc", || {
            allocations::measure(|| drop(subscriber)).1
        });

        let (replacement, probe) =
            allocations::measure(|| bus.subscribe(&topic, Default::default()).unwrap());
        let live =
            before as isize + probe.bytes - gate.freed_bytes.load(Ordering::Acquire) as isize;
        let charged = bus.budget_used();
        let (attempt, rejected) =
            allocations::measure(|| bus.subscribe(&topic, Default::default()));
        assert!(
            live + rejected.bytes <= limit as isize,
            "reusing an in-progress Drop's budget exceeds the configured limit: {} live, {limit} allowed",
            live + rejected.bytes
        );
        assert!(
            live <= charged as isize,
            "in-progress receiver Drop is still live: {live} bytes, only {} charged",
            charged
        );
        assert_eq!(bus.budget_used(), limit);
        assert!(matches!(
            attempt,
            Err(moiren_signal::SubscribeError::BudgetExceeded)
        ));
        assert_eq!(
            rejected.count, 0,
            "occupied budget must reject before allocation"
        );
        assert!(live <= limit as isize);

        let dropped = worker.resume_and_join();
        assert!(dropped.bytes <= -(receiver.size as isize));
        assert_eq!(
            dropped.bytes,
            -(gate.freed_bytes.load(Ordering::Acquire) as isize)
        );
        replacement
    });
    drop(replacement);
}

#[test]
fn concurrent_value_receiver_drop_keeps_live_allocations_charged() {
    assert_concurrent_receiver_drop_stays_budgeted(|bus| {
        bus.register_latest_value("latest", 99_u32).unwrap()
    });
}

#[test]
fn bounded_pump_retires_at_most_one_stream_payload_owner_per_paid_step() {
    use moiren_signal::{PumpBudget, SubscriptionOptions};
    let mut bus = Bus::new(BusConfig {
        max_topics: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus
        .register_stream_block(
            "retire",
            BlockInit {
                header: 7_u32,
                element: 9_u64,
                capacity: 2,
            },
            2,
        )
        .unwrap();
    let subscriber = bus
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(1000),
            },
        )
        .unwrap();
    drop(subscriber);
    let before = bus.budget_used();
    let (report, probe) = allocations::measure(|| {
        bus.pump(PumpBudget {
            topic_checks: 1,
            deliveries: 1,
            copy_bytes: 0,
        })
    });
    assert_eq!(report.deliveries, 1);
    let freed = probe.events[..probe.count.min(probe.events.len())]
        .iter()
        .filter(|event| !event.allocated)
        .count();
    assert!(
        freed <= 1,
        "one paid retirement step freed {freed} allocations ({} events)",
        probe.count
    );
    assert!(report.has_pending);
    assert_eq!(
        bus.budget_used(),
        before,
        "partially retired pool remains reserved"
    );
}

#[test]
fn unregister_and_final_topic_release_refund_only_actual_released_allocations() {
    fn check<P: moiren_signal::Shape>(register: impl Fn(&mut Bus) -> moiren_signal::Topic<P>) {
        let mut bus = Bus::new(Default::default()).unwrap();
        let baseline = bus.budget_used();
        let (topic, registration) = allocations::measure(|| register(&mut bus));
        let (subscriber, subscription) =
            allocations::measure(|| bus.subscribe(&topic, Default::default()).unwrap());
        let total = bus.budget_used() - baseline;
        assert_eq!(total as isize, registration.bytes + subscription.bytes);
        let (_, closure) = allocations::measure(|| bus.unregister(&topic).unwrap());
        let still_live = total as isize + closure.bytes;
        assert!(bus.budget_used() as isize - baseline as isize >= still_live);
        assert!(bus.budget_used() > baseline);
        let (_, receiver_drop) = allocations::measure(|| drop(subscriber));
        assert!(
            bus.budget_used() > baseline,
            "old Topic retains charged state"
        );
        let (_, topic_drop) = allocations::measure(|| drop(topic));
        assert_eq!(bus.budget_used(), baseline);
        assert_eq!(
            registration.bytes
                + subscription.bytes
                + closure.bytes
                + receiver_drop.bytes
                + topic_drop.bytes,
            0
        );
    }
    check(|bus| bus.register_latest_value("value", Aligned(7)).unwrap());
    check(|bus| bus.register_stream_value("value", (), 20).unwrap());
    check(|bus| {
        bus.register_latest_block(
            "block",
            BlockInit {
                header: Aligned(7),
                element: Aligned(8),
                capacity: 3,
            },
        )
        .unwrap()
    });
    check(|bus| {
        bus.register_stream_block(
            "block",
            BlockInit {
                header: (),
                element: (),
                capacity: 3,
            },
            20,
        )
        .unwrap()
    });
    check(|bus| {
        bus.register_stream_block(
            "block",
            BlockInit {
                header: 7_u32,
                element: 8_u64,
                capacity: 3,
            },
            20,
        )
        .unwrap()
    });
}

#[test]
fn final_state_and_reservation_deallocation_stay_charged_until_allocator_returns() {
    for reservation in [false, true] {
        let mut bus = Bus::new(BusConfig::default()).unwrap();
        let baseline = bus.budget_used();
        let (topic, prepared) =
            allocations::measure(|| bus.register_latest_value("final", 7_u32).unwrap());
        let (_, removed) = allocations::measure(|| bus.unregister(&topic).unwrap());
        let survivors: Vec<_> = prepared.events[..prepared.count]
            .iter()
            .filter(|event| {
                event.allocated
                    && !removed.events[..removed.count]
                        .iter()
                        .any(|freed| !freed.allocated && freed.pointer == event.pointer)
            })
            .collect();
        assert_eq!(
            survivors.len(),
            2,
            "only Topic state and its reservation remain"
        );
        let target = survivors[usize::from(reservation)].pointer;
        let gate = Arc::new(allocations::DeallocationGate::new(target));
        let before = bus.budget_used();
        std::thread::scope(|scope| {
            let worker = spawn_parked_drop(
                scope,
                &gate,
                "final deallocation never reached allocator",
                || drop(topic),
            );
            assert_eq!(
                bus.budget_used(),
                before,
                "no refund before actual final allocation release"
            );
            worker.resume_and_join();
        });
        assert_eq!(bus.budget_used(), baseline);
    }
}

#[test]
fn closed_receiver_box_deallocation_does_not_refund_its_live_reservation() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let baseline = bus.budget_used();
    let topic = bus
        .register_stream_block(
            "closed",
            BlockInit {
                header: 7_u32,
                element: 8_u64,
                capacity: 5,
            },
            2,
        )
        .unwrap();
    let (subscriber, prepared) =
        allocations::measure(|| bus.subscribe(&topic, Default::default()).unwrap());
    let receiver = prepared.events[..prepared.count]
        .iter()
        .rfind(|event| event.allocated)
        .unwrap();
    let gate = Arc::new(allocations::DeallocationGate::new(receiver.pointer));
    bus.unregister(&topic).unwrap();
    drop(topic);
    let before = bus.budget_used();
    std::thread::scope(|scope| {
        let worker = spawn_parked_drop(
            scope,
            &gate,
            "closed receiver never reached dealloc",
            || drop(subscriber),
        );
        assert_eq!(
            bus.budget_used(),
            before,
            "receiver reservation survives its box deallocator"
        );
        worker.resume_and_join();
    });
    assert_eq!(bus.budget_used(), baseline);
}

#[test]
fn concurrent_block_receiver_drop_keeps_live_allocations_charged() {
    assert_concurrent_receiver_drop_stays_budgeted(|bus| {
        bus.register_latest_block(
            "block",
            BlockInit {
                header: 99_u32,
                element: 7_u64,
                capacity: 128,
            },
        )
        .unwrap()
    });
}

#[test]
fn concurrent_stream_receiver_drop_keeps_live_allocations_charged() {
    assert_concurrent_receiver_drop_stays_budgeted(|bus| {
        bus.register_stream_value("stream", 99_u32, 2).unwrap()
    });
    assert_concurrent_receiver_drop_stays_budgeted(|bus| {
        bus.register_stream_block(
            "stream-block",
            BlockInit {
                header: 99_u32,
                element: 7_u64,
                capacity: 128,
            },
            2,
        )
        .unwrap()
    });
}

#[test]
fn stream_subscriber_budget_matches_owner_rings_loan_and_retained_storage() {
    fn check<P: moiren_signal::Shape>(bus: &mut Bus, topic: &moiren_signal::Topic<P>) {
        let mut replacements = Vec::new();
        for depth in [1, 2, 8] {
            let before = bus.budget_used();
            let (subscriber, prepared) = allocations::measure(|| {
                bus.subscribe(
                    topic,
                    moiren_signal::SubscriptionOptions {
                        stream_depth: Some(depth),
                    },
                )
                .unwrap()
            });
            assert!(prepared.count < prepared.events.len());
            assert_eq!(
                bus.budget_used() as isize - before as isize,
                prepared.bytes,
                "all receiver/ring/loan payload layouts must be charged"
            );
            let charged = bus.budget_used();
            let (_, dropped) = allocations::measure(|| drop(subscriber));
            assert_eq!(
                bus.budget_used(),
                charged,
                "retained dispatcher owners cannot be refunded yet"
            );
            let (replacement, reclaimed) = allocations::measure(|| {
                bus.subscribe(
                    topic,
                    moiren_signal::SubscriptionOptions {
                        stream_depth: Some(depth),
                    },
                )
                .unwrap()
            });
            assert_eq!(bus.budget_used(), charged);
            assert_eq!(
                prepared.bytes + dropped.bytes + reclaimed.bytes,
                charged as isize - before as isize
            );
            replacements.push(replacement);
        }
    }
    let mut bus = Bus::new(Default::default()).unwrap();
    let value = bus.register_stream_value("value", 7_u64, 2).unwrap();
    check(&mut bus, &value);
    let zst = bus.register_stream_value("zst", (), 2).unwrap();
    check(&mut bus, &zst);
    let aligned = bus.register_stream_value("aligned", Aligned(7), 2).unwrap();
    check(&mut bus, &aligned);
    let block = bus
        .register_stream_block(
            "block",
            BlockInit {
                header: 7_u32,
                element: 8_u64,
                capacity: 3,
            },
            2,
        )
        .unwrap();
    check(&mut bus, &block);
    let block = bus
        .register_stream_block(
            "zst-block",
            BlockInit {
                header: (),
                element: (),
                capacity: 3,
            },
            2,
        )
        .unwrap();
    check(&mut bus, &block);
    let block = bus
        .register_stream_block(
            "aligned-block",
            BlockInit {
                header: Aligned(7),
                element: Aligned(8),
                capacity: 3,
            },
            2,
        )
        .unwrap();
    check(&mut bus, &block);
}

#[test]
fn stream_subscription_budget_boundary_and_rejections_allocate_nothing() {
    use moiren_signal::{SubscribeError, SubscriptionOptions};
    let mut measure = Bus::new(Default::default()).unwrap();
    let topic = measure
        .register_stream_block(
            "block",
            BlockInit {
                header: 7_u32,
                element: 8_u64,
                capacity: 3,
            },
            2,
        )
        .unwrap();
    let base = measure.budget_used();
    let _subscriber = measure
        .subscribe(
            &topic,
            SubscriptionOptions {
                stream_depth: Some(2),
            },
        )
        .unwrap();
    let limit = measure.budget_used();
    for (budget, success) in [(limit, true), (limit - 1, false)] {
        let mut bus = Bus::new(BusConfig {
            byte_budget: budget,
            ..Default::default()
        })
        .unwrap();
        let topic = bus
            .register_stream_block(
                "block",
                BlockInit {
                    header: 7_u32,
                    element: 8_u64,
                    capacity: 3,
                },
                2,
            )
            .unwrap();
        assert_eq!(bus.budget_used(), base);
        let (result, probe) = allocations::measure(|| {
            bus.subscribe(
                &topic,
                SubscriptionOptions {
                    stream_depth: Some(2),
                },
            )
        });
        if success {
            assert!(result.is_ok());
            assert_eq!(bus.budget_used(), limit);
        } else {
            assert!(matches!(result, Err(SubscribeError::BudgetExceeded)));
            assert_eq!(probe.count, 0);
            assert_eq!(bus.budget_used(), base);
        }
    }
    let mut bus = Bus::new(BusConfig {
        max_subscribers_per_topic: 1,
        ..Default::default()
    })
    .unwrap();
    let topic = bus.register_stream_value("value", 7_u32, 2).unwrap();
    let before = bus.budget_used();
    let (_, probe) = allocations::measure(|| {
        assert!(matches!(
            bus.subscribe(
                &topic,
                SubscriptionOptions {
                    stream_depth: Some(0)
                }
            ),
            Err(SubscribeError::InvalidCapacity)
        ));
    });
    assert_eq!(probe.count, 0);
    assert_eq!(bus.budget_used(), before);
    let _subscriber = bus.subscribe(&topic, Default::default()).unwrap();
    let before = bus.budget_used();
    let (_, probe) = allocations::measure(|| {
        assert!(matches!(
            bus.subscribe(&topic, Default::default()),
            Err(SubscribeError::SubscriberLimit)
        ));
    });
    assert_eq!(probe.count, 0);
    assert_eq!(bus.budget_used(), before);
}

#[test]
fn retiring_receiver_keeps_its_sender_slot_until_drop_completes() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus.register_latest_value("latest", 99_u32).unwrap();
    let subscribers: Vec<_> = (0..63)
        .map(|_| bus.subscribe(&topic, Default::default()).unwrap())
        .collect();
    let (subscriber, probe) =
        allocations::measure(|| bus.subscribe(&topic, Default::default()).unwrap());
    let receiver = probe.events[..probe.count]
        .iter()
        .rfind(|event| event.allocated)
        .unwrap();
    let gate = Arc::new(allocations::DeallocationGate::new(receiver.pointer));
    let before = bus.budget_used();
    std::thread::scope(|scope| {
        let worker = spawn_parked_drop(scope, &gate, "receiver Drop never reached dealloc", || {
            drop(subscriber)
        });
        let (attempt, probe) = allocations::measure(|| bus.subscribe(&topic, Default::default()));
        assert!(matches!(
            attempt,
            Err(moiren_signal::SubscribeError::SubscriberLimit)
        ));
        assert_eq!(probe.count, 0);
        assert_eq!(bus.budget_used(), before);
        worker.resume_and_join();
    });
    // Once deallocation completes, the same fixed slot and full old credit may
    // be reused. No in-progress Drop may set a stale flag for this replacement.
    let replacement = bus.subscribe(&topic, Default::default()).unwrap();
    assert_eq!(bus.budget_used(), before);
    assert!(matches!(
        bus.subscribe(&topic, Default::default()),
        Err(moiren_signal::SubscribeError::SubscriberLimit)
    ));
    drop(replacement);
    drop(subscribers);
}

#[test]
fn completed_receiver_drop_refunds_only_after_sender_reclamation() {
    let mut bus = Bus::new(BusConfig::default()).unwrap();
    let topic = bus.register_latest_value("latest", 99_u32).unwrap();
    let baseline = bus.budget_used();
    let (subscriber, prepared) =
        allocations::measure(|| bus.subscribe(&topic, Default::default()).unwrap());
    let charged = bus.budget_used();
    let (_, dropped) = allocations::measure(|| drop(subscriber));
    assert_eq!(bus.budget_used(), charged);
    let (_replacement, reclaimed) =
        allocations::measure(|| bus.subscribe(&topic, Default::default()).unwrap());
    assert_eq!(bus.budget_used(), charged);
    assert_eq!(
        prepared.bytes + dropped.bytes + reclaimed.bytes,
        (charged - baseline) as isize,
        "completed retirement must release every old allocation before its refund"
    );
}

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
