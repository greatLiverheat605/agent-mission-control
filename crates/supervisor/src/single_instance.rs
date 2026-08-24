use std::io;

pub use mission_protocol::windows_security::current_user_sid;
use mission_protocol::windows_security::{
    SecurityAttributes, opaque_user_id, user_system_admin_sddl,
};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

pub enum AcquireResult {
    Acquired(SingleInstance),
    AlreadyRunning,
}

pub struct SingleInstance {
    handle: HANDLE,
}

pub fn production_pipe_name(sid: &str) -> String {
    let digest = opaque_user_id(sid);
    format!("mission-control-{}", &digest[..16])
}

fn mutex_name(sid: &str) -> String {
    format!("Global\\MissionControlSupervisor-{}", opaque_user_id(sid))
}

impl SingleInstance {
    pub fn acquire(sid: &str) -> io::Result<AcquireResult> {
        let name = mutex_name(sid);
        let wide_name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let security = SecurityAttributes::from_sddl(&user_system_admin_sddl(sid))?;
        let handle = unsafe { CreateMutexW(security.as_ptr(), 1, wide_name.as_ptr()) };
        let create_error = unsafe { GetLastError() };
        if handle.is_null() {
            return Err(io::Error::from_raw_os_error(create_error as i32));
        }

        if create_error == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return Ok(AcquireResult::AlreadyRunning);
        }

        Ok(AcquireResult::Acquired(Self { handle }))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mutex_name, production_pipe_name};

    #[test]
    fn production_pipe_name_is_a_stable_opaque_sid_digest() {
        let sid = "S-1-5-21-111-222-333-1001";

        let pipe_name = production_pipe_name(sid);

        assert_eq!(pipe_name, "mission-control-4c51f3baadf41ed2");
        assert!(!pipe_name.contains(sid));
    }

    #[test]
    fn mutex_name_is_a_global_stable_opaque_user_identity() {
        let sid = "S-1-5-21-111-222-333-1001";

        let first = mutex_name(sid);
        let second = mutex_name(sid);

        assert_eq!(first, second);
        assert!(first.starts_with("Global\\MissionControlSupervisor-"));
        assert!(!first.contains(sid));
    }
}
