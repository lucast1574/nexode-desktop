//! Keep the private engine's descendants bound to the native window process.
//! The SDK currently stops detached daemons explicitly only on Unix.
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::{JobObjects::*, Threading::GetCurrentProcess},
};

pub fn contain_child_processes() -> std::io::Result<()> {
    // SAFETY: null security/name pointers request an unnamed, non-inheritable
    // handle. All pointers below refer to live, correctly sized Windows structs.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        ) == 0
            || AssignProcessToJobObject(job, GetCurrentProcess()) == 0
        {
            let error = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(error);
        }
        // Intentionally retain the sole handle until OS process teardown. A
        // Rust Drop would terminate this process before its own cleanup ran.
        // Child membership is inherited; the handle itself is not inherited.
    }
    Ok(())
}
