//! Optional disk observations. Never called on the service, UI or audio thread.
use std::path::Path;

pub(super) struct Observation {
    pub available: u64,
    pub volume: Option<String>,
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(super) fn observe(path: &Path) -> Option<Observation> {
    use std::os::windows::ffi::OsStrExt;

    // Bind the OS entry point directly: no new dependency or transitive Cargo
    // feature is required just to observe the caller's available quota.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
        fn GetVolumePathNameW(path: *const u16, volume_path: *mut u16, length: u32) -> i32;
        fn GetVolumeNameForVolumeMountPointW(path: *const u16, name: *mut u16, length: u32) -> i32;
    }
    // Resolve parent junctions/symlinks before identifying the volume. Failure
    // means unknown rather than grouping unrelated volumes by drive letters.
    let directory = std::fs::canonicalize(path.parent()?).ok()?;
    let mut wide: Vec<u16> = directory.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    let mut available = 0;
    // SAFETY: wide is a live NUL-terminated UTF-16 buffer; available is an
    // aligned writable ULARGE_INTEGER-sized output. Optional outputs are null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    let mut root = vec![0u16; 32_768];
    let mut name = [0u16; 64];
    // SAFETY: input is terminated, output buffers are initialized and writable,
    // lengths describe the full buffers. The second call reads root only after
    // the first succeeds. Network shares without a volume GUID remain unknown.
    let identified = unsafe {
        GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) != 0
            && GetVolumeNameForVolumeMountPointW(
                root.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
            ) != 0
    };
    let volume = identified.then(|| {
        let length = name
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(name.len());
        String::from_utf16_lossy(&name[..length]).to_ascii_uppercase()
    });
    Some(Observation { available, volume })
}

#[cfg(not(windows))]
pub(super) fn observe(_path: &Path) -> Option<Observation> {
    // std exposes no portable free-space query. Do not report a made-up limit.
    None
}
