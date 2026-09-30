//! Process liveness by pid, for boot and read-time reconciliation of stranded
//! rows (`main.rs`) and for the executor's watchdog, which stops once the
//! spawned process is gone.

/// Whether process `pid` is running. On Unix, `kill(pid, 0)`: alive and
/// signalable, or alive but owned by someone else (`EPERM`). On Windows, an
/// open-able process whose exit code is still `STILL_ACTIVE`. Elsewhere there
/// is no portable check, so every pid reads as alive and a peer server's tasks
/// are never clobbered.
pub fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    alive(pid)
}

#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // kill(pid, 0): 0 => alive & signalable; EPERM => alive but not ours; ESRCH => dead.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn alive(pid: i32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code as *mut u32);
        CloseHandle(handle);
        ok != 0 && code as i32 == STILL_ACTIVE
    }
}

#[cfg(not(any(unix, windows)))]
fn alive(_pid: i32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_nonpositive_pids_are_not() {
        assert!(process_alive(std::process::id() as i32));
        assert!(!process_alive(0));
        assert!(!process_alive(-1));
    }
}
