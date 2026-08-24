use std::io;
use std::mem::MaybeUninit;
use std::ptr;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetSidIdentifierAuthority, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    IsValidSid, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
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

    let mut buffer = aligned_token_buffer(required_bytes);
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

    let token_user = unsafe { buffer.as_ptr().cast::<TOKEN_USER>().read() };
    let sid = sid_to_string(token_user.User.Sid);
    drop(buffer);
    sid
}

fn aligned_token_buffer(required_bytes: u32) -> Vec<MaybeUninit<usize>> {
    let words = (required_bytes as usize).div_ceil(size_of::<usize>());
    vec![MaybeUninit::uninit(); words]
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

fn mutex_name(sid: &str) -> String {
    format!(
        "Global\\MissionControlSupervisor-{:x}",
        Sha256::digest(sid.as_bytes())
    )
}

struct MutexSecurity {
    attributes: SECURITY_ATTRIBUTES,
    descriptor: PSECURITY_DESCRIPTOR,
}

impl MutexSecurity {
    fn for_user(sid: &str) -> io::Result<Self> {
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
        let wide_sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide_sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error(
                "ConvertStringSecurityDescriptorToSecurityDescriptorW",
            ));
        }

        Ok(Self {
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
            descriptor,
        })
    }
}

impl Drop for MutexSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

impl SingleInstance {
    pub fn acquire(sid: &str) -> io::Result<AcquireResult> {
        let name = mutex_name(sid);
        let wide_name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let security = MutexSecurity::for_user(sid)?;
        let handle = unsafe { CreateMutexW(&security.attributes, 1, wide_name.as_ptr()) };
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
    use std::mem::{align_of, size_of};

    use windows_sys::Win32::Security::TOKEN_USER;

    use super::{aligned_token_buffer, mutex_name, production_pipe_name};

    #[test]
    fn token_buffer_covers_requested_bytes_with_token_user_alignment() {
        let buffer = aligned_token_buffer(17);

        assert!(buffer.len() * size_of::<usize>() >= 17);
        assert_eq!(buffer.as_ptr().align_offset(align_of::<TOKEN_USER>()), 0);
    }

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
