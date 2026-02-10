//! Linux-specific single instance detection using file locks.
//!
//! This module provides a fallback mechanism for single instance detection on Linux,
//! where the D-Bus based detection from tauri-plugin-single-instance may not work
//! reliably in all environments (different D-Bus sessions, SSH, etc.).
//!
//! ## Crash Recovery
//!
//! This implementation uses `flock()` which is **process-level locking**:
//! - When a process crashes or exits (normally or abnormally), the OS automatically
//!   releases all `flock` locks held by that process.
//! - Even if the lock file still exists on disk, a new process can successfully
//!   acquire the lock because the previous lock holder is gone.
//! - This means stale lock files from crashed processes are NOT a problem.

#[cfg(target_os = "linux")]
use std::fs::{File, OpenOptions};
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use log::{error, info, warn};

/// Holds the lock file handle to keep the lock active for the application lifetime.
#[cfg(target_os = "linux")]
pub struct SingleInstanceLock {
    _lock_file: File,
    lock_path: PathBuf,
}

#[cfg(target_os = "linux")]
impl Drop for SingleInstanceLock {
    fn drop(&mut self) {
        // Lock is automatically released when file is closed
        // Optionally remove the lock file
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Get the path for the lock file.
/// Uses XDG_RUNTIME_DIR if available (preferred for Linux), falls back to /tmp.
#[cfg(target_os = "linux")]
fn get_lock_file_path() -> PathBuf {
    // Try XDG_RUNTIME_DIR first (user-specific, cleared on logout)
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime_dir).join("ai-toolbox.lock");
    }

    // Fallback to /tmp with user ID to avoid conflicts
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/ai-toolbox-{}.lock", uid))
}

/// Try to acquire a single instance lock.
/// Returns Ok(SingleInstanceLock) if this is the first instance.
/// Returns Err with the PID of the existing instance if another instance is running.
#[cfg(target_os = "linux")]
pub fn try_acquire_lock() -> Result<SingleInstanceLock, String> {
    use std::os::unix::io::AsRawFd;

    let lock_path = get_lock_file_path();
    info!("尝试获取单实例锁: {:?}", lock_path);

    // Ensure parent directory exists
    if let Some(parent) = lock_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| format!("无法创建锁文件目录: {}", e))?;
        }
    }

    // Open or create the lock file
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600) // Only owner can read/write
        .open(&lock_path)
        .map_err(|e| format!("无法打开锁文件: {}", e))?;

    // Try to acquire an exclusive lock (non-blocking)
    let fd = lock_file.as_raw_fd();
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

    if result != 0 {
        let errno = std::io::Error::last_os_error();
        if errno.raw_os_error() == Some(libc::EWOULDBLOCK) {
            // Another instance is holding the lock
            // Try to read the PID from the lock file
            let mut existing_file = File::open(&lock_path).ok();
            let mut pid_str = String::new();
            if let Some(ref mut f) = existing_file {
                let _ = f.read_to_string(&mut pid_str);
            }
            let pid = pid_str.trim();

            warn!(
                "检测到另一个实例正在运行 (PID: {})",
                if pid.is_empty() { "unknown" } else { pid }
            );
            return Err(format!(
                "另一个实例正在运行 (PID: {})",
                if pid.is_empty() { "unknown" } else { pid }
            ));
        } else {
            error!("获取文件锁失败: {}", errno);
            return Err(format!("获取文件锁失败: {}", errno));
        }
    }

    // Successfully acquired the lock, write our PID
    let mut lock_file = lock_file;
    let _ = lock_file.set_len(0); // Truncate
    let pid = std::process::id();
    let _ = lock_file.write_all(pid.to_string().as_bytes());
    let _ = lock_file.sync_all();

    info!("成功获取单实例锁 (PID: {})", pid);

    Ok(SingleInstanceLock {
        _lock_file: lock_file,
        lock_path,
    })
}

/// Check if another instance is running and try to bring it to focus.
/// This attempts to use D-Bus to communicate with the existing instance.
#[cfg(target_os = "linux")]
pub fn try_focus_existing_instance() -> bool {
    // Try to send a signal to the existing instance via a simple mechanism
    // For now, we just return false and let the application exit
    // The tauri-plugin-single-instance should handle the focus if D-Bus works
    false
}

// Stub implementations for non-Linux platforms
#[cfg(not(target_os = "linux"))]
pub struct SingleInstanceLock;

#[cfg(not(target_os = "linux"))]
pub fn try_acquire_lock() -> Result<SingleInstanceLock, String> {
    // On non-Linux platforms, always succeed (rely on tauri-plugin-single-instance)
    Ok(SingleInstanceLock)
}

#[cfg(not(target_os = "linux"))]
pub fn try_focus_existing_instance() -> bool {
    false
}
