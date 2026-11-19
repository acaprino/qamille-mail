//! Processi: se il padre di un figlio è ancora vivo.

/// Unix: a child whose parent died is adopted by another process.
#[cfg(unix)]
pub fn parent_alive(parent: u32) -> bool {
    std::os::unix::process::parent_id() == parent
}

#[cfg(windows)]
pub fn parent_alive(parent: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, parent);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(any(unix, windows)))]
pub fn parent_alive(_parent: u32) -> bool {
    true
}

// --- The child of the built-in LLM and everything it starts ------------------------

use std::process::{Child, Command};

/// A child process and its descendants as one unit that dies with this server (INV-40): on
/// Windows a job object with `KILL_ON_JOB_CLOSE` (closed by the system when the server ends, even
/// if it is killed); on Unix a process group of its own and, on Linux, `PR_SET_PDEATHSIG`.
/// `spawn` must be called from a thread that lives as long as the child: on Linux the death signal
/// is tied to the thread that forked.
pub struct ChildGroup {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    pgid: i32,
}

// SAFETY: the job handle is only used through the thread-safe job object API.
#[cfg(windows)]
unsafe impl Send for ChildGroup {}
#[cfg(windows)]
unsafe impl Sync for ChildGroup {}

impl ChildGroup {
    #[cfg(windows)]
    pub fn spawn(command: &mut Command) -> std::io::Result<(Child, ChildGroup)> {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        };
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // SAFETY: plain calls with valid arguments; the handle is closed on every error path.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }
            command.creation_flags(CREATE_NO_WINDOW);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    CloseHandle(job);
                    return Err(error);
                }
            };
            if AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
                // Without the job the child could outlive the server: it does not run.
                let error = std::io::Error::last_os_error();
                let _ = child.kill();
                let _ = child.wait();
                CloseHandle(job);
                return Err(error);
            }
            Ok((child, ChildGroup { job }))
        }
    }

    #[cfg(unix)]
    pub fn spawn(command: &mut Command) -> std::io::Result<(Child, ChildGroup)> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        #[cfg(target_os = "linux")]
        {
            // SAFETY: `getpid` has no preconditions.
            let parent = unsafe { libc::getpid() };
            // SAFETY: between fork and exec the closure only calls `prctl` and `getppid`, which
            // are async-signal-safe. The child gets SIGKILL when the thread that forked ends, and
            // gives up if the parent was already gone before the call took effect.
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != parent {
                        return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
                    }
                    Ok(())
                });
            }
        }
        let child = command.spawn()?;
        let pgid = child.id() as i32;
        Ok((child, ChildGroup { pgid }))
    }

    /// The child and its descendants are killed; it does not wait for them.
    pub fn kill_all(&self) {
        #[cfg(windows)]
        // SAFETY: `self.job` is a job handle owned by this value.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
        #[cfg(unix)]
        // SAFETY: `killpg` only sends a signal to the group this value created.
        unsafe {
            libc::killpg(self.pgid, libc::SIGKILL);
        }
    }

    /// The job object, for the tests that check its limits.
    #[cfg(windows)]
    pub fn job_handle(&self) -> isize {
        self.job as isize
    }
}

impl Drop for ChildGroup {
    fn drop(&mut self) {
        self.kill_all();
        #[cfg(windows)]
        // SAFETY: the handle is closed once, here.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}
