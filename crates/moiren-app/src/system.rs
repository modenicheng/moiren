//! Host system performance samples for the status bar's performance panel:
//! total CPU from the kernel's time accounting and a one-call process table
//! with per-process CPU times and private working sets. The Win32 reads live
//! in [`win`]; every delta, ranking and formatting decision is pure code at
//! the bottom of this file, unit-tested with synthetic samples on every
//! platform the crate builds on.

use std::time::Duration;

#[cfg(windows)]
pub(crate) use win::{sample_cpu_totals, sample_processes};

#[cfg(not(windows))]
pub(crate) fn sample_cpu_totals() -> Option<CpuTotals> {
    None
}

#[cfg(not(windows))]
pub(crate) fn sample_processes() -> Vec<ProcessSample> {
    Vec::new()
}

/// One instant of the kernel's CPU accounting, in 100 ns units. Kernel time
/// includes the idle thread's time, which is what makes the busy share
/// computable without a second system call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CpuTotals {
    idle: u64,
    kernel: u64,
    user: u64,
}

impl CpuTotals {
    /// Synthetic accounting for the model tests; the real values only come
    /// from [`sample_cpu_totals`].
    #[cfg(test)]
    pub(crate) fn synthetic(idle: u64, kernel: u64, user: u64) -> Self {
        Self { idle, kernel, user }
    }
}

/// One process row as the kernel reports it: cumulative CPU time (100 ns)
/// and the private working set the Task Manager "Memory" column shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessSample {
    pub pid: u32,
    pub name: String,
    pub cpu_100ns: u64,
    pub mem_bytes: u64,
}

/// The machine's busy share (0..=100) between two [`CpuTotals`] samples.
/// `None` until two samples exist and time has actually passed, so the
/// readout stays blank instead of flapping on a zero interval.
pub(crate) fn total_cpu_percent(previous: &CpuTotals, current: &CpuTotals) -> Option<f32> {
    let kernel = current.kernel.saturating_sub(previous.kernel);
    let user = current.user.saturating_sub(previous.user);
    let idle = current.idle.saturating_sub(previous.idle);
    let total = kernel.checked_add(user)?;
    (total > 0).then(|| {
        let busy = total.saturating_sub(idle);
        (busy as f32 / total as f32 * 100.).clamp(0., 100.)
    })
}

/// One ranked process readout: CPU share of the whole machine over the
/// sampling interval, plus the memory snapshot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProcessUsage {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub mem_bytes: u64,
}

