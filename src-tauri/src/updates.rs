//! In-app updates. Checking, downloading and installing are the updater
//! plugin's, driven by the UI (`src/lib/updater.ts`); this adds what the
//! plugin leaves to the app: whether this copy can replace itself at all.
//! "Check for Updates…" in the app menu is in `menu.rs`.

/// Whether the app runs from somewhere it cannot replace itself: a mounted
/// DMG, or a copy macOS translocated because it was opened without being
/// moved out of Downloads. Both sit on a read-only volume.
#[tauri::command]
pub fn update_blocked() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    // Contents/MacOS/otomo → Otomo.app
    match exe.ancestors().nth(3) {
        Some(bundle) => read_only_volume(bundle),
        None => false,
    }
}

#[cfg(target_os = "macos")]
fn read_only_volume(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: statfs writes into the zeroed struct and reads only c_path.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) } == 0;
    ok && (stat.f_flags as u64) & (libc::MNT_RDONLY as u64) != 0
}

#[cfg(not(target_os = "macos"))]
fn read_only_volume(_path: &std::path::Path) -> bool {
    false
}
