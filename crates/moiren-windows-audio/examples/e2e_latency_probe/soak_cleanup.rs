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
    for step in [
        Cleanup::observe_final,
        Cleanup::stop_source,
        Cleanup::close_gate,
        Cleanup::shutdown_host,
        Cleanup::invalidate_admission,
        Cleanup::retire_endpoint,
    ] {
        result = combine(result, step(cleanup));
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
                "source stop",
                "gate close",
                "host shutdown",
                "admission invalidation",
                "endpoint retirement"
            ]
        );
        let text = format!("{error:#}");
        assert!(text.contains("collection disk full"));
        for stage in &cleanup.attempted {
            assert!(text.contains(stage), "lost {stage}: {text}");
        }
    }
}
