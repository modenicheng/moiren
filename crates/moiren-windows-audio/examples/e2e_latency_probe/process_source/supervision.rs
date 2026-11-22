//! Control-only child retirement; the clock/OS seams keep tests hardware-free.
use super::*;

pub(super) trait ChildOwner {
    fn id(&self) -> u32;
    fn inspect(&self) -> Result<moiren_windows_audio::process_loopback::ProcessIdentity>;
    fn try_exit(&mut self) -> Result<Option<Option<i32>>>;
    fn kill(&mut self) -> Result<()>;
}

pub(super) trait Clock {
    fn elapsed(&self) -> Duration;
    fn poll(&mut self);
}

pub(super) fn retire(child: &mut impl ChildOwner, clock: &mut impl Clock, outcome: &mut Outcome) {
    loop {
        match child.try_exit() {
            Ok(Some(code)) => {
                outcome.child_exit_code = code;
                outcome.joined = true;
                return;
            }
            Err(error) => {
                append(&mut outcome.failure, &format!("child wait: {error:#}"));
                break;
            }
            Ok(None) if clock.elapsed() < STOP_GRACE => clock.poll(),
            Ok(None) => {
                append(&mut outcome.failure, "source stop grace expired");
                break;
            }
        }
    }
    outcome.forced_termination = true;
    if let Err(error) = child.kill() {
        append(&mut outcome.failure, &format!("source kill: {error:#}"));
    }
    let confirmation_started = clock.elapsed();
    loop {
        match child.try_exit() {
            Ok(Some(code)) => {
                outcome.child_exit_code = code;
                outcome.joined = true;
                return;
            }
            Err(error) => {
                append(
                    &mut outcome.failure,
                    &format!("source exit confirmation: {error:#}"),
                );
                break;
            }
            Ok(None) if clock.elapsed().saturating_sub(confirmation_started) < STOP_GRACE => {
                clock.poll()
            }
            Ok(None) => break,
        }
    }
    append(
        &mut outcome.failure,
        "source termination unconfirmed; no join observed",
    );
}

pub(super) fn abort_preparation(
    before_join: &mut impl FnMut(),
    cleanup: impl FnOnce() -> Outcome,
) -> Outcome {
    let mut failure = None;
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(before_join)).is_err() {
        append(&mut failure, "source preparation abort callback panicked");
    }
    let mut outcome = cleanup();
    append_option(&mut outcome.failure, failure.as_deref());
    outcome.valid = false;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Child<'a> {
        events: &'a RefCell<Vec<&'static str>>,
        exited: bool,
        fail_wait: bool,
        kill_error: bool,
        unconfirmed: bool,
    }
    impl ChildOwner for Child<'_> {
        fn id(&self) -> u32 {
            17
        }
        fn inspect(&self) -> Result<moiren_windows_audio::process_loopback::ProcessIdentity> {
            Ok(moiren_windows_audio::process_loopback::ProcessIdentity {
                pid: 17,
                creation_time_100ns: 1234,
                executable_name: "probe.exe".into(),
            })
        }
        fn try_exit(&mut self) -> Result<Option<Option<i32>>> {
            if self.fail_wait {
                anyhow::bail!("wait failed");
            }
            Ok(self.exited.then_some(Some(7)))
        }
        fn kill(&mut self) -> Result<()> {
            self.events.borrow_mut().push("kill");
            if self.kill_error {
                anyhow::bail!("kill failed");
            }
            self.exited = !self.unconfirmed;
            self.fail_wait = false;
            Ok(())
        }
    }
    struct Time(Duration);
    impl Clock for Time {
        fn elapsed(&self) -> Duration {
            self.0
        }
        fn poll(&mut self) {
            self.0 += Duration::from_secs(1);
        }
    }

    #[test]
    fn owned_process_source_abort_hook_precedes_failed_prepare_kill_and_reap() {
        let events = RefCell::new(Vec::new());
        let mut child = Child {
            events: &events,
            exited: false,
            fail_wait: true,
            kill_error: false,
            unconfirmed: false,
        };
        let outcome = abort_preparation(&mut || events.borrow_mut().push("peer_stop"), || {
            let mut outcome = Outcome::default();
            retire(&mut child, &mut Time(Duration::ZERO), &mut outcome);
            outcome
        });
        assert_eq!(*events.borrow(), ["peer_stop", "kill"]);
        assert!(outcome.joined && outcome.forced_termination);
        assert_eq!(outcome.child_exit_code, Some(7));
        assert!(outcome.failure.as_deref().unwrap().contains("wait failed"));
    }

    #[test]
    fn owned_process_source_abort_callback_panic_cannot_skip_cleanup_or_partial() {
        let outcome = abort_preparation(&mut || panic!("peer stop panic"), || Outcome {
            child_pid: Some(91),
            child_exit_code: Some(3),
            joined: true,
            failure: Some("original preparation error".into()),
            ..Outcome::default()
        });
        assert_eq!(outcome.child_pid, Some(91));
        assert_eq!(outcome.child_exit_code, Some(3));
        assert!(outcome.joined && !outcome.valid);
        let failure = outcome.failure.unwrap();
        assert!(
            failure.contains("original preparation error") && failure.contains("callback panicked")
        );
    }

    #[test]
    fn owned_process_source_stop_grace_forces_reap_but_early_exit_is_not_killed() {
        for exited in [false, true] {
            let events = RefCell::new(Vec::new());
            let mut child = Child {
                events: &events,
                exited,
                fail_wait: false,
                kill_error: false,
                unconfirmed: false,
            };
            let mut time = Time(Duration::ZERO);
            let mut outcome = Outcome::default();
            retire(&mut child, &mut time, &mut outcome);
            assert!(outcome.joined);
            assert_eq!(outcome.forced_termination, !exited);
            assert_eq!(outcome.child_exit_code, Some(7));
            if exited {
                assert!(events.borrow().is_empty());
                assert_eq!(time.0, Duration::ZERO);
            } else {
                assert_eq!(*events.borrow(), ["kill"]);
                assert_eq!(time.0, STOP_GRACE);
            }
        }
    }

    #[test]
    fn owned_process_source_unconfirmed_termination_never_blocks_reap_or_claims_join() {
        for kill_error in [false, true] {
            let events = RefCell::new(Vec::new());
            let mut child = Child {
                events: &events,
                exited: false,
                fail_wait: false,
                kill_error,
                unconfirmed: true,
            };
            let mut time = Time(Duration::ZERO);
            let mut outcome = Outcome::default();
            retire(&mut child, &mut time, &mut outcome);
            assert!(!outcome.joined && outcome.forced_termination);
            assert_eq!(outcome.child_exit_code, None);
            assert_eq!(
                *events.borrow(),
                ["kill"],
                "must never enter unbounded reap"
            );
            assert!(time.0 <= Duration::from_secs(10));
            assert!(outcome.failure.as_deref().unwrap().contains("unconfirmed"));
        }
    }
}
