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

pub(super) fn run(collection: Result<()>, cleanup: &mut impl Cleanup) -> Result<()> {
    let mut result = collection;
    // Observe while still admitted, then declare stop intent before native
    // Stop/gate IO can cancel outstanding PCM. A failed stage must not prevent
    // later owners from being stopped and drained, including during unwind.
    for (stage, step) in [
        "final observation",
        "admission invalidation",
        "source stop",
        "gate close",
        "host shutdown",
        "endpoint retirement",
    ]
    .into_iter()
    .zip([
        Cleanup::observe_final,
        Cleanup::invalidate_admission,
        Cleanup::stop_source,
        Cleanup::close_gate,
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

    #[test]
    fn soak_cleanup_attempts_every_stage_and_keeps_collection_and_all_cleanup_errors() {
        let mut cleanup = FailingCleanup::default();
        let error = run(Err(anyhow::anyhow!("collection disk full")), &mut cleanup).unwrap_err();
        assert_eq!(
            cleanup.attempted,
            [
                "final observation",
                "admission invalidation",
                "source stop",
                "gate close",
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
        let mut cleanup = PanickingCleanup(FailingCleanup::default());
        let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(Err(anyhow::anyhow!("collection failure")), &mut cleanup)
        }));
        assert!(
            attempted.is_ok(),
            "one cleanup panic must not skip its peers"
        );
        assert_eq!(
            cleanup.0.attempted,
            [
                "final observation",
                "admission invalidation",
                "source stop",
                "gate close",
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
            "gate close",
            "host shutdown",
            "endpoint retirement",
        ] {
            assert!(text.contains(stage), "lost {stage}: {text}");
        }
    }
}
