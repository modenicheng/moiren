//! Bounded control-side recovery notifications, published after native teardown.
use std::sync::{Arc, Mutex, TryLockError};

use moiren_core::graph::NodeId;
use moiren_engine::host::{NodeConfig, ResourceEvent};

#[derive(Clone, Default)]
pub(super) struct RecoveryMonitor {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    active: Option<Arc<Registration>>,
    attempt_used: bool,
    pending: Option<ResourceEvent>,
    event_generation: u64,
}

struct Registration {
    owner_generation: u64,
    configurations: Vec<(NodeId, NodeConfig)>,
    processing_rate: u32,
}

#[derive(Clone)]
pub(super) struct RecoveryTicket {
    state: Arc<Mutex<State>>,
    registration: Arc<Registration>,
}

impl RecoveryMonitor {
    pub(super) fn register(
        &self,
        owner_generation: u64,
        mut configurations: Vec<(NodeId, NodeConfig)>,
        processing_rate: u32,
    ) -> RecoveryTicket {
        // Node identity defines the configuration set, not caller iteration order.
        configurations.sort_by_key(|(node, _)| *node);
        let mut registration = Arc::new(Registration {
            owner_generation,
            configurations,
            processing_rate,
        });
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(active) = &state.active {
            if owner_generation < active.owner_generation {
                // Return an inert ticket without changing the current budget or slot.
                return RecoveryTicket {
                    state: self.state.clone(),
                    registration,
                };
            }
            let unchanged = active.configurations == registration.configurations
                && active.processing_rate == processing_rate;
            if unchanged && owner_generation == active.owner_generation {
                registration = active.clone();
            } else {
                // ResourceEvent carries only one anchor's config, so it cannot
                // fence an old owner, a changed duplex peer, or a changed rate.
                state.pending = None;
            }
            if !unchanged {
                state.attempt_used = false;
            }
        }
        // A newer owner never replenishes an unchanged configuration's allowance.
        // There is deliberately no healthy-run timer or automatic replenishment.
        state.active = Some(registration.clone());
        RecoveryTicket {
            state: self.state.clone(),
            registration,
        }
    }

    /// Supervisor polling never waits for a registration or owner-side request.
    pub(super) fn poll(&self) -> Option<ResourceEvent> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(poison)) => poison.into_inner(),
        };
        state.pending.take()
    }
}

