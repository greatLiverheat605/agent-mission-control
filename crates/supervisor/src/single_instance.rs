use std::io;
use std::ptr;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::Security::{
    GetSidIdentifierAuthority, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    IsValidSid, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, OpenProcessToken, ReleaseMutex,
};

pub enum AcquireResult {
    Acquired(SingleInstance),
    AlreadyRunning,
}

pub struct SingleInstance {
    handle: HANDLE,
}

pub fn production_pipe_name(sid: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(sid.as_bytes()));
    format!("mission-control-{}", &digest[..16])
}

pub fn current_user_sid() -> io::Result<String> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("OpenProcessToken"));
    }

    let result = user_sid_from_token(token);
    unsafe { CloseHandle(token) };
    result
}

fn user_sid_from_token(token: HANDLE) -> io::Result<String> {
    let mut required_bytes = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required_bytes);
    }
    if required_bytes == 0 {
        return Err(last_error("GetTokenInformation(size)"));
    }

    let mut buffer = vec![0_u8; required_bytes as usize];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required_bytes,
            &mut required_bytes,
        )
    } == 0
    {
        return Err(last_error("GetTokenInformation(user)"));
    }

    let token_user = unsafe { ptr::read_unaligned(buffer.as_ptr().cast::<TOKEN_USER>()) };
    sid_to_string(token_user.User.Sid)
}

fn sid_to_string(sid: windows_sys::Win32::Security::PSID) -> io::Result<String> {
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "current process token contains an invalid user SID",
        ));
    }

    let revision = unsafe { *sid.cast::<u8>() };
    let authority = unsafe { (*GetSidIdentifierAuthority(sid)).Value }
        .into_iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(byte));
    let count = unsafe { *GetSidSubAuthorityCount(sid) };
    let mut value = format!("S-{revision}-{authority}");
    for index in 0..u32::from(count) {
        value.push_str(&format!("-{}", unsafe { *GetSidSubAuthority(sid, index) }));
    }
    Ok(value)
}

fn last_error(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{operation} failed: {error}"))
}

fn mutex_name(sid: &str, pipe_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sid.as_bytes());
    hasher.update([0]);
    hasher.update(pipe_name.as_bytes());
    format!("Local\\MissionControlSupervisor-{:x}", hasher.finalize())
}

impl SingleInstance {
    pub fn acquire(pipe_name: &str, sid: &str) -> io::Result<AcquireResult> {
        let name = mutex_name(sid, pipe_name);
        let wide_name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateMutexW(ptr::null(), 1, wide_name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
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
    use super::production_pipe_name;

    #[test]
    fn production_pipe_name_is_a_stable_opaque_sid_digest() {
        let sid = "S-1-5-21-111-222-333-1001";

        let pipe_name = production_pipe_name(sid);

        assert_eq!(pipe_name, "mission-control-4c51f3baadf41ed2");
        assert!(!pipe_name.contains(sid));
    }
}
