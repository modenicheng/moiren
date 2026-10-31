use super::*;
use crate::live::vad;
use moiren_windows_audio::process_loopback::{ProcessIdentity, inspect_process};
use std::{
    os::windows::io::AsRawHandle,
    process::{Child, Command, Stdio},
    time::Instant,
};
use windows::Win32::{
    Foundation::{FILETIME, HANDLE},
    System::Threading::{GetCurrentProcess, GetProcessTimes},
};

fn creation(handle: HANDLE) -> Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)? };
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn current_identity() -> Result<Identity> {
    Ok(Identity {
        pid: std::process::id(),
        creation_time_100ns: creation(unsafe { GetCurrentProcess() })?,
        executable_name: std::env::current_exe()?
            .file_name()
            .context("source executable basename missing")?
            .to_string_lossy()
            .into(),
    })
}

impl supervision::ChildOwner for Child {
    fn id(&self) -> u32 {
        Child::id(self)
    }
    fn inspect(&self) -> Result<ProcessIdentity> {
        let identity = inspect_process(self.id())?;
        ensure!(
            identity.pid == self.id()
                && identity.creation_time_100ns == creation(HANDLE(self.as_raw_handle()))?,
            "source identity changed from retained child"
        );
        Ok(identity)
    }
    fn try_exit(&mut self) -> Result<Option<Option<i32>>> {
        Ok(self.try_wait()?.map(|status| status.code()))
    }
    fn kill(&mut self) -> Result<()> {
        Ok(Child::kill(self)?)
    }
}

struct RetirementClock(Instant);
impl supervision::Clock for RetirementClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn poll(&mut self) {
        std::thread::sleep(POLL);
    }
}

pub(crate) struct ProcessSourceSession {
    inner: super::session::Session<Child>,
    created: Instant,
}

impl ProcessSourceSession {
    #[expect(dead_code, reason = "Section B parent constructor")]
    pub(crate) fn prepare(config: Config, run_directory: &Path) -> Result<Self> {
        Self::prepare_with_abort(config, run_directory, || {}, &mut None)
    }
    pub(crate) fn prepare_with_abort(
        config: Config,
        run_directory: &Path,
        before_join: impl FnMut(),
        partial: &mut Option<Outcome>,
    ) -> Result<Self> {
        *partial = None;
        let source_endpoint = config.source_endpoint.clone();
        let seconds = config.seconds;
        let validation = config.validation;
        let result = Self::prepare_owned(config, run_directory, before_join, partial);
        if let Err(error) = &result
            && partial.is_none()
        {
            *partial = Some(Outcome {
                source_endpoint: Some(source_endpoint),
                seconds: Some(seconds),
                validation,
                failure: Some(format!(
                    "source preparation before child ownership: {error:#}"
                )),
                ..Outcome::default()
            });
        }
        result
    }

