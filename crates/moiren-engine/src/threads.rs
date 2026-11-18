//! Process-wide registry of Moiren's own threads: who spawned which named
//! thread, which OS thread id it ended up on, and what class of work it does.
//! The status bar's per-thread performance panel matches this registry against
//! the OS's per-thread CPU accounting (`SystemProcessInformation` walk in
//! `moiren-app`), so every row the panel draws is attributable to a spawn site
//! in this workspace — with [`ThreadClass::Plugin`] reserved for the future
//! plugin-host isolation threads.
//!
//! ## Registration discipline (read before wiring a spawn site)
//!
//! Registration is deliberately **parent-side**: [`register_thread`] runs on
//! the spawning (control) thread, where the registry mutex and the row
//! allocation are allowed. The spawned thread's only registry action is
//! [`ThreadGuard::enter`] — one `GetCurrentThreadId` call plus one relaxed
//! atomic store. Realtime threads therefore never touch a lock or an
//! allocator on the registry path, which is stricter than the documented
//! contract requires (startup/teardown may allocate) and keeps the active
//! phase's zero-syscall budget intact: the registry is never consulted again
//! after `enter`.
//!
//! The guard is moved into the thread closure and unregisters on drop, so a
//! panicked, early-returning or short-lived thread (e.g. an ASIO capability
//! probe) always retires its row. Long-lived threads hold their guard until
//! the thread itself returns. Rows are keyed by OS thread id, not by name —
//! names are presentation, and duplicated names get instance ordinals at
//! snapshot time.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{
    Arc, LazyLock, Mutex,
    atomic::{AtomicU32, Ordering},
};

/// What class of work a registered thread performs. The status bar's panel
/// groups rows by this; the labels (RT/CTL/UI/VST) are presentation and live
/// with the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadClass {
    /// Realtime audio: touches native streams or paces `Engine::render`
    /// against audio deadlines (`moiren-professional-master`, followers,
    /// `moiren-virtual-owner`, `moiren-vad-pcm`, `moiren-shared-capture`).
    Rt,
    /// Control/observation: compilation, reclamation, service dispatch —
    /// everything `AGENTS.md` requires to stay off the audio thread.
    Control,
    /// The UI thread (gpui event loop).
    Ui,
    /// Plugin host threads (VST/CLAP isolation). Reserved: no spawn site
    /// registers this class yet.
    Plugin,
}

/// One registry row as the UI reads it: the OS thread id the row landed on
/// (0 while the spawned thread has not entered yet) and the display name,
/// which carries an instance ordinal whenever the name is live more than
/// once (two follower device owners read `moiren-professional-device #1` and
/// `#2`; a lone thread reads its bare name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInfo {
    pub tid: u32,
    pub name: String,
    pub class: ThreadClass,
}

struct Row {
    serial: u64,
    name: Box<str>,
    class: ThreadClass,
    ordinal: usize,
    tid: AtomicU32,
}

#[derive(Default)]
struct Registry {
    /// Serial → row. `BTreeMap` keeps snapshots in registration order so the
    /// panel's rows are stable without sorting churn.
    rows: BTreeMap<u64, Arc<Row>>,
    /// Per-name set of currently used ordinals; a new registration takes the
    /// smallest free one, so ordinals stay small under device churn instead
    /// of climbing with every re-prepare.
    ordinals: HashMap<Box<str>, BTreeSet<usize>>,
    next_serial: u64,
}

impl Registry {
    fn register(&mut self, name: &str, class: ThreadClass) -> Arc<Row> {
        let ordinals = self.ordinals.entry(name.into()).or_default();
        let ordinal = (1..)
            .find(|candidate| !ordinals.contains(candidate))
            .expect("the ordinal candidate range is inexhaustible");
        ordinals.insert(ordinal);
        let serial = self.next_serial;
        self.next_serial += 1;
        let row = Arc::new(Row {
            serial,
            name: name.into(),
            class,
            ordinal,
            tid: AtomicU32::new(0),
        });
        self.rows.insert(serial, Arc::clone(&row));
        row
    }

    fn unregister(&mut self, row: &Row) {
        self.rows.remove(&row.serial);
        if let Some(ordinals) = self.ordinals.get_mut(&row.name) {
            ordinals.remove(&row.ordinal);
            if ordinals.is_empty() {
                self.ordinals.remove(&row.name);
            }
        }
    }
}

/// The process-wide registry. `LazyLock` because `HashMap::new` is not const;
/// the first registration (a control thread) pays the allocation.
static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));

/// Registers a thread row on the spawning thread and returns the guard that
/// owns the row's lifetime. Call this next to `thread::Builder::spawn`, move
/// the guard into the closure, and make [`ThreadGuard::enter`] the closure's
/// first statement. Dropping the guard without `enter` (spawn failed, thread
/// never ran) removes the row.
///
/// The lock and the row allocation happen here, on the caller's thread: see
/// the module header for why the spawned thread's path must stay lock-free.
pub fn register_thread(name: &str, class: ThreadClass) -> ThreadGuard {
    let row = REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .register(name, class);
    ThreadGuard { row: Some(row) }
}

