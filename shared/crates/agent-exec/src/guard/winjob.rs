//! Windows process-orphaning protection via Job Objects.
//!
//! Unlike Linux/macOS (see `pdeath.rs`), Windows needs no guard process:
//! `AssignProcessToJobObject` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` makes the
//! **kernel** the reactor — job membership propagates to children
//! automatically, and the whole job is torn down when the job handle's last
//! reference closes, with no process needing to stay alive to react. That
//! includes the server dying: its handles close with it.
//!
//! Entry point: [`WinJob::protect`], called right after a spawn; the returned
//! job is owned by `guard::ProcessTree`, which terminates it on cancel and
//! drops (closes) it when the run ends.

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

/// A kill-on-close Job Object. Dropping it closes the handle, which kills
/// every process still in the job. Job Object handles are usable from any
/// thread (the Win32 API has no thread affinity for them), so this is `Send`
/// and `Sync`; the owner never shares it mutably.
pub(crate) struct WinJob(HANDLE);

unsafe impl Send for WinJob {}
unsafe impl Sync for WinJob {}

impl WinJob {
    /// Create a kill-on-close Job Object and assign the just-spawned process
    /// (`process`, from `Child::raw_handle()`) to it.
    pub(crate) fn protect(process: HANDLE) -> std::io::Result<Self> {
        let job = Self::new_kill_on_close()?;
        if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    fn new_kill_on_close() -> std::io::Result<Self> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(job);
            }
            return Err(err);
        }
        Ok(WinJob(job))
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
