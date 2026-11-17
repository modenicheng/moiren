//! Host system performance samples for the status bar's performance panel:
//! total CPU from the kernel's time accounting and a per-thread table of
//! this process's own threads (the raw half of the per-thread panel rows;
//! deltas, rebasing and presentation live in the UI model). The Win32 reads
//! live in [`win`]; every delta and formatting decision is pure code at the
//! bottom of this file, unit-tested with synthetic samples on every platform
//! the crate builds on.

#[cfg(windows)]
pub(crate) use win::{sample_cpu_totals, sample_threads};

#[cfg(not(windows))]
pub(crate) fn sample_cpu_totals() -> Option<CpuTotals> {
    None
}

#[cfg(not(windows))]
pub(crate) fn sample_threads() -> Vec<ThreadSample> {
    Vec::new()
}

/// One instant of one thread's kernel accounting, in 100 ns units, plus the
/// thread's creation time: the sampler rebases on a creation-time mismatch
/// instead of trusting a CPU delta across a Windows thread-id reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThreadSample {
    pub tid: u32,
    pub kernel_100ns: u64,
    pub user_100ns: u64,
    pub create_time_100ns: i64,
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

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    //! All the unsafe in this module is Win32's own contract: documented
    //! kernel calls with caller-owned out-buffers, plus one
    //! `NtQuerySystemInformation` walk whose entry layout is declared in
    //! full below rather than read out of the crate's `Reserved1` bytes.

    use windows::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemProcessInformation,
    };
    use windows::Win32::Foundation::{FILETIME, NTSTATUS, UNICODE_STRING};
    use windows::Win32::System::Threading::GetSystemTimes;

    use super::{CpuTotals, ThreadSample};

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
    /// layout here — its field order has been stable across every 64-bit
    /// Windows release this targets (field-for-field: u32 pair, then seven
    /// 64-bit fields at their natural offsets, the `UNICODE_STRING`, then
    /// the id/handle/size block).
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

    /// One `SYSTEM_THREAD_INFORMATION` record, the array each process entry
    /// carries inline after its fixed part. Same declaration policy as
    /// [`ProcessEntry`]: spelled out in full because the crate binding hides
    /// the time fields inside reserved bytes.
    ///
    /// The record changed with Windows 11 24H2: the creation time and the
    /// `CLIENT_ID` pair moved eight bytes later (an extra 8-byte slot after
    /// the CPU times), while the stride stayed 80 bytes. Both generations
    /// are declared; [`collect_threads`] calibrates per record by picking
    /// the layout whose every entry carries the owning process's id.
    #[repr(C)]
    struct ThreadEntryClassic {
        kernel_time: i64,
        user_time: i64,
        create_time: i64,
        wait_time: u32,
        start_address: usize,
        client_process_id: usize,
        client_thread_id: usize,
        priority: i32,
        base_priority: i32,
        context_switches: u32,
        thread_state: u32,
        wait_reason: u32,
    }

    #[repr(C)]
    struct ThreadEntry24H2 {
        kernel_time: i64,
        user_time: i64,
        reserved: u64,
        create_time: i64,
        start_address: usize,
        wait_time: u64,
        client_process_id: usize,
        client_thread_id: usize,
        priority: i32,
        base_priority: i32,
        context_switches: u32,
        thread_state: u32,
    }

    // Layout pins: the walker's stride arithmetic relies on both records
    // being 80 bytes. The geometry constants are the 64-bit ABI — x64 is
    // the only Windows target this workspace builds (the ASIO query path
    // is likewise 64-bit-only).
    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(size_of::<ThreadEntryClassic>() == 80);
    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(size_of::<ThreadEntry24H2>() == 80);

    pub(crate) fn sample_threads() -> Vec<ThreadSample> {
        // The thread-record geometry below is the 64-bit ABI; other pointer
        // widths are not Windows targets for this workspace.
        if !cfg!(target_pointer_width = "64") {
            return Vec::new();
        }
        let own_pid = std::process::id();
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
            let base = buffer.as_ptr().cast();
            let len = returned as usize;
            let mut threads = Vec::new();
            walk_entries(base, len, |entry, record_end| {
                if entry.unique_process_id as u32 != own_pid {
                    return;
                }
                collect_threads(entry, base, record_end, &mut threads);
            });
            return threads;
        }
    }

    /// Walks the kernel's chain of process entries once, handing each entry
    /// to `visit` together with the record's end offset (the bound the
    /// inline thread array must fit inside). `base` points at `len` aligned
    /// bytes written by the kernel in the call above; every hop follows the
    /// chain's own `next_entry_offset`.
    fn walk_entries(
        base: *const ProcessEntry,
        len: usize,
        mut visit: impl FnMut(&ProcessEntry, usize),
    ) {
        let entry_size = size_of::<ProcessEntry>();
        let mut offset = 0usize;
        while offset + entry_size <= len {
            // SAFETY: `offset + entry_size <= len` bounds this read, the base
            // allocation is 8-aligned, and kernel entry offsets are multiples
            // of that alignment, so the reference is aligned and in bounds.
            let entry = unsafe { &*base.cast::<u8>().add(offset).cast::<ProcessEntry>() };
            let next = entry.next_entry_offset as usize;
            let record_end = if next > entry_size {
                offset.checked_add(next).unwrap_or(len).min(len)
            } else {
                len
            };
            visit(entry, record_end);
            if next <= entry_size {
                break;
            }
            offset += next;
        }
    }

    /// Collects the `SYSTEM_THREAD_INFORMATION` array that follows this
    /// process record's fixed part. The array's start and stride are stable
    /// across Windows releases (fixed part, then the 80-byte records); only
    /// the field offsets inside a record moved with Windows 11 24H2, so the
    /// layout is calibrated per record: the chosen generation must carry the
    /// owning process's id in every entry. Anything else skips the rows — a
    /// malformed or unknown kernel reply costs missing rows, never garbage
    /// or an out-of-bounds read.
    fn collect_threads(
        entry: &ProcessEntry,
        base: *const ProcessEntry,
        record_end: usize,
        threads: &mut Vec<ThreadSample>,
    ) {
        let count = entry.number_of_threads as usize;
        if count == 0 {
            return;
        }
        let record_offset = (entry as *const ProcessEntry).addr() - base.addr();
        let stride = size_of::<ThreadEntryClassic>();
        let threads_end = size_of::<ProcessEntry>().checked_add(count * stride);
        let Some(relative_end) = threads_end else {
            return;
        };
        if record_offset + relative_end > record_end {
            return;
        }
        let record_pid = entry.unique_process_id as u32;
        // SAFETY: the whole array sits inside the process record, which
        // `walk_entries` bounded by the returned length, and the base is
        // 8-aligned, so the u64-word reads at 8-aligned field offsets are
        // aligned and in bounds.
        let array = |bytes: usize| unsafe {
            std::slice::from_raw_parts(
                base.cast::<u8>()
                    .add(record_offset + size_of::<ProcessEntry>())
                    .cast::<u64>(),
                bytes / size_of::<u64>(),
            )
        };
        fn record_words(words: &[u64], index: usize, stride: usize) -> &[u64] {
            let per_record = stride / size_of::<u64>();
            &words[index * per_record..][..per_record]
        }
        // The calibrated read: `(client_process_id, client_thread_id,
        // kernel_time, user_time, create_time)` per entry under one
        // generation's field offsets.
        let read_with = |stride: usize, offsets: (usize, usize, usize, usize, usize)| {
            let words = array(count * stride);
            (0..count)
                .all(|index| {
                    let record = record_words(words, index, stride);
                    let pid = u64_at(record, offsets.0) as u32;
                    pid == record_pid && u64_at(record, offsets.1) != 0
                })
                .then(|| {
                    (0..count)
                        .map(|index| {
                            let record = record_words(words, index, stride);
                            ThreadSample {
                                tid: u64_at(record, offsets.1) as u32,
                                kernel_100ns: u64_at(record, offsets.2),
                                user_100ns: u64_at(record, offsets.3),
                                create_time_100ns: u64_at(record, offsets.4) as i64,
                            }
                        })
                        .collect::<Vec<ThreadSample>>()
                })
        };
        // Windows 11 24H2 first (the current generation), the classic
        // layout second. Reading a record through the wrong generation's
        // offsets lands on start-address bytes for the client ids, which
        // never equal the owning pid for a whole array.
        let parsed =
            read_with(stride, (48, 56, 0, 8, 24)).or_else(|| read_with(stride, (40, 48, 0, 8, 16)));
        threads.extend(parsed.unwrap_or_default());
    }

    /// Reads a little-endian u64 at a byte offset inside an aligned u64
    /// slice (the offsets are 8-aligned kernel fields).
    fn u64_at(record: &[u64], byte_offset: usize) -> u64 {
        record[byte_offset / size_of::<u64>()]
    }

    #[cfg(test)]
    mod walker_tests {
        //! Synthetic-buffer construction and shape assertions for the
        //! `SystemProcessInformation` walk: the ABI layout is pinned by the
        //! constants below, the parse is driven from hand-built buffers in
        //! both field-layout generations, and one live sample smoke-checks
        //! the calibration against the running kernel (Windows-only; the
        //! pure math is tested everywhere).

        use super::*;
        use windows::Win32::Foundation::UNICODE_STRING;
        use windows::core::PWSTR;

        /// Which thread-record generation a synthetic buffer is built with.
        #[derive(Clone, Copy, Debug)]
        enum Layout {
            Classic,
            Win1124H2,
        }

        impl Layout {
            /// Writes one thread record's five parsed fields into `words`
            /// at this generation's offsets.
            fn write(
                self,
                words: &mut [u64],
                kernel: i64,
                user: i64,
                create: i64,
                pid: usize,
                tid: usize,
            ) {
                let _ = (kernel, user);
                match self {
                    // SAFETY: the slice is 8-aligned and exactly one
                    // declared record long.
                    Self::Classic => unsafe {
                        let record = words.as_mut_ptr().cast::<ThreadEntryClassic>();
                        (*record) = ThreadEntryClassic {
                            kernel_time: kernel,
                            user_time: user,
                            create_time: create,
                            wait_time: 0,
                            start_address: 0,
                            client_process_id: pid,
                            client_thread_id: tid,
                            priority: 8,
                            base_priority: 8,
                            context_switches: 0,
                            thread_state: 5,
                            wait_reason: 6,
                        };
                    },
                    // SAFETY: as above, for the newer generation's record.
                    Self::Win1124H2 => unsafe {
                        let record = words.as_mut_ptr().cast::<ThreadEntry24H2>();
                        (*record) = ThreadEntry24H2 {
                            kernel_time: kernel,
                            user_time: user,
                            reserved: 0,
                            create_time: create,
                            start_address: 0,
                            wait_time: 0,
                            client_process_id: pid,
                            client_thread_id: tid,
                            priority: 8,
                            base_priority: 8,
                            context_switches: 0,
                            thread_state: 5,
                        };
                    },
                }
            }
        }

        #[test]
        fn the_thread_record_layouts_match_the_documented_abis() {
            // Both strides are 80 bytes on 64-bit so one array serves either
            // generation; the walker's bounds math relies on the stride.
            assert_eq!(size_of::<ThreadEntryClassic>(), 80);
            assert_eq!(size_of::<ThreadEntry24H2>(), 80);
            assert_eq!(
                size_of::<ThreadEntryClassic>(),
                size_of::<ThreadEntry24H2>()
            );
        }

        /// Builds a two-process buffer: one other process (no threads), then
        /// Moiren's own with `thread_count` threads written in `layout`.
        /// Both records follow the kernel's observed geometry: fixed part,
        /// thread array, one image-name buffer at the record's end.
        fn build_buffer(
            own_pid: u32,
            thread_count: usize,
            layout: Layout,
        ) -> (Vec<u64>, usize, usize) {
            let entry_size = size_of::<ProcessEntry>();
            let name = "moiren-app".encode_utf16().chain([0]).collect::<Vec<_>>();
            let name_bytes = name.len() * size_of::<u16>();
            let threads_bytes = thread_count * size_of::<ThreadEntryClassic>();
            let name_offset = entry_size + threads_bytes;
            let record_len = name_offset + name_bytes;
            let record_stride = record_len.next_multiple_of(8);
            let other_name = "chrome".encode_utf16().chain([0]).collect::<Vec<_>>();
            let other_bytes = other_name.len() * size_of::<u16>();
            let other_len = (entry_size + other_bytes).next_multiple_of(8);
            let total = other_len + record_stride;
            let mut buffer = vec![0u64; total.div_ceil(size_of::<u64>())];
            let base = buffer.as_mut_ptr() as usize;

            let write_process = |offset: usize, pid: u32, next: usize, name_units: &[u16]| {
                let entry = ProcessEntry {
                    next_entry_offset: next as u32,
                    number_of_threads: if pid == own_pid {
                        thread_count as u32
                    } else {
                        0
                    },
                    working_set_private_size: 0,
                    hard_fault_count: 0,
                    number_of_threads_high_watermark: 0,
                    cycle_time: 0,
                    create_time: 0,
                    user_time: 7,
                    kernel_time: 11,
                    image_name: UNICODE_STRING {
                        Length: (name_units.len() * 2) as u16,
                        MaximumLength: (name_units.len() * 2) as u16,
                        Buffer: PWSTR::null(),
                    },
                    base_priority: 0,
                    unique_process_id: pid as usize,
                    inherited_from_unique_process_id: 0,
                    handle_count: 0,
                    session_id: 0,
                    peak_virtual_size: 0,
                    virtual_size: 0,
                    page_fault_count: 0,
                    peak_working_set_size: 0,
                    working_set_size: 0,
                    quota_peak_paged_pool_usage: 0,
                    quota_paged_pool_usage: 0,
                    quota_peak_non_paged_pool_usage: 0,
                    quota_non_paged_pool_usage: 0,
                    pagefile_usage: 0,
                    peak_pagefile_usage: 0,
                    private_page_count: 0,
                    reserved: [0; 6],
                };
                // SAFETY: `buffer` is a live 8-aligned allocation and
                // `offset` is the record's aligned start inside it.
                unsafe {
                    let entry_ptr = (base + offset) as *mut ProcessEntry;
                    entry_ptr.write(entry);
                    let name_ptr = (base + offset + name_offset) as *mut u16;
                    for (index, unit) in name_units.iter().enumerate() {
                        name_ptr.add(index).write(*unit);
                    }
                    (*entry_ptr).image_name.Buffer = PWSTR(name_ptr);
                }
            };
            write_process(0, 99, other_len, &other_name);
            write_process(other_len, own_pid, 0, &name);
            for index in 0..thread_count {
                // SAFETY: the array offset was computed from the same layout
                // constants the walker uses, inside the live allocation.
                let words = unsafe {
                    std::slice::from_raw_parts_mut(
                        (base + other_len + entry_size + index * size_of::<ThreadEntryClassic>())
                            as *mut u64,
                        size_of::<ThreadEntryClassic>() / size_of::<u64>(),
                    )
                };
                layout.write(
                    words,
                    1_000 * (index as i64 + 1),
                    2_000 * (index as i64 + 1),
                    4_000_000_000 + index as i64,
                    own_pid as usize,
                    500 + index,
                );
            }
            (buffer, total, other_len)
        }

        fn own_threads(buffer: &[u64], total: usize, own_pid: u32) -> Vec<ThreadSample> {
            let mut threads = Vec::new();
            walk_entries(buffer.as_ptr().cast(), total, |entry, record_end| {
                if entry.unique_process_id as u32 == own_pid {
                    collect_threads(entry, buffer.as_ptr().cast(), record_end, &mut threads);
                }
            });
            threads
        }

        #[test]
        fn the_walker_reads_the_own_process_thread_array() {
            for layout in [Layout::Classic, Layout::Win1124H2] {
                let (buffer, total, _) = build_buffer(4242, 2, layout);
                let threads = own_threads(&buffer, total, 4242);
                assert_eq!(threads.len(), 2, "{layout:?}");
                assert_eq!(threads[0].tid, 500);
                assert_eq!(threads[0].kernel_100ns, 1_000);
                assert_eq!(threads[0].user_100ns, 2_000);
                assert_eq!(threads[0].create_time_100ns, 4_000_000_000);
                assert_eq!(threads[1].tid, 501);
                assert_eq!(threads[1].kernel_100ns, 2_000);
            }
        }

        #[test]
        fn other_processes_threads_are_not_collected() {
            let (buffer, total, _) = build_buffer(4242, 2, Layout::Win1124H2);
            assert!(own_threads(&buffer, total, 99).is_empty());
            assert!(own_threads(&buffer, total, 7).is_empty());
        }

        #[test]
        fn a_thread_array_with_a_foreign_process_id_is_refused() {
            // Thread records claiming another owner's pid in every entry
            // match neither generation's calibration: the walker refuses the
            // array instead of reporting garbage.
            let (mut buffer, total, own_offset) = build_buffer(4242, 2, Layout::Win1124H2);
            for index in 0..2 {
                let record = (buffer.as_mut_ptr() as usize
                    + own_offset
                    + size_of::<ProcessEntry>()
                    + index * size_of::<ThreadEntry24H2>())
                    as *mut ThreadEntry24H2;
                // SAFETY: patching the live synthetic record's client ids.
                unsafe {
                    (*record).client_process_id = 999;
                }
            }
            assert!(own_threads(&buffer, total, 4242).is_empty());
        }

        #[test]
        fn a_thread_count_beyond_the_record_is_skipped_not_read() {
            let (mut buffer, total, own_offset) = build_buffer(4242, 2, Layout::Win1124H2);
            // Claim twelve threads: only two fit in the record. The walker
            // must refuse the whole array rather than read past its end.
            // SAFETY: patching the live synthetic record's thread count.
            unsafe {
                let entry_ptr = (buffer.as_mut_ptr() as usize + own_offset) as *mut ProcessEntry;
                (*entry_ptr).number_of_threads = 12;
            }
            assert!(own_threads(&buffer, total, 4242).is_empty());
        }

        /// The layout calibration is validated against the running kernel
        /// once per Windows CI run: a real sample must carry several distinct
        /// thread ids with nonzero creation times.
        #[test]
        fn a_live_sample_carries_the_process_threads() {
            let threads = sample_threads();
            assert!(threads.len() >= 2, "test harness + runtime threads");
            let mut tids: Vec<u32> = threads.iter().map(|t| t.tid).collect();
            tids.sort_unstable();
            tids.dedup();
            assert_eq!(tids.len(), threads.len(), "thread ids are unique");
            assert!(
                threads.iter().any(|t| t.create_time_100ns != 0),
                "at least one creation time is real"
            );
        }
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
}
