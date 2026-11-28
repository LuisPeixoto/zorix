//! A Windows Job Object that holds a stdio server and everything it starts.
//!
//! On Unix a server leads its own process group and stopping it signals the group. Windows has no such thing for a process tree: the
//! real program behind `npx.cmd` is a child of `cmd.exe`, and ending the child ZORIX started leaves it running. A job collects the
//! whole tree, and closing the job (or ZORIX ending, however it ends) ends every process in it.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::RawHandle;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

/// A job whose processes end when it is closed.
pub struct Job(HANDLE);

// SAFETY: a job handle is a plain kernel handle, usable from any thread, and closed once, in `Drop`.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// A new job that kills what is in it when its last handle closes. A confined server's job also limits how many processes it may
    /// have at once and how much memory each may hold: Windows confines no more than that natively (ADR-0050).
    pub fn new(confined: bool) -> io::Result<Job> {
        // SAFETY: both arguments may be null; the result is checked before use.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the structure is plain data, and all-zero is a valid "no limits" value.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if confined {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            info.BasicLimitInformation.ActiveProcessLimit = 64;
            info.ProcessMemoryLimit = 2 * 1024 * 1024 * 1024;
        }
        // SAFETY: `info` is a live, correctly sized structure of the class named, and `handle` is the job just made.
        let set = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast::<c_void>(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: the handle was made above and is closed once here.
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        Ok(Job(handle))
    }

    /// Put a process in the job. Everything it starts afterwards is in it too.
    pub fn add(&self, process: RawHandle) -> io::Result<()> {
        // SAFETY: both handles are valid for the duration of the call.
        if unsafe { AssignProcessToJobObject(self.0, process as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// End every process in the job now.
    pub fn terminate(&self) {
        // SAFETY: the job handle is valid until `Drop`.
        unsafe { TerminateJobObject(self.0, 1) };
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is closed once; the limit flag ends what is left in the job.
        unsafe { CloseHandle(self.0) };
    }
}
