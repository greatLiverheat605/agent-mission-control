use std::collections::BTreeSet;
use std::io;
use std::process::Child;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

#[derive(Debug, Default)]
pub struct OwnedProcessTree {
    root_pid: Option<u32>,
    pids: BTreeSet<u32>,
    #[cfg(windows)]
    job: Option<HANDLE>,
}

// Job handles are kernel objects with no thread affinity. Ownership is guarded by the
// MissionService mutex, so transferring the owning actor between command workers is safe.
unsafe impl Send for OwnedProcessTree {}
unsafe impl Sync for OwnedProcessTree {}

impl OwnedProcessTree {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            let job = (!job.is_null()).then_some(job);
            if let Some(job_handle) = job {
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let configured = unsafe {
                    SetInformationJobObject(
                        job_handle,
                        JobObjectExtendedLimitInformation,
                        (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    )
                } != 0;
                if !configured {
                    unsafe { CloseHandle(job_handle) };
                    return Self {
                        root_pid: None,
                        pids: BTreeSet::new(),
                        job: None,
                    };
                }
            }
            Self {
                root_pid: None,
                pids: BTreeSet::new(),
                job,
            }
        }
        #[cfg(not(windows))]
        {
            Self::default()
        }
    }

    pub fn from_child(child: &Child) -> Self {
        let mut tree = Self::new();
        tree.register(child.id());
        #[cfg(windows)]
        if let Some(job) = tree.job {
            let assigned =
                unsafe { AssignProcessToJobObject(job, child.as_raw_handle().cast()) } != 0;
            if !assigned {
                tree.job = None;
                unsafe { CloseHandle(job) };
            }
        }
        tree
    }

    pub fn register(&mut self, pid: u32) {
        if self.root_pid.is_none() {
            self.root_pid = Some(pid);
        }
        if pid != 0 {
            self.pids.insert(pid);
        }
    }

    pub fn root_pid(&self) -> Option<u32> {
        self.root_pid
    }
    pub fn owns(&self, pid: u32) -> bool {
        self.pids.contains(&pid)
    }
    pub fn pids(&self) -> impl Iterator<Item = u32> + '_ {
        self.pids.iter().copied()
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        let Some(_root) = self.root_pid else {
            return Ok(());
        };
        #[cfg(windows)]
        if let Some(job) = self.job {
            if unsafe { TerminateJobObject(job, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
        } else {
            return Err(io::Error::other("owned process job object is unavailable"));
        }
        self.pids.clear();
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for OwnedProcessTree {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            unsafe { CloseHandle(job) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OwnedProcessTree;

    #[test]
    fn only_registered_processes_are_owned() {
        let mut tree = OwnedProcessTree::new();
        tree.register(101);
        tree.register(102);
        assert_eq!(tree.root_pid(), Some(101));
        assert!(tree.owns(102));
        assert!(!tree.owns(103));
    }
}
