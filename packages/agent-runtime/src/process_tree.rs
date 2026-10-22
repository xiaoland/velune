//! Child processes that stop together with everything they started.
//!
//! Unix places the child in its own process group and kills the group.
//! Windows assigns the child to a Job Object that kills its members when it is
//! terminated or closed. The assignment happens right after the child is
//! created, so a grandchild started before it would escape; Harness entry
//! points do not start one that early.
use std::{
    io,
    ops::{Deref, DerefMut},
    process::{Child, Command},
};

/// A command that does not open a console window when the host is a GUI app.
/// Every child process starts here; clippy refuses `Command::new` elsewhere.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(clippy::disallowed_methods)]
    let mut command = Command::new(program);
    hide_console(&mut command);
    command
}

pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

pub(crate) struct ProcessTree {
    child: Child,
    #[cfg(windows)]
    job: windows::Job,
}

impl ProcessTree {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        hide_console(command);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn()?;
        #[cfg(windows)]
        {
            let job = match windows::Job::containing(&child) {
                Ok(job) => job,
                Err(error) => {
                    let mut child = child;
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            };
            Ok(Self { child, job })
        }
        #[cfg(not(windows))]
        Ok(Self { child })
    }

    /// Kill the child and its descendants, then reap the child.
    pub(crate) fn stop(&mut self) {
        // A reaped child's process group id may already belong to another process.
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            #[cfg(windows)]
            self.job.terminate();
            return;
        }
        #[cfg(unix)]
        // SAFETY: the child has not been reaped, so its group id is still ours.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Deref for ProcessTree {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}

impl DerefMut for ProcessTree {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

#[cfg(windows)]
mod windows {
    #![allow(unsafe_code)]
    use std::{io, os::windows::io::AsRawHandle, process::Child, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
    };

    pub(super) struct Job(HANDLE);

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub(super) fn containing(child: &Child) -> io::Result<Self> {
            // SAFETY: both pointers may be null: no name, default security.
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            // SAFETY: an all-zero value is valid for this plain C struct.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: the pointer and size describe `limits`, which outlives the call.
            let configured = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the child handle stays open while `child` is borrowed.
            if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        pub(super) fn terminate(&self) {
            // SAFETY: the handle is open until drop.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is open and closed exactly once; closing the last
            // handle kills any remaining members.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
