use std::collections::BTreeSet;
use std::io;
use std::process::{Child, Command};

#[derive(Debug, Default)]
pub struct OwnedProcessTree {
    root_pid: Option<u32>,
    pids: BTreeSet<u32>,
}

impl OwnedProcessTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_child(child: &Child) -> Self {
        let mut tree = Self::new();
        tree.register(child.id());
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
        let Some(root) = self.root_pid else {
            return Ok(());
        };
        #[cfg(windows)]
        {
            let status = Command::new("taskkill")
                .args(["/PID", &root.to_string(), "/T", "/F"])
                .status()?;
            if !status.success() {
                return Err(io::Error::other(format!("taskkill exited with {status}")));
            }
        }
        #[cfg(not(windows))]
        let _ = root;
        self.pids.clear();
        Ok(())
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