/// Reads the live rows for the UI's performance panel. Copies under the
/// registry lock — this runs on the UI side at 1 Hz while the panel is open,
/// never on a realtime thread.
pub fn snapshot() -> Vec<ThreadInfo> {
    let registry = REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for row in registry.rows.values() {
        *counts.entry(&row.name).or_insert(0) += 1;
    }
    registry
        .rows
        .values()
        .map(|row| {
            let duplicated = counts[&*row.name] > 1;
            ThreadInfo {
                tid: row.tid.load(Ordering::Relaxed),
                name: if duplicated {
                    format!("{} #{}", row.name, row.ordinal)
                } else {
                    row.name.to_string()
                },
                class: row.class,
            }
        })
        .collect()
}

/// One thread's stake in the registry. `Send` by construction (the closure
/// moves it into the spawned thread); dropping it unregisters the row.
pub struct ThreadGuard {
    row: Option<Arc<Row>>,
}

impl ThreadGuard {
    /// The spawned thread's first statement: records this thread's OS id
    /// under the row. Realtime-safe — one `GetCurrentThreadId` call and one
    /// relaxed store, no allocation, no lock (see the module header). A row
    /// is entered once; a later `enter` on the same guard keeps the first id.
    pub fn enter(&self) {
        let Some(row) = self.row.as_ref() else {
            return;
        };
        row.tid
            .compare_exchange(0, current_thread_id(), Ordering::Relaxed, Ordering::Relaxed)
            .ok();
    }

    /// The row's OS thread id, 0 until [`Self::enter`].
    pub fn thread_id(&self) -> u32 {
        self.row
            .as_ref()
            .map_or(0, |row| row.tid.load(Ordering::Relaxed))
    }
}

impl Drop for ThreadGuard {
    fn drop(&mut self) {
        if let Some(row) = self.row.take() {
            REGISTRY
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .unregister(&row);
        }
    }
}

/// The current thread's OS id, the key the panel's sampler joins against the
/// kernel's per-thread accounting. Windows is the platform the audio backend
/// and the sampler exist on; elsewhere the registry still needs *a* stable
/// per-thread key for its own rows, so a process-local counter stands in —
/// the panel degrades to name-only rows there (no CPU sampling data).
fn current_thread_id() -> u32 {
    #[cfg(windows)]
    {
        // SAFETY: GetCurrentThreadId takes no arguments, cannot fail, and
        // only reads this thread's environment block.
        unsafe { GetCurrentThreadId() }
    }
    #[cfg(not(windows))]
    {
        static SURROGATE: AtomicU32 = AtomicU32::new(1);
        SURROGATE.fetch_add(1, Ordering::Relaxed)
    }
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is process-global and the harness runs tests in
    /// parallel, so every test reads only the rows of its own unique name
    /// prefix instead of asserting on whole-snapshot counts.
    fn rows_with(prefix: &str) -> Vec<ThreadInfo> {
        snapshot()
            .into_iter()
            .filter(|info| info.name.starts_with(prefix))
            .collect()
    }

    #[test]
    fn a_registered_row_is_live_until_the_guard_drops() {
        let guard = register_thread("test-row", ThreadClass::Control);
        assert_eq!(guard.thread_id(), 0, "not entered yet");
        guard.enter();
        assert_ne!(guard.thread_id(), 0, "enter records the OS id");
        let rows = rows_with("test-row");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "test-row", "a lone row reads its bare name");
        assert_eq!(rows[0].class, ThreadClass::Control);
        assert_eq!(rows[0].tid, guard.thread_id());
        drop(guard);
        assert!(rows_with("test-row").is_empty(), "drop unregisters");
    }

    #[test]
    fn entering_twice_keeps_the_first_os_id() {
        let guard = register_thread("test-once", ThreadClass::Rt);
        guard.enter();
        let first = guard.thread_id();
        guard.enter();
        assert_eq!(guard.thread_id(), first);
        drop(guard);
    }

    #[test]
    fn a_dropped_guard_without_enter_removes_the_row() {
        let guard = register_thread("test-never-entered", ThreadClass::Control);
        assert_eq!(guard.thread_id(), 0);
        drop(guard);
        assert!(rows_with("test-never-entered").is_empty());
    }

    #[test]
    fn duplicated_names_carry_ordinals_and_lone_names_do_not() {
        let first = register_thread("test-follower", ThreadClass::Rt);
        let second = register_thread("test-follower", ThreadClass::Rt);
        let live = rows_with("test-follower");
        assert_eq!(live.len(), 2);
        assert!(
            live.iter()
                .all(|info| info.name.starts_with("test-follower #")),
            "both live rows show their ordinal: {live:?}"
        );
        assert_ne!(live[0].name, live[1].name, "the ordinals differ");
        drop(second);
        let live = rows_with("test-follower");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "test-follower");
        drop(first);
    }

    #[test]
    fn freed_ordinals_are_reused_so_churn_does_not_climb() {
        let keep = register_thread("test-churn", ThreadClass::Control);
        for _ in 0..4 {
            let churned = register_thread("test-churn", ThreadClass::Control);
            // Two live rows: both carry ordinals, and the pair is #1 and #2
            // every time — the freed slot was reused, not stacked.
            let mut rows = rows_with("test-churn");
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            assert_eq!(rows.len(), 2, "one lone + one ordinalled row");
            assert_eq!(rows[0].name, "test-churn #1");
            assert_eq!(rows[1].name, "test-churn #2");
            drop(churned);
        }
        drop(keep);
        let lone = register_thread("test-churn", ThreadClass::Control);
        assert_eq!(
            rows_with("test-churn")[0].name,
            "test-churn",
            "a lone row reads its bare name again"
        );
        drop(lone);
    }
}
