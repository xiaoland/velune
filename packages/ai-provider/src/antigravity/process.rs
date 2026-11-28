//! Keep the ACP server and its per-session worker in the same ownership scope.
use std::process::{Command, Stdio};
use tokio::process::Child;
use velune_ai::NativeErrorKind as Error;

pub(super) struct ChildTree {
    _tree: platform::Tree,
    pub child: Child,
}

impl ChildTree {
    pub fn spawn(mut command: Command) -> Result<Self, Error> {
        platform::prepare(&mut command);
        let child = tokio::process::Command::from(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Error::Transport)?;
        let tree = platform::Tree::containing(&child).ok_or(Error::Transport)?;
        // Field order kills the tree before Child can reap and release the PID.
        Ok(Self { _tree: tree, child })
    }
}

#[cfg(unix)]
mod platform {
    use std::{os::unix::process::CommandExt, process::Command};
    use tokio::process::Child;

    pub(super) fn prepare(command: &mut Command) {
        command.process_group(0);
    }

    pub(super) struct Tree(rustix::process::Pid);

    impl Tree {
        pub(super) fn containing(child: &Child) -> Option<Self> {
            let pid = i32::try_from(child.id()?).ok()?;
            rustix::process::Pid::from_raw(pid).map(Self)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = rustix::process::kill_process_group(self.0, rustix::process::Signal::KILL);
        }
    }
}

#[cfg(windows)]
mod platform {
    #![allow(unsafe_code)]
    use std::{os::windows::process::CommandExt, process::Command, ptr};
    use tokio::process::Child;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
    };

    pub(super) fn prepare(command: &mut Command) {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    pub(super) struct Tree(HANDLE);

    // SAFETY: a Job Object handle may be closed from any thread.
    unsafe impl Send for Tree {}

    impl Tree {
        pub(super) fn containing(child: &Child) -> Option<Self> {
            let process = child.raw_handle()?;
            // SAFETY: a null security descriptor and null name are valid.
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let tree = Self(handle);
            // SAFETY: an all-zero value is valid for this plain C struct.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: the pointer and size describe the living C struct;
            // both handles are open for the duration of the calls.
            let contained = unsafe {
                SetInformationJobObject(
                    tree.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) != 0
                    && AssignProcessToJobObject(tree.0, process) != 0
            };
            contained.then_some(tree)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            // SAFETY: the job handle is owned here. Its last close kills members.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::ChildTree;
    use std::{process::Command, time::Duration};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    #[tokio::test]
    async fn dropping_child_tree_closes_worker_stdout() {
        #[allow(clippy::disallowed_methods)]
        let mut command = Command::new("/bin/sh");
        // The worker inherits stdout and would keep it open if only the shell
        // itself were killed.
        command.args(["-c", "/bin/sleep 30 & printf 'started\\n'; wait"]);
        let mut tree = ChildTree::spawn(command).unwrap();
        let mut stdout = BufReader::new(tree.child.stdout.take().unwrap());
        let mut ready = String::new();
        tokio::time::timeout(Duration::from_secs(2), stdout.read_line(&mut ready))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready, "started\n");

        drop(tree);
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), stdout.read_to_end(&mut rest))
            .await
            .expect("ACP worker survived process tree cancellation")
            .unwrap();
    }
}
