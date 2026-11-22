//! Read-only process handles retained through the admitted CLI's teardown.
use super::*;
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    time::Instant,
};
use windows::Win32::{
    Foundation::{FILETIME, HANDLE, WAIT_TIMEOUT},
    Security::{GetTokenInformation, TOKEN_QUERY, TokenSessionId},
    System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    },
};

const FILETIME_TO_DATETIME_TICKS: u64 = 504_911_232_000_000_000;

fn creation_ticks(process: HANDLE) -> Result<u64> {
    let (mut creation, mut exit, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // All four outputs are live locals, and the caller retains this handle.
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user)? };
    ((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
        .checked_add(FILETIME_TO_DATETIME_TICKS)
        .filter(|ticks| *ticks <= MAX_TICKS)
        .context("process creation cannot be represented as DateTime UTC ticks")
}

fn open_owner(pid: u32, ticks: u64) -> Result<OwnedHandle> {
    // No inheritance, mutation, termination, suspend or debug rights requested.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )?
    };
    // OpenProcess returned an owned real handle, transferred exactly once.
    let owner = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    ensure!(
        creation_ticks(HANDLE(owner.as_raw_handle()))? == ticks,
        "managed owner creation differs"
    );
    Ok(owner)
}

fn session_id() -> Result<u32> {
    let mut token = HANDLE::default();
    // TokenSessionId is read from this process's token, not a PID lookup that
    // could silently bind a different process after an identity race.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let mut session = 0u32;
    let mut returned = 0;
    unsafe {
        GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenSessionId,
            Some((&mut session as *mut u32).cast()),
            size_of::<u32>() as u32,
            &mut returned,
        )?;
    }
    ensure!(
        returned == size_of::<u32>() as u32,
        "invalid process session size"
    );
    Ok(session)
}

pub(super) struct Processes {
    controller: OwnedHandle,
    worker: OwnedHandle,
    created: Instant,
    probe_ticks: u64,
    session: u32,
}

impl Processes {
    pub(super) fn open(config: &Config) -> Result<Self> {
        let created = Instant::now();
        ensure!(
            config.controller_pid != std::process::id() && config.worker_pid != std::process::id(),
            "managed probe cannot own itself"
        );
        let controller = open_owner(config.controller_pid, config.controller_ticks)?;
        let worker = open_owner(config.worker_pid, config.worker_ticks)?;
        let probe_ticks = creation_ticks(unsafe { GetCurrentProcess() })?;
        ensure!(
            config.controller_ticks <= probe_ticks && config.worker_ticks <= probe_ticks,
            "managed owners must predate probe"
        );
        Ok(Self {
            controller,
            worker,
            created,
            probe_ticks,
            session: session_id()?,
        })
    }
}

impl Boundary for Processes {
    fn receipt(&self, config: &Config) -> Receipt {
        Receipt {
            schema_version: 1,
            nonce: config.nonce.clone(),
            probe_pid: std::process::id(),
            probe_creation_utc_ticks: self.probe_ticks,
            controller_pid: config.controller_pid,
            controller_creation_utc_ticks: config.controller_ticks,
            worker_pid: config.worker_pid,
            worker_creation_utc_ticks: config.worker_ticks,
            session_id: self.session,
        }
    }

    fn owners_alive(&self) -> Result<()> {
        for owner in [&self.controller, &self.worker] {
            // Zero-time wait only. Only WAIT_TIMEOUT means still alive; neither
            // a signalled process nor a failed wait authorizes native entry.
            ensure!(
                unsafe { WaitForSingleObject(HANDLE(owner.as_raw_handle()), 0) } == WAIT_TIMEOUT,
                "managed owner exited or cannot be observed"
            );
        }
        Ok(())
    }

    fn elapsed(&self) -> Duration {
        self.created.elapsed()
    }

    fn poll(&mut self) {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
