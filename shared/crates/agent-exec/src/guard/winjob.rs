//! Windows process-tree control via Job Objects: a kill-on-close job is the
//! parent-death guard, and a plain job (for `GuardMode::Off`) only lets a
//! cancel end the whole tree, as a process group does on Unix.
//!
//! Unlike Linux/macOS (see `src/bin/agent-guard/`), Windows needs no guard
//! process:
//! `AssignProcessToJobObject` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` makes the
//! **kernel** the reactor — job membership propagates to children
//! automatically, and the whole job is torn down when the job handle's last
//! reference closes, with no process needing to stay alive to react. That
//! includes the server dying: its handles close with it.
//!
//! Entry points: [`WinJob::protect`], called right after a spawn of a process
//! created suspended, and [`resume_process`], called once the job holds it, so
//! the process cannot start anything before it is in the job (a child created
//! before the assignment would be outside the job and outlive it). The
//! returned job is owned by `guard::ProcessTree`, which terminates it on
//! cancel and drops (closes) it when the run ends.

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

/// A Job Object. Dropping it closes the handle, which kills every process
/// still in the job when it was created kill-on-close. Job Object handles are usable from any
/// thread (the Win32 API has no thread affinity for them), so this is `Send`
/// and `Sync`; the owner never shares it mutably.
pub(crate) struct WinJob(HANDLE);

unsafe impl Send for WinJob {}
unsafe impl Sync for WinJob {}

impl WinJob {
    /// Create a Job Object and assign the just-spawned process (`process`,
    /// from `Child::raw_handle()`) to it. With `kill_on_close` the job kills
    /// every process still in it when its last handle closes, including when
    /// the server dies (the guard); without it the job only lets
    /// [`WinJob::terminate`] end the whole tree on cancel.
    pub(crate) fn protect(process: HANDLE, kill_on_close: bool) -> std::io::Result<Self> {
        let job = Self::new(kill_on_close)?;
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    fn new(kill_on_close: bool) -> std::io::Result<Self> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = WinJob(job);
        if !kill_on_close {
            return Ok(job);
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            // Dropping `job` closes the handle.
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Terminate every process in the job — the process plus any descendants
    /// that inherited job membership.
    pub(crate) fn terminate(&self, exit_code: u32) {
        unsafe {
            TerminateJobObject(self.0, exit_code);
        }
    }
}

impl Drop for WinJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Resume every thread of process `pid`, which was created suspended (only its
/// primary thread exists). Fails when no thread could be resumed, so the
/// caller never leaves a process suspended forever.
pub(crate) fn resume_process(pid: u32) -> std::io::Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0usize;
    let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                if unsafe { ResumeThread(thread) } != u32::MAX {
                    resumed += 1;
                }
                unsafe {
                    CloseHandle(thread);
                }
            }
        }
        more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    if resumed == 0 {
        return Err(std::io::Error::other(format!(
            "no thread of process {pid} could be resumed"
        )));
    }
    Ok(())
}
