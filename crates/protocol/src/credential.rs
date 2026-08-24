use std::io;
use std::ptr;
use std::slice;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_NOT_FOUND, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW, CredWriteW,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, INFINITE, ReleaseMutex, WaitForSingleObject,
};

use crate::handshake::InstallSecretProvider;

pub const INSTALL_SECRET_BYTES: usize = 32;
const CREDENTIAL_TARGET: &str = "Agent Mission Control/Install Secret/v1";
const SECRET_MUTEX_NAME: &str = "Global\\AgentMissionControlInstallSecretV1";

#[derive(Clone, Debug)]
pub struct WindowsCredentialInstallSecret {
    target: String,
}

impl Default for WindowsCredentialInstallSecret {
    fn default() -> Self {
        Self {
            target: CREDENTIAL_TARGET.to_owned(),
        }
    }
}

impl WindowsCredentialInstallSecret {
    #[cfg(debug_assertions)]
    pub fn for_test_target(target: impl Into<String>) -> io::Result<Self> {
        let target = target.into();
        if target.is_empty() || target.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "credential target is invalid",
            ));
        }
        Ok(Self { target })
    }
}

impl InstallSecretProvider for WindowsCredentialInstallSecret {
    fn install_secret(&self) -> io::Result<Vec<u8>> {
        let _lock = NamedMutexGuard::acquire(SECRET_MUTEX_NAME)?;
        let target = wide(&self.target);
        if let Some(secret) = read_secret(&target)? {
            return Ok(secret);
        }

        let mut secret = secure_random::<INSTALL_SECRET_BYTES>()?.to_vec();

        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_ptr().cast_mut(),
            CredentialBlobSize: secret.len() as u32,
            CredentialBlob: secret.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..CREDENTIALW::default()
        };
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(last_error("CredWriteW"));
        }
        Ok(secret)
    }
}

pub fn secure_random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            bytes.as_mut_ptr(),
            N as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(io::Error::other("BCryptGenRandom failed"));
    }
    Ok(bytes)
}

fn read_secret(target: &[u16]) -> io::Result<Option<Vec<u8>>> {
    let mut raw = ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
            return Ok(None);
        }
        return Err(io::Error::new(
            error.kind(),
            format!("CredReadW failed: {error}"),
        ));
    }

    let credential = CredentialGuard(raw);
    let credential_ref = unsafe { &*credential.0 };
    if credential_ref.CredentialBlobSize as usize != INSTALL_SECRET_BYTES
        || credential_ref.CredentialBlob.is_null()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stored install secret has an invalid size",
        ));
    }
    Ok(Some(
        unsafe {
            slice::from_raw_parts(
                credential_ref.CredentialBlob,
                credential_ref.CredentialBlobSize as usize,
            )
        }
        .to_vec(),
    ))
}

struct CredentialGuard(*mut CREDENTIALW);

impl Drop for CredentialGuard {
    fn drop(&mut self) {
        unsafe { CredFree(self.0.cast()) };
    }
}

struct NamedMutexGuard(HANDLE);

impl NamedMutexGuard {
    fn acquire(name: &str) -> io::Result<Self> {
        let name = wide(name);
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_error("CreateMutexW"));
        }
        let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
        if !matches!(wait, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            unsafe { CloseHandle(handle) };
            return Err(io::Error::other("WaitForSingleObject failed"));
        }
        Ok(Self(handle))
    }
}

impl Drop for NamedMutexGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn last_error(operation: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{operation} failed: {error}"))
}
