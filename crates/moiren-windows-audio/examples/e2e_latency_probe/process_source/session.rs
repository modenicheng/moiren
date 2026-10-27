//! Actual parent lifecycle with native process/clock boundaries injected.
use super::*;
use moiren_windows_audio::process_loopback::ProcessIdentity;

pub(super) struct Session<C: supervision::ChildOwner> {
    pub(super) child: C,
    pub(super) identity: ProcessIdentity,
    pub(super) config: Config,
    pub(super) schedule: super::super::source::SourceSchedule,
    pub(super) directory: PathBuf,
    pub(super) progress: Progress,
    pub(super) activated: bool,
    pub(super) stop_requested: bool,
    pub(super) finished: bool,
    pub(super) elapsed: Duration,
    pub(super) failure: Option<String>,
}

impl<C: supervision::ChildOwner> Session<C> {
    pub(super) fn await_ready(&mut self, clock: &mut impl supervision::Clock) -> Result<()> {
        let timeout = READY_GRACE.min(Duration::from_secs(self.config.seconds));
        loop {
            self.elapsed = clock.elapsed();
            self.check()?;
            if self.progress.prepared {
                return Ok(());
            }
            ensure!(clock.elapsed() < timeout, "source ready timeout");
            clock.poll();
        }
    }
    pub(crate) fn activate(&mut self) -> Result<()> {
        ensure!(
            !self.activated && !self.stop_requested && !self.finished,
            "source activation already attempted or stopping"
        );
        self.check()?;
        ensure!(self.progress.prepared, "source not prepared");
        token(&self.directory, "activate")?;
        self.activated = true;
        Ok(())
    }

    fn poll_receipts(&mut self) -> Result<()> {
        let mut failure = None;
        for phase in [Phase::Ready, Phase::Running, Phase::Submitted] {
            let present = match phase {
                Phase::Ready => self.progress.ready.is_some(),
                Phase::Running => self.progress.running_receipt.is_some(),
                Phase::Submitted => self.progress.submitted_receipt.is_some(),
                Phase::Final => false,
            };
            let path = self.directory.join(phase.file());
            if present || !path.try_exists()? {
                continue;
            }
            let result = read_receipt(&path).and_then(|receipt| {
                self.progress.observe(
                    receipt,
                    phase,
                    self.activated,
                    &Identity::from(&self.identity),
                    &self.config,
                    &self.schedule,
                )
            });
            if let Err(error) = result {
                append(&mut failure, &format!("{}: {error:#}", phase.file()));
            }
        }
        ensure!(failure.is_none(), "{}", failure.unwrap_or_default());
        Ok(())
    }

    pub(crate) fn check(&mut self) -> Result<Progress> {
        let result = (|| -> Result<Progress> {
            ensure!(!self.finished, "source already joined");
            self.poll_receipts()?;
            ensure!(
                self.child.try_exit()?.is_none(),
                "source child exited before requested stop"
            );
            check_lifetime(self.elapsed, self.config.seconds)?;
            let current = self.child.inspect()?;
            ensure!(
                current.pid == self.child.id() && current.same_process(&self.identity),
                "source identity changed"
            );
            Ok(self.progress.clone())
        })();
        if let Err(error) = &result {
            append(&mut self.failure, &format!("{error:#}"));
        }
        result
    }

    pub(crate) fn request_stop(&mut self) -> Result<()> {
        let result = self.request_stop_once();
        if let Err(error) = &result {
            append(&mut self.failure, &format!("stop token: {error:#}"));
        }
        result
    }

    fn request_stop_once(&mut self) -> Result<()> {
        if self.stop_requested {
            return Ok(());
        }
        if self.child.try_exit()?.is_some() {
            append(&mut self.failure, "source exited before stop request");
        }
        token(&self.directory, "stop")?;
        self.stop_requested = true;
        Ok(())
    }

    pub(super) fn finish(&mut self, clock: &mut impl supervision::Clock) -> Outcome {
        let mut outcome = Outcome {
            source_endpoint: Some(self.config.source_endpoint.clone()),
            seconds: Some(self.config.seconds),
            schedule: Some(self.schedule.clone()),
            child_pid: Some(self.child.id()),
            pinned_identity: Some(Identity::from(&self.identity)),
            progress: self.progress.clone(),
            final_receipt: None,
            child_exit_code: None,
            forced_termination: false,
            joined: false,
            valid: false,
            failure: self.failure.clone(),
        };
        if self.finished {
            append(&mut outcome.failure, "source session already finished");
            return outcome;
        }
        if let Err(error) = self.request_stop() {
            append(&mut outcome.failure, &format!("stop token: {error:#}"));
        }
        supervision::retire(&mut self.child, clock, &mut outcome);
        if let Err(error) = self.poll_receipts() {
            append(&mut outcome.failure, &format!("receipts: {error:#}"));
        }
        outcome.progress = self.progress.clone();
        match read_receipt(&self.directory.join(Phase::Final.file())) {
            Ok(receipt) => {
                if let Err(error) = validate_receipt(
                    &receipt,
                    Phase::Final,
                    &Identity::from(&self.identity),
                    &self.config,
                    &self.schedule,
                ) {
                    append(&mut outcome.failure, &format!("final receipt: {error:#}"));
                }
                outcome.final_receipt = Some(receipt);
            }
            Err(error) => append(
                &mut outcome.failure,
                &format!("final receipt missing/invalid: {error:#}"),
            ),
        }
        append_option(&mut outcome.failure, self.failure.as_deref());
        finalize(&mut outcome);
        self.finished = outcome.joined;
        self.failure = outcome.failure.clone();
        outcome
    }
}