impl RecoveryTicket {
    /// Call on the control owner thread only after stop/dispose/release succeeded.
    /// The caller owns teardown ordering; native callbacks must never call this.
    pub(super) fn request(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // Identity also fences an old ticket if settings change away and back
        // within the same owner generation.
        if !state
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, &self.registration))
            || state.attempt_used
            || state.pending.is_some()
        {
            return false;
        }
        let Some((node, configuration)) = self.registration.configurations.first() else {
            return false;
        };
        let Some(generation) = state.event_generation.checked_add(1) else {
            // Exhaustion is terminal: wrapping or saturation would reuse an event ID.
            return false;
        };
        let event = ResourceEvent {
            node: *node,
            configuration: configuration.clone(),
            generation,
            available: true,
        };
        state.pending = Some(event);
        state.event_generation = generation;
        state.attempt_used = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moiren_core::graph::{LogicalGraph, NodeId, NodeKind};
    use moiren_engine::{
        audio_config::{AudioBackend, SampleFormat},
        boundary::ClockRole,
        host::NodeConfig,
        node::{
            DeviceMode, DeviceOptions, EndpointSelector, InputConfig, InputSource, OutputConfig,
            OutputTarget,
        },
    };

    fn configurations() -> Vec<(NodeId, NodeConfig)> {
        let mut graph = LogicalGraph::new();
        let input = graph.create_node(NodeKind::Source, 2).unwrap();
        let output = graph.create_node(NodeKind::Sink, 2).unwrap();
        let endpoint = EndpointSelector::PinnedEndpoint {
            endpoint_id: "{188135E1-7171-3434-854F-01A3C71F3DF9}".into(),
            stable_id: Some("exact saved driver identity".into()),
        };
        let device = DeviceOptions {
            backend: AudioBackend::Asio,
            mode: DeviceMode::Exclusive,
            hardware_sample_rate: Some(96_000),
            sample_format: SampleFormat::Float32,
            period_frames: Some(128),
            clock_role: ClockRole::Master,
        };
        vec![
            (
                input,
                NodeConfig::Input(InputConfig {
                    source: InputSource::Physical(endpoint.clone()),
                    channels: 2,
                    device,
                }),
            ),
            (
                output,
                NodeConfig::Output(OutputConfig {
                    target: OutputTarget::Physical(endpoint),
                    channels: 2,
                    device,
                }),
            ),
        ]
    }

    fn change_output_buffer(configurations: &mut [(NodeId, NodeConfig)], frames: usize) {
        let NodeConfig::Output(output) = &mut configurations[1].1 else {
            panic!("expected output configuration");
        };
        output.device.period_frames = Some(frames);
    }

    #[test]
    fn unused_monitor_and_empty_registration_have_no_event() {
        let monitor = RecoveryMonitor::default();
        assert!(monitor.poll().is_none());
        let ticket = monitor.register(1, Vec::new(), 48_000);
        assert!(!ticket.request());
        assert!(monitor.poll().is_none());
        assert!(monitor.register(2, configurations(), 48_000).request());
    }

    #[test]
    fn request_preserves_exact_saved_anchor_configuration() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let expected = configurations[0].clone();
        let ticket = monitor.register(7, configurations, 48_000);

        assert!(ticket.request());
        let event = monitor.poll().unwrap();
        assert_eq!(event.node, expected.0);
        assert_eq!(event.configuration, expected.1);
        assert!(event.available);
        assert_eq!(event.generation, 1);
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn one_attempt_is_shared_by_ticket_clones_and_monitor_clones() {
        let monitor = RecoveryMonitor::default();
        let other_monitor = monitor.clone();
        let ticket = monitor.register(1, configurations(), 48_000);
        let duplicate = ticket.clone();

        assert!(duplicate.request());
        assert!(!ticket.request());
        assert!(other_monitor.poll().is_some());
        assert!(!duplicate.request());
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn consumed_budget_survives_new_owner_generations_for_unchanged_settings() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        assert!(
            monitor
                .register(1, configurations.clone(), 48_000)
                .request()
        );
        assert!(monitor.poll().is_some());

        for generation in 2..=10 {
            let ticket = monitor.register(generation, configurations.clone(), 48_000);
            assert!(
                !ticket.request(),
                "generation {generation} reset the budget"
            );
            assert!(monitor.poll().is_none());
        }
    }

    #[test]
    fn duplicate_registration_does_not_reset_consumed_budget() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let first = monitor.register(4, configurations.clone(), 48_000);
        let duplicate = monitor.register(4, configurations.clone(), 48_000);
        assert!(first.request());
        assert!(!duplicate.request());
        assert!(monitor.poll().is_some());
        assert!(!monitor.register(4, configurations, 48_000).request());
    }

    #[test]
    fn newer_owner_fences_old_ticket_even_before_budget_is_used() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let old = monitor.register(8, configurations.clone(), 48_000);
        let current = monitor.register(9, configurations, 48_000);

        assert!(!old.request());
        assert!(monitor.poll().is_none());
        assert!(current.request());
        assert!(monitor.poll().is_some());
    }

    #[test]
    fn stale_registration_cannot_replace_current_settings_or_reset_budget() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let current = monitor.register(10, configurations.clone(), 48_000);
        let mut stale_configurations = configurations.clone();
        change_output_buffer(&mut stale_configurations, 256);
        let stale = monitor.register(9, stale_configurations.clone(), 44_100);

        assert!(!stale.request());
        assert!(current.request());
        assert!(monitor.poll().is_some());
        assert!(!monitor.register(8, stale_configurations, 96_000).request());
        assert!(!monitor.register(11, configurations, 48_000).request());
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn changed_non_anchor_settings_reset_budget_and_fence_prior_ticket() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        let old = monitor.register(1, configurations.clone(), 48_000);
        assert!(old.request());
        assert!(monitor.poll().is_some());
        change_output_buffer(&mut configurations, 256);
        let current = monitor.register(2, configurations, 48_000);

        assert!(!old.request());
        assert!(current.request());
        assert_eq!(monitor.poll().unwrap().generation, 2);
        assert!(!current.request());
    }

    #[test]
    fn changed_configuration_with_same_owner_generation_fences_prior_ticket() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        let old = monitor.register(1, configurations.clone(), 48_000);
        change_output_buffer(&mut configurations, 256);
        let current = monitor.register(1, configurations, 48_000);

        assert!(!old.request());
        assert!(current.request());
        assert!(monitor.poll().is_some());
    }

    #[test]
    fn returning_to_prior_settings_does_not_revive_old_registration_ticket() {
        let monitor = RecoveryMonitor::default();
        let original = configurations();
        let old = monitor.register(1, original.clone(), 48_000);
        let mut changed = original.clone();
        change_output_buffer(&mut changed, 256);
        let intermediate = monitor.register(1, changed, 48_000);
        let current = monitor.register(1, original, 48_000);

        assert!(!old.request());
        assert!(!intermediate.request());
        assert!(current.request());
        assert!(monitor.poll().is_some());
    }

    #[test]
    fn changed_anchor_emits_new_configuration_without_normalization() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        assert!(
            monitor
                .register(1, configurations.clone(), 48_000)
                .request()
        );
        assert!(monitor.poll().is_some());
        let NodeConfig::Input(input) = &mut configurations[0].1 else {
            panic!("expected input configuration");
        };
        input.channels = 1;
        input.device.hardware_sample_rate = None;
        input.device.sample_format = SampleFormat::Native;
        input.device.period_frames = None;
        input.source = InputSource::Physical(EndpointSelector::PinnedEndpoint {
            endpoint_id: " exact opaque driver ID ".into(),
            stable_id: None,
        });
        let expected = configurations[0].clone();

        assert!(monitor.register(2, configurations, 48_000).request());
        let event = monitor.poll().unwrap();
        assert_eq!(event.node, expected.0);
        assert_eq!(event.configuration, expected.1);
    }

    #[test]
    fn processing_rate_change_resets_budget_without_changing_node_configuration() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let old = monitor.register(1, configurations.clone(), 48_000);
        assert!(old.request());
        assert!(monitor.poll().is_some());
        let current = monitor.register(2, configurations.clone(), 44_100);

        assert!(!old.request());
        assert!(current.request());
        assert_eq!(monitor.poll().unwrap().configuration, configurations[0].1);
        assert!(!monitor.register(3, configurations, 44_100).request());
    }

    #[test]
    fn processing_rate_change_with_same_owner_generation_fences_prior_ticket() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let old = monitor.register(1, configurations.clone(), 48_000);
        let current = monitor.register(1, configurations, 44_100);

        assert!(!old.request());
        assert!(current.request());
        assert!(monitor.poll().is_some());
    }

    #[test]
    fn reordering_unchanged_node_configurations_does_not_reset_budget() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        assert!(
            monitor
                .register(1, configurations.clone(), 48_000)
                .request()
        );
        assert!(monitor.poll().is_some());
        configurations.reverse();

        assert!(!monitor.register(2, configurations, 48_000).request());
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn node_membership_change_resets_budget() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        assert!(
            monitor
                .register(1, configurations.clone(), 48_000)
                .request()
        );
        assert!(monitor.poll().is_some());
        let expected = configurations.remove(0);

        assert!(
            monitor
                .register(2, vec![expected.clone()], 48_000)
                .request()
        );
        let event = monitor.poll().unwrap();
        assert_eq!(event.node, expected.0);
        assert_eq!(event.configuration, expected.1);
    }

    #[test]
    fn requests_coalesce_without_superseding_unconsumed_event() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let first = monitor.register(1, configurations.clone(), 48_000);
        assert!(first.request());
        assert!(!first.clone().request());
        let duplicate = monitor.register(1, configurations, 48_000);

        assert!(!duplicate.request());
        assert!(!duplicate.clone().request());
        let event = monitor.poll().unwrap();
        assert_eq!(event.generation, 1);
        assert_eq!(event.configuration, self::configurations()[0].1);
        assert!(event.available);
        assert!(monitor.poll().is_none());
        assert!(!duplicate.request());
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn changed_non_anchor_configuration_discards_pending_old_event() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        let anchor = configurations[0].clone();
        let old = monitor.register(1, configurations.clone(), 48_000);
        assert!(old.request());
        change_output_buffer(&mut configurations, 256);
        let current = monitor.register(1, configurations, 48_000);

        assert!(monitor.poll().is_none());
        assert!(!old.request());
        assert!(current.request());
        let event = monitor.poll().unwrap();
        assert_eq!(event.generation, 2);
        assert_eq!(event.node, anchor.0);
        assert_eq!(event.configuration, anchor.1);
        assert!(!current.request());
    }

    #[test]
    fn changed_processing_rate_discards_pending_old_event() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let old = monitor.register(1, configurations.clone(), 48_000);
        assert!(old.request());
        let current = monitor.register(1, configurations, 44_100);

        assert!(monitor.poll().is_none());
        assert!(!old.request());
        assert!(current.request());
        assert_eq!(monitor.poll().unwrap().generation, 2);
    }

    #[test]
    fn unchanged_newer_owner_discards_pending_event_without_replenishing_budget() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        let old = monitor.register(1, configurations.clone(), 48_000);
        assert!(old.request());
        let current = monitor.register(2, configurations.clone(), 48_000);

        assert!(monitor.poll().is_none());
        assert!(!old.request());
        assert!(!current.request());
        assert!(!monitor.register(3, configurations, 48_000).request());
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn stale_registration_cannot_discard_current_pending_event() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        let current = monitor.register(10, configurations.clone(), 48_000);
        assert!(current.request());
        change_output_buffer(&mut configurations, 256);
        let stale = monitor.register(9, configurations, 96_000);

        assert!(!stale.request());
        assert_eq!(monitor.poll().unwrap().generation, 1);
        assert!(!current.request());
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn discarded_pending_events_do_not_reset_monotonic_event_generation() {
        let monitor = RecoveryMonitor::default();
        let configurations = configurations();
        assert!(
            monitor
                .register(1, configurations.clone(), 48_000)
                .request()
        );
        assert!(
            monitor
                .register(2, configurations.clone(), 44_100)
                .request()
        );
        assert!(monitor.register(3, configurations, 96_000).request());

        assert_eq!(monitor.poll().unwrap().generation, 3);
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn events_increase_monotonically_across_configuration_and_rate_changes() {
        let monitor = RecoveryMonitor::default();
        let mut configurations = configurations();
        let mut previous = 0;
        for generation in 1..=8 {
            change_output_buffer(&mut configurations, 64 * generation as usize);
            let ticket = monitor.register(generation, configurations.clone(), 48_000);
            assert!(ticket.request());
            let event = monitor.poll().unwrap();
            assert_eq!(event.generation, previous + 1);
            assert!(event.available);
            previous = event.generation;
            assert!(!ticket.request());
        }
        assert!(monitor.register(9, configurations, 96_000).request());
        assert_eq!(monitor.poll().unwrap().generation, previous + 1);
    }

    #[test]
    fn concurrent_requests_share_one_budget_and_one_pending_slot() {
        use std::{sync::Barrier, thread};

        let monitor = RecoveryMonitor::default();
        let ticket = monitor.register(1, configurations(), 48_000);
        let barrier = Barrier::new(8);
        let accepted = thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    let ticket = ticket.clone();
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        ticket.request()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|accepted| *accepted)
                .count()
        });

        assert_eq!(accepted, 1);
        assert_eq!(monitor.poll().unwrap().generation, 1);
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn poll_returns_none_on_contention_without_consuming_pending_event() {
        use std::{sync::mpsc, thread, time::Duration};

        let monitor = RecoveryMonitor::default();
        assert!(monitor.register(1, configurations(), 48_000).request());
        let guard = monitor.state.lock().unwrap();
        let other_monitor = monitor.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || sender.send(other_monitor.poll()).unwrap());
        // The timeout is only a deadlock watchdog; lock ownership fixes the race.
        let result = receiver.recv_timeout(Duration::from_secs(2));
        drop(guard);
        worker.join().unwrap();

        assert!(matches!(result, Ok(None)));
        assert_eq!(monitor.poll().unwrap().generation, 1);
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn event_generation_exhaustion_never_wraps_or_reuses_a_generation() {
        let monitor = RecoveryMonitor::default();
        monitor.state.lock().unwrap().event_generation = u64::MAX - 1;
        let configurations = configurations();
        assert!(
            monitor
                .register(1, configurations.clone(), 48_000)
                .request()
        );
        assert_eq!(monitor.poll().unwrap().generation, u64::MAX);

        assert!(!monitor.register(2, configurations, 96_000).request());
        assert!(monitor.poll().is_none());
    }
}
