//! Best-effort non-RT retirement: no fallible stage suppresses later cleanup.

use anyhow::Result;

pub(super) trait Cleanup {
    fn observe_final(&mut self) -> Result<()>;
    fn stop_source(&mut self) -> Result<()>;
    fn close_gate(&mut self) -> Result<()>;
    fn shutdown_host(&mut self) -> Result<()>;
    fn invalidate_admission(&mut self) -> Result<()>;
    fn retire_endpoint(&mut self) -> Result<()>;
}

/// Only non-collection VAD -> physical output uses this early-close policy.
/// Process owners and supervised VAD collection keep their own stop ordering.
pub(super) trait PhysicalVadCleanup: Cleanup {
    fn request_source_stop(&mut self) -> Result<()>;
    fn close_gate_before_stop(&mut self) -> Result<()>;
}

pub(super) fn run_physical_vad(
    collection: Result<()>,
    cleanup: &mut impl PhysicalVadCleanup,
) -> Result<()> {
    let mut result = collection;
    // Close the original source gate while native identity is stable, before
    // revocation starts worker cleanup or native Stop advances its epoch.
    // Broadcast source stop before any join. Failure/panic never skips peers.
    for (stage, step) in [
        "final observation",
        "gate close",
        "admission invalidation",
        "source stop request",
        "source stop",
        "host shutdown",
        "endpoint retirement",
    ]
    .into_iter()
    .zip([
        Cleanup::observe_final,
        PhysicalVadCleanup::close_gate_before_stop,
        Cleanup::invalidate_admission,
        PhysicalVadCleanup::request_source_stop,
        Cleanup::stop_source,
        Cleanup::shutdown_host,
        Cleanup::retire_endpoint,
    ]) {
        let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(cleanup)))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("{stage}: cleanup stage panicked")));
        result = combine(result, attempted);
    }
    result
}

