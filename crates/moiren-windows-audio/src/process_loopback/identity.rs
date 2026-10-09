//! Resolve and pin the selected process; no full path or process handle escapes.
use super::ProcessIdentity;
use crate::{
    capture::{CaptureError, wasapi::api},
    owner::{OwnedHandle, Process},
};
use std::collections::{BTreeMap, BTreeSet};
use windows::Win32::{
    Foundation::ERROR_NO_MORE_FILES,
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    },
};

pub fn inspect_process(pid: u32) -> Result<ProcessIdentity, CaptureError> {
    if pid == 0 {
        return Err(CaptureError::InvalidProcess);
    }
    let process = api("OpenProcess(loopback selection)", Process::open(pid))?;
    reject_feedback_target(&process.identity)?;
    if api("Process liveness(selection)", process.exited())? {
        return Err(CaptureError::TargetExited);
    }
    Ok(process.identity)
}

/// Accessible live candidates for explicit process-tree selection. Enumeration
/// never activates capture and omits inaccessible/exited processes, this host,
/// and its ancestors. Startup still revalidates PID plus creation time.
pub fn list_processes() -> Result<Vec<ProcessIdentity>, CaptureError> {
    let parents = process_parents()?;
    let host_pid = std::process::id();
    let ancestors = ancestor_ids(&parents, host_pid);
    let host = api("OpenProcess(host identity)", Process::open(host_pid))?;
    let mut choices = Vec::new();
    for pid in parents
        .keys()
        .copied()
        .filter(|&pid| pid != 0 && pid != host_pid)
    {
        let Ok(process) = Process::open(pid) else {
            continue;
        };
        if ancestors.contains(&pid)
            && process.identity.creation_time_100ns <= host.identity.creation_time_100ns
        {
            continue;
        }
        if matches!(process.exited(), Ok(false)) {
            choices.push(process.identity);
        }
    }
    Ok(choices)
}
pub(super) fn open_selected(selected: &ProcessIdentity) -> Result<Process, CaptureError> {
    let process = api("OpenProcess(loopback owner)", Process::open(selected.pid))?;
    if !selected.same_process(&process.identity) {
        return Err(CaptureError::ProcessIdentityChanged);
    }
    reject_feedback_target(selected)?;
    if api("Process liveness(owner)", process.exited())? {
        return Err(CaptureError::TargetExited);
    }
    Ok(process)
}
fn ancestor_ids(parents: &BTreeMap<u32, u32>, host: u32) -> BTreeSet<u32> {
    let mut ids = BTreeSet::new();
    let mut current = host;
    while current != 0 && ids.insert(current) {
        current = parents.get(&current).copied().unwrap_or(0);
    }
    ids
}
fn process_parents() -> Result<BTreeMap<u32, u32>, CaptureError> {
    let snapshot = OwnedHandle(api("Snapshot(process ancestry)", unsafe {
        CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    })?);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    api("Process32First(ancestry)", unsafe {
        Process32FirstW(snapshot.0, &mut entry)
    })?;
    let mut parents = BTreeMap::new();
    loop {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        match unsafe { Process32NextW(snapshot.0, &mut entry) } {
            Ok(()) => {}
            Err(error)
                if error.code() == windows::core::HRESULT::from_win32(ERROR_NO_MORE_FILES.0) =>
            {
                break;
            }
            Err(error) => {
                return Err(CaptureError::Api {
                    stage: "Process32Next(ancestry)",
                    hresult: error.code().0,
                });
            }
        }
    }
    Ok(parents)
}
fn reject_feedback_target(target: &ProcessIdentity) -> Result<(), CaptureError> {
    if target.pid == std::process::id() {
        return Err(CaptureError::FeedbackTarget);
    }
    if ancestor_ids(&process_parents()?, std::process::id()).contains(&target.pid) {
        // Parent PIDs can be recycled. Only a process created before this host
        // can be its ancestor; conservative rejection avoids output feedback.
        let host = api(
            "OpenProcess(host identity)",
            Process::open(std::process::id()),
        )?;
        if target.creation_time_100ns <= host.identity.creation_time_100ns {
            return Err(CaptureError::FeedbackTarget);
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ancestry_walk_is_bounded_and_includes_host_and_parents() {
        let parents = BTreeMap::from([(3, 2), (2, 1), (1, 2), (4, 1)]);
        assert_eq!(ancestor_ids(&parents, 3), BTreeSet::from([1, 2, 3]));
        assert!(!ancestor_ids(&parents, 3).contains(&4));
    }
    #[test]
    fn reused_identity_is_rejected_before_any_activation() {
        let mut identity = Process::open(std::process::id()).unwrap().identity;
        identity.creation_time_100ns += 1;
        assert!(matches!(
            open_selected(&identity),
            Err(CaptureError::ProcessIdentityChanged)
        ));
    }
    #[test]
    fn process_choices_are_pinned_and_exclude_the_host() {
        let choices = list_processes().unwrap();
        assert!(choices.iter().all(|choice| choice.pid != std::process::id()
            && choice.creation_time_100ns != 0
            && !choice.executable_name.is_empty()
            && !choice.executable_name.contains(['\\', '/'])));
        assert!(choices.windows(2).all(|pair| pair[0].pid < pair[1].pid));
    }
}