/// Joins the two process tables by PID, computes each process's CPU share
/// over `elapsed` against `cores` (a process can occupy several at once, so
/// the share is machine-relative like Task Manager's column), and returns
/// Moiren's own row first followed by the `keep` highest-CPU other
/// processes. Without a previous table every CPU share reads 0 — the first
/// sample after opening the panel shows memory immediately and honest
/// zeroes for CPU.
pub(crate) fn top_processes(
    previous: &[ProcessSample],
    current: &[ProcessSample],
    elapsed: Duration,
    cores: f32,
    own_pid: u32,
    keep: usize,
) -> Vec<ProcessUsage> {
    let capacity = elapsed.as_secs_f32() * 1e7 * cores.max(1.);
    let mut rows: Vec<ProcessUsage> = current
        .iter()
        .filter(|sample| sample.pid > 0)
        .map(|sample| {
            let delta = previous
                .iter()
                .find(|previous| previous.pid == sample.pid)
                .map_or(0, |previous| {
                    sample.cpu_100ns.saturating_sub(previous.cpu_100ns)
                });
            let cpu_percent = if capacity > 0. {
                (delta as f32 / capacity * 100.).clamp(0., 100.)
            } else {
                0.
            };
            ProcessUsage {
                pid: sample.pid,
                name: sample.name.clone(),
                cpu_percent,
                mem_bytes: sample.mem_bytes,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    let mut top = Vec::new();
    // Moiren's own row leads the table even when idle: the panel exists to
    // answer "what is the audio app itself doing".
    if let Some(index) = rows.iter().position(|row| row.pid == own_pid) {
        top.push(rows.remove(index));
    }
    top.extend(rows.into_iter().take(keep));
    top
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    //! All the unsafe in this module is Win32's own contract: documented
    //! kernel calls with caller-owned out-buffers, plus one
    //! `NtQuerySystemInformation` walk whose entry layout is declared in
    //! full below rather than read out of the crate's `Reserved1` bytes.

    use std::slice;

    use windows::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessInformation,
    };
    use windows::Win32::Foundation::{FILETIME, NTSTATUS, UNICODE_STRING};
    use windows::Win32::System::Threading::GetSystemTimes;

    use super::{CpuTotals, ProcessSample};

    /// `STATUS_INFO_LENGTH_MISMATCH`: the kernel's "buffer too small", the
    /// loop condition of every `NtQuerySystemInformation` walker.
    const STATUS_INFO_LENGTH_MISMATCH: NTSTATUS = NTSTATUS(0xC000_0004u32 as i32);
    /// The chain grows with the process count; a kernel asking for more than
    /// this is not telling the truth, and giving up beats looping forever.
    const MAX_BUFFER: usize = 64 * 1024 * 1024;
    const INITIAL_BUFFER: usize = 64 * 1024;

    pub(crate) fn sample_cpu_totals() -> Option<CpuTotals> {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: three out-parameters to a documented kernel call; it writes
        // exactly one FILETIME each.
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        Some(CpuTotals {
            idle: filetime_100ns(idle),
            kernel: filetime_100ns(kernel),
            user: filetime_100ns(user),
        })
    }

    fn filetime_100ns(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }

    /// One `SystemProcessInformation` record, declared in full. The crate's
    /// partial binding hides the time fields inside `Reserved1` bytes;
    /// reading reserved bytes by offset is worse than spelling out the
    /// stable Vista-and-later layout here (field-for-field: u32 pair, then
    /// seven 64-bit fields at their natural offsets, the `UNICODE_STRING`,
    /// then the id/handle/size block).
    #[repr(C)]
    struct ProcessEntry {
        next_entry_offset: u32,
        number_of_threads: u32,
        working_set_private_size: i64,
        hard_fault_count: u32,
        number_of_threads_high_watermark: u32,
        cycle_time: u64,
        create_time: i64,
        user_time: i64,
        kernel_time: i64,
        image_name: UNICODE_STRING,
        base_priority: i32,
        unique_process_id: usize,
        inherited_from_unique_process_id: usize,
        handle_count: u32,
        session_id: u32,
        peak_virtual_size: usize,
        virtual_size: usize,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_page_count: usize,
        reserved: [i64; 6],
    }

    pub(crate) fn sample_processes() -> Vec<ProcessSample> {
        let mut size = INITIAL_BUFFER;
        loop {
            // u64 elements put the buffer on the entries' 8-byte alignment;
            // casting a byte allocation up would make every reference UB.
            let mut buffer: Vec<u64> = vec![0; size.div_ceil(size_of::<u64>())];
            let capacity = buffer.len() * size_of::<u64>();
            let mut returned = 0u32;
            // SAFETY: `buffer` owns `capacity` writable, 8-aligned bytes for
            // the duration of the call; the kernel writes at most that many
            // and reports the needed size in `returned`.
            let status = unsafe {
                NtQuerySystemInformation(
                    SystemProcessInformation,
                    buffer.as_mut_ptr().cast(),
                    capacity as u32,
                    &mut returned,
                )
            };
            if status == STATUS_INFO_LENGTH_MISMATCH {
                let Some(grown) = size.checked_mul(2).filter(|grown| *grown <= MAX_BUFFER) else {
                    return Vec::new();
                };
                size = grown;
                continue;
            }
            if status != NTSTATUS(0) {
                return Vec::new();
            }
            return parse_entries(buffer.as_ptr().cast(), returned as usize);
        }
    }

    /// Walks the kernel's chain of process entries. `base` points at
    /// `len` aligned bytes written by the kernel in the call above; every
    /// hop follows the chain's own `next_entry_offset`.
    fn parse_entries(base: *const ProcessEntry, len: usize) -> Vec<ProcessSample> {
        let entry_size = size_of::<ProcessEntry>();
        let mut offset = 0usize;
        let mut processes = Vec::new();
        while offset + entry_size <= len {
            // SAFETY: `offset + entry_size <= len` bounds this read, the base
            // allocation is 8-aligned, and kernel entry offsets are multiples
            // of that alignment, so the reference is aligned and in bounds.
            let entry = unsafe { &*base.cast::<u8>().add(offset).cast::<ProcessEntry>() };
            if entry.unique_process_id > 0 && entry.image_name.Length > 0 {
                let units = entry.image_name.Length as usize / 2;
                // SAFETY: `Length` counts the kernel's UTF-16 name, which
                // lives inside the same returned buffer.
                let units = unsafe { slice::from_raw_parts(entry.image_name.Buffer.0, units) };
                processes.push(ProcessSample {
                    pid: entry.unique_process_id as u32,
                    name: String::from_utf16_lossy(units),
                    cpu_100ns: (entry.user_time.max(0) as u64)
                        .saturating_add(entry.kernel_time.max(0) as u64),
                    // The private working set can read negative on system
                    // processes; clamp instead of wrapping into a huge MB row.
                    mem_bytes: entry.working_set_private_size.max(0) as u64,
                });
            }
            let next = entry.next_entry_offset as usize;
            if next <= entry_size {
                break;
            }
            offset += next;
        }
        processes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(idle: u64, kernel: u64, user: u64) -> CpuTotals {
        CpuTotals::synthetic(idle, kernel, user)
    }

    #[test]
    fn total_cpu_is_busy_over_total_with_idle_inside_kernel() {
        // One core, one second: kernel time (5 ms) carries no idle here, so
        // the machine was busy for the whole interval.
        let previous = totals(0, 0, 0);
        let current = totals(0, 5_000_000, 5_000_000);
        let percent = total_cpu_percent(&previous, &current).unwrap();
        assert!((percent - 100.).abs() < 1e-4, "fully busy core: {percent}");
    }

    #[test]
    fn total_cpu_reads_zero_while_idle_and_none_without_time() {
        // Kernel time grows by exactly the idle time: nothing was busy.
        let previous = totals(0, 10_000_000, 0);
        let current = totals(10_000_000, 20_000_000, 0);
        let percent = total_cpu_percent(&previous, &current).unwrap();
        assert!(
            percent.abs() < 1e-4,
            "an idle machine is 0% busy: {percent}"
        );
        assert_eq!(
            total_cpu_percent(&current, &current),
            None,
            "no time passed"
        );
    }

    #[test]
    fn total_cpu_stays_in_range_when_counters_move_oddly() {
        let percent = total_cpu_percent(&totals(0, 0, 0), &totals(100, 50, 50)).unwrap();
        assert_eq!(
            percent, 0.,
            "idle covering the whole interval is not negative"
        );
    }

    #[test]
    fn top_processes_rank_by_cpu_and_pin_the_own_row() {
        let previous = vec![
            ProcessSample {
                pid: 1,
                name: "system".into(),
                cpu_100ns: 0,
                mem_bytes: 1,
            },
            ProcessSample {
                pid: 2,
                name: "chrome".into(),
                cpu_100ns: 0,
                mem_bytes: 2,
            },
            ProcessSample {
                pid: 3,
                name: "moiren-app".into(),
                cpu_100ns: 0,
                mem_bytes: 3,
            },
        ];
        let current = vec![
            ProcessSample {
                pid: 1,
                name: "system".into(),
                cpu_100ns: 1_000_000,
                mem_bytes: 1,
            },
            ProcessSample {
                pid: 2,
                name: "chrome".into(),
                cpu_100ns: 8_000_000,
                mem_bytes: 2,
            },
            ProcessSample {
                pid: 3,
                name: "moiren-app".into(),
                cpu_100ns: 4_000_000,
                mem_bytes: 3,
            },
        ];
        // One second on two cores is 2e7 units of capacity: chrome 40%,
        // moiren 20%, system 5%.
        let rows = top_processes(&previous, &current, Duration::from_secs(1), 2., 3, 2);
        assert_eq!(rows[0].pid, 3, "moiren's own row leads");
        assert!(
            (rows[0].cpu_percent - 20.).abs() < 1e-3,
            "{}",
            rows[0].cpu_percent
        );
        assert_eq!(rows[1].pid, 2, "the rest rank by CPU");
        assert!(
            (rows[1].cpu_percent - 40.).abs() < 1e-3,
            "{}",
            rows[1].cpu_percent
        );
        assert_eq!(rows.len(), 3, "the own row rides on top of `keep` others");
    }

    #[test]
    fn top_processes_without_a_previous_table_reads_zero_cpu() {
        let current = vec![ProcessSample {
            pid: 1,
            name: "system".into(),
            cpu_100ns: 9,
            mem_bytes: 7,
        }];
        let rows = top_processes(&[], &current, Duration::from_secs(1), 2., 99, 3);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cpu_percent, 0.);
        assert_eq!(rows[0].mem_bytes, 7);
    }

    #[test]
    fn top_processes_skips_the_idle_process() {
        let idle = ProcessSample {
            pid: 0,
            name: "Idle".into(),
            cpu_100ns: u64::MAX,
            mem_bytes: 0,
        };
        assert!(top_processes(&[], &[idle], Duration::from_secs(1), 2., 99, 3).is_empty());
    }
}