pub(super) fn combine(first: Result<()>, next: Result<()>) -> Result<()> {
    match (first, next) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(first), Err(next)) => Err(anyhow::anyhow!("{first:#}; {next:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_soak_closes_gate_before_revocation_and_native_stop() {
        #[derive(Default)]
        struct PhysicalCleanup {
            closed: bool,
            stop_requested: bool,
            attempted: Vec<&'static str>,
        }
        impl Cleanup for PhysicalCleanup {
            fn observe_final(&mut self) -> Result<()> {
                self.attempted.push("observe");
                assert!(
                    !self.closed,
                    "retain live evidence before closing admission"
                );
                Ok(())
            }
            fn close_gate(&mut self) -> Result<()> {
                self.attempted.push("close");
                self.closed = true;
                Ok(())
            }
            fn invalidate_admission(&mut self) -> Result<()> {
                self.attempted.push("invalidate");
                anyhow::ensure!(
                    self.closed,
                    "worker revocation began while native gate remained open"
                );
                Ok(())
            }
            fn stop_source(&mut self) -> Result<()> {
                self.attempted.push("stop and join source");
                // Pause/reset advances the epoch before native release. It
                // must not race a worker's first close of an open gate.
                anyhow::ensure!(self.closed, "native Stop began before gate close");
                assert!(self.stop_requested, "broadcast must precede source join");
                Ok(())
            }
            fn shutdown_host(&mut self) -> Result<()> {
                self.attempted.push("join host");
                Ok(())
            }
            fn retire_endpoint(&mut self) -> Result<()> {
                self.attempted.push("retire");
                Ok(())
            }
        }
        impl PhysicalVadCleanup for PhysicalCleanup {
            fn request_source_stop(&mut self) -> Result<()> {
                self.attempted.push("request source stop");
                self.stop_requested = true;
                Ok(())
            }
            fn close_gate_before_stop(&mut self) -> Result<()> {
                self.close_gate()
            }
        }
        let mut cleanup = PhysicalCleanup::default();
        run_physical_vad(Ok(()), &mut cleanup).unwrap();
        assert_eq!(
            cleanup.attempted,
            [
                "observe",
                "close",
                "invalidate",
                "request source stop",
                "stop and join source",
                "join host",
                "retire"
            ]
        );
    }

    #[derive(Default)]
    struct FailingCleanup {
        attempted: Vec<&'static str>,
    }

    impl FailingCleanup {
        fn fail(&mut self, stage: &'static str) -> Result<()> {
            self.attempted.push(stage);
            anyhow::bail!("injected {stage} failure")
        }
    }

    impl Cleanup for FailingCleanup {
        fn observe_final(&mut self) -> Result<()> {
            self.fail("final observation")
        }
        fn stop_source(&mut self) -> Result<()> {
            self.fail("source stop")
        }
        fn close_gate(&mut self) -> Result<()> {
            self.fail("gate close")
        }
        fn shutdown_host(&mut self) -> Result<()> {
            self.fail("host shutdown")
        }
        fn invalidate_admission(&mut self) -> Result<()> {
            self.fail("admission invalidation")
        }
        fn retire_endpoint(&mut self) -> Result<()> {
            self.fail("endpoint retirement")
        }
    }

    impl PhysicalVadCleanup for FailingCleanup {
        fn request_source_stop(&mut self) -> Result<()> {
            self.fail("source stop request")
        }
        fn close_gate_before_stop(&mut self) -> Result<()> {
            self.close_gate()
        }
    }

    #[test]
    fn soak_cleanup_attempts_every_stage_and_keeps_collection_and_all_cleanup_errors() {
        let mut cleanup = FailingCleanup::default();
        let error = run_physical_vad(Err(anyhow::anyhow!("collection disk full")), &mut cleanup)
            .unwrap_err();
        assert_eq!(
            cleanup.attempted,
            [
                "final observation",
                "gate close",
                "admission invalidation",
                "source stop request",
                "source stop",
                "host shutdown",
                "endpoint retirement"
            ]
        );
        let text = format!("{error:#}");
        assert!(text.contains("collection disk full"));
        for stage in &cleanup.attempted {
            assert!(text.contains(stage), "lost {stage}: {text}");
        }
    }

    #[test]
    fn retirement_intent_soak_invalidation_panic_still_stops_and_drains() {
        struct PanickingCleanup(FailingCleanup);
        impl Cleanup for PanickingCleanup {
            fn observe_final(&mut self) -> Result<()> {
                self.0.observe_final()
            }
            fn invalidate_admission(&mut self) -> Result<()> {
                self.0.attempted.push("admission invalidation");
                panic!("injected invalidation panic");
            }
            fn stop_source(&mut self) -> Result<()> {
                self.0.stop_source()
            }
            fn close_gate(&mut self) -> Result<()> {
                self.0.close_gate()
            }
            fn shutdown_host(&mut self) -> Result<()> {
                self.0.shutdown_host()
            }
            fn retire_endpoint(&mut self) -> Result<()> {
                self.0.retire_endpoint()
            }
        }
        impl PhysicalVadCleanup for PanickingCleanup {
            fn request_source_stop(&mut self) -> Result<()> {
                self.0.request_source_stop()
            }
            fn close_gate_before_stop(&mut self) -> Result<()> {
                self.close_gate()
            }
        }
        let mut cleanup = PanickingCleanup(FailingCleanup::default());
        let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_physical_vad(Err(anyhow::anyhow!("collection failure")), &mut cleanup)
        }));
        assert!(
            attempted.is_ok(),
            "one cleanup panic must not skip its peers"
        );
        assert_eq!(
            cleanup.0.attempted,
            [
                "final observation",
                "gate close",
                "admission invalidation",
                "source stop request",
                "source stop",
                "host shutdown",
                "endpoint retirement"
            ]
        );
        let text = format!("{:#}", attempted.unwrap().unwrap_err());
        assert!(text.contains("collection failure"));
        assert!(text.contains("admission invalidation"));
        assert!(text.contains("panicked"));
        for stage in [
            "source stop",
            "source stop request",
            "gate close",
            "host shutdown",
            "endpoint retirement",
        ] {
            assert!(text.contains(stage), "lost {stage}: {text}");
        }
    }

    #[test]
    fn physical_soak_close_error_or_panic_still_broadcasts_and_joins_every_owner() {
        struct CloseFailure {
            cleanup: FailingCleanup,
            panic: bool,
        }
        impl Cleanup for CloseFailure {
            fn observe_final(&mut self) -> Result<()> {
                self.cleanup.observe_final()
            }
            fn close_gate(&mut self) -> Result<()> {
                self.cleanup.attempted.push("gate close");
                if self.panic {
                    panic!("stale gate close panic");
                }
                anyhow::bail!("stale QUERY -> SET_GATE: Cancelled")
            }
            fn invalidate_admission(&mut self) -> Result<()> {
                self.cleanup.invalidate_admission()
            }
            fn stop_source(&mut self) -> Result<()> {
                self.cleanup.stop_source()
            }
            fn shutdown_host(&mut self) -> Result<()> {
                self.cleanup.shutdown_host()
            }
            fn retire_endpoint(&mut self) -> Result<()> {
                self.cleanup.retire_endpoint()
            }
        }
        impl PhysicalVadCleanup for CloseFailure {
            fn request_source_stop(&mut self) -> Result<()> {
                self.cleanup.request_source_stop()
            }
            fn close_gate_before_stop(&mut self) -> Result<()> {
                self.close_gate()
            }
        }
        for panic in [false, true] {
            let mut cleanup = CloseFailure {
                cleanup: FailingCleanup::default(),
                panic,
            };
            let error = run_physical_vad(
                Err(anyhow::anyhow!("original collection error")),
                &mut cleanup,
            )
            .unwrap_err();
            assert_eq!(
                cleanup.cleanup.attempted,
                [
                    "final observation",
                    "gate close",
                    "admission invalidation",
                    "source stop request",
                    "source stop",
                    "host shutdown",
                    "endpoint retirement"
                ]
            );
            let text = format!("{error:#}");
            assert!(text.contains("original collection error"));
            assert!(text.contains(if panic { "panicked" } else { "Cancelled" }));
            for stage in [
                "source stop request",
                "source stop",
                "host shutdown",
                "endpoint retirement",
            ] {
                assert!(text.contains(stage), "lost {stage}: {text}");
            }
        }
    }
}