    fn prepare_owned(
        config: Config,
        run_directory: &Path,
        mut before_join: impl FnMut(),
        partial: &mut Option<Outcome>,
    ) -> Result<Self> {
        config.validate()?;
        let (bytes, _) = super::super::source::read_plan(
            &config.source_schedule,
            super::super::source::MAX_SOURCE_BYTES,
            config.seconds,
        )?;
        let schedule: super::super::source::SourceSchedule = serde_json::from_slice(&bytes)?;
        validate_receipt_budget(&config, &schedule)?;
        let directory = create_run_directory(run_directory)?;
        let source_schedule = directory.join("source-schedule.json");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&source_schedule)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        let created = Instant::now();
        let mut child = Command::new(std::env::current_exe()?)
            .args(["process-source-worker", "--source-endpoint"])
            .arg(&config.source_endpoint)
            .arg("--source-schedule")
            .arg(&source_schedule)
            .arg("--run-directory")
            .arg(&directory)
            .arg("--seconds")
            .arg(config.seconds.to_string())
            .arg("--validation-mode")
            .arg(if config.validation.is_correctness() {
                "correctness"
            } else {
                "latency-sensitive"
            })
            .arg("--jitter-tolerance-ms")
            .arg(config.validation.jitter_tolerance_ms.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let identity = (|| -> Result<ProcessIdentity> {
            let pinned = creation(HANDLE(child.as_raw_handle()))?;
            let identity = inspect_process(child.id())?;
            ensure!(
                identity.pid == child.id() && identity.creation_time_100ns == pinned && pinned != 0,
                "source producer identity recycled"
            );
            Ok(identity)
        })();
        let identity = match identity {
            Ok(value) => value,
            Err(error) => {
                let mut outcome = supervision::abort_preparation(&mut before_join, || {
                    let mut outcome = Outcome {
                        source_endpoint: Some(config.source_endpoint.clone()),
                        seconds: Some(config.seconds),
                        validation: config.validation,
                        schedule: Some(schedule.clone()),
                        child_pid: Some(child.id()),
                        ..Outcome::default()
                    };
                    append(
                        &mut outcome.failure,
                        &format!("source identity pin: {error:#}"),
                    );
                    if let Err(stop) = token(&directory, "stop") {
                        append(
                            &mut outcome.failure,
                            &format!("source stop token: {stop:#}"),
                        );
                    }
                    supervision::retire(
                        &mut child,
                        &mut RetirementClock(Instant::now()),
                        &mut outcome,
                    );
                    // No validated identity exists. Retain observations without accepting them.
                    for phase in [Phase::Ready, Phase::Running, Phase::Submitted, Phase::Final] {
                        match read_receipt(&directory.join(phase.file())) {
                            Ok(receipt) => match phase {
                                Phase::Ready => outcome.progress.ready = Some(receipt),
                                Phase::Running => outcome.progress.running_receipt = Some(receipt),
                                Phase::Submitted => {
                                    outcome.progress.submitted_receipt = Some(receipt)
                                }
                                Phase::Final => outcome.final_receipt = Some(receipt),
                            },
                            Err(read) => {
                                append(&mut outcome.failure, &format!("{}: {read:#}", phase.file()))
                            }
                        }
                    }
                    outcome
                });
                finalize(&mut outcome);
                *partial = Some(outcome);
                return Err(error);
            }
        };
        let mut session = Self {
            inner: super::session::Session {
                child,
                identity,
                config,
                schedule,
                directory,
                progress: Progress::default(),
                activated: false,
                stop_requested: false,
                finished: false,
                elapsed: Duration::ZERO,
                failure: None,
            },
            created,
        };
        let ready = session.inner.await_ready(&mut RetirementClock(created));
        if let Err(error) = ready {
            append(
                &mut session.inner.failure,
                &format!("source preparation: {error:#}"),
            );
            *partial = Some(supervision::abort_preparation(&mut before_join, || {
                session.finish()
            }));
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) fn identity(&self) -> &ProcessIdentity {
        &self.inner.identity
    }
    pub(crate) fn activate(&mut self) -> Result<()> {
        self.inner.elapsed = self.created.elapsed();
        self.inner.activate()
    }
    pub(crate) fn check(&mut self) -> Result<Progress> {
        self.inner.elapsed = self.created.elapsed();
        self.inner.check()
    }
    pub(crate) fn request_stop(&mut self) -> Result<()> {
        self.inner.request_stop()
    }
    pub(crate) fn finish(&mut self) -> Outcome {
        self.inner.finish(&mut RetirementClock(Instant::now()))
    }
}
impl Drop for ProcessSourceSession {
    fn drop(&mut self) {
        if !self.inner.finished {
            let outcome = self.finish();
            if !outcome.joined {
                eprintln!(
                    "process source PID {} exit unconfirmed at owner drop: {}",
                    self.inner.child.id(),
                    outcome
                        .failure
                        .as_deref()
                        .unwrap_or("unknown cleanup failure")
                );
            }
        }
    }
}
pub(super) fn run(config: Config, directory: PathBuf, witness: bool) -> Result<i32> {
    let created = Instant::now();
    config.validate()?;
    regular_path(&directory)?;
    ensure!(directory.is_dir(), "source run directory must be directory");
    // The parent creates the directory; the child claims it exactly once.
    token(&directory, "worker-owned")?;
    for name in [
        "ready.json",
        "running.json",
        "submitted.json",
        "final.json",
        "activate",
    ] {
        ensure!(
            !directory.join(name).try_exists()?,
            "source IPC path collision"
        );
    }
    let identity = current_identity()?;
    let (bytes, plan) = super::super::source::read_plan(
        &config.source_schedule,
        super::super::source::MAX_SOURCE_BYTES,
        config.seconds,
    )?;
    let schedule: super::super::source::SourceSchedule = serde_json::from_slice(&bytes)?;
    validate_receipt_budget(&config, &schedule)?;
    let mut failure = None;
    let mut stop_requested = false;
    let mut session = None;
    let mut partial = None;
    let result = (|| -> Result<()> {
        session = Some(vad::PlaybackSession::prepare_shared(
            config.source_endpoint.clone(),
            plan,
            witness,
            &mut partial,
        )?);
        let owner = session.as_mut().unwrap();
        let receipt = |phase, playback, stop_requested, failure| Receipt {
            schema_version: 1,
            phase,
            identity: identity.clone(),
            source_endpoint: config.source_endpoint.clone(),
            seconds: config.seconds,
            validation: config.validation,
            schedule: schedule.clone(),
            playback,
            stop_requested,
            failure,
        };
        publish(
            &directory,
            Phase::Ready.file(),
            &receipt(
                Phase::Ready,
                serde_json::to_value(&owner.info)?,
                false,
                None,
            ),
        )?;
        let mut activated = false;
        let mut running = false;
        let mut submitted = false;
        loop {
            check_lifetime(created.elapsed(), config.seconds)?;
            if has_token(&directory, "stop")? {
                stop_requested = true;
                break;
            }
            if !activated && has_token(&directory, "activate")? {
                owner.activate()?;
                activated = true;
            }
            if let Some(progress) = owner.check()? {
                validate_event_timeouts(progress.event_timeouts, config.validation)?;
                if progress.started && !running {
                    ensure!(activated, "source started before activation");
                    publish(
                        &directory,
                        Phase::Running.file(),
                        &receipt(
                            Phase::Running,
                            serde_json::to_value(&progress)?,
                            false,
                            None,
                        ),
                    )?;
                    running = true;
                }
                if progress.source.schedule_submitted_complete && !submitted {
                    ensure!(running, "source submitted before Start");
                    publish(
                        &directory,
                        Phase::Submitted.file(),
                        &receipt(
                            Phase::Submitted,
                            serde_json::to_value(&progress)?,
                            false,
                            None,
                        ),
                    )?;
                    submitted = true;
                }
            }
            std::thread::sleep(POLL);
        }
        Ok(())
    })();
    if let Err(error) = result {
        append(&mut failure, &format!("{error:#}"));
    }
    let playback = if let Some(mut owner) = session {
        owner.request_stop();
        match owner.finish() {
            Ok(outcome) => {
                append_option(&mut failure, outcome.failure.as_deref());
                if let Err(error) =
                    validate_event_timeouts(outcome.event_timeouts, config.validation)
                {
                    append(&mut failure, "final source event timeout");
                    append(&mut failure, &error.to_string());
                }
                serde_json::to_value(outcome)?
            }
            Err(error) => {
                append(&mut failure, &format!("source finish: {error:#}"));
                Value::Null
            }
        }
    } else if let Some(outcome) = partial {
        append_option(&mut failure, outcome.failure.as_deref());
        serde_json::to_value(outcome)?
    } else {
        Value::Null
    };
    let receipt = Receipt {
        schema_version: 1,
        phase: Phase::Final,
        identity,
        source_endpoint: config.source_endpoint,
        seconds: config.seconds,
        validation: config.validation,
        schedule,
        playback,
        stop_requested,
        failure,
    };
    publish(&directory, Phase::Final.file(), &receipt)?;
    Ok(i32::from(receipt.failure.is_some()))
}
