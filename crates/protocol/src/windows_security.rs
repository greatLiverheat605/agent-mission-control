use std::io;
use std::mem::MaybeUninit;
use std::ptr;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetSidIdentifierAuthority, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    IsValidSid, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub fn opaque_user_id(sid: &str) -> String {
    format!("{:x}", Sha256::digest(sid.as_bytes()))
}

pub fn user_system_admin_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})")
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
    sid_to_string(token_user.User.Sid)
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

pub struct SecurityAttributes {
    attributes: SECURITY_ATTRIBUTES,
    descriptor: PSECURITY_DESCRIPTOR,
}

impl SecurityAttributes {
    pub fn from_sddl(sddl: &str) -> io::Result<Self> {
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

    pub fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        &self.attributes
    }
}

impl Drop for SecurityAttributes {
    fn drop(&mut self) {
        unsafe { LocalFree(self.descriptor) };
    }
}

fn last_error(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{operation} failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use windows_sys::Win32::Security::TOKEN_USER;

    use super::aligned_token_buffer;

    #[test]
    fn token_buffer_covers_requested_bytes_with_token_user_alignment() {
        let buffer = aligned_token_buffer(17);

        assert!(buffer.len() * size_of::<usize>() >= 17);
        assert_eq!(buffer.as_ptr().align_offset(align_of::<TOKEN_USER>()), 0);
    }
}
