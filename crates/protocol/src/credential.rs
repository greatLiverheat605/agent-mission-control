use std::io;
use std::ptr;
use std::slice;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_NOT_FOUND, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW, CredWriteW,
};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

use crate::handshake::InstallSecretProvider;
use crate::windows_security::{
    SecurityAttributes, current_user_sid, opaque_user_id, user_system_admin_sddl,
};

pub const INSTALL_SECRET_BYTES: usize = 32;
const CREDENTIAL_TARGET: &str = "Agent Mission Control/Install Secret/v1";
const SECRET_MUTEX_TIMEOUT: Duration = Duration::from_secs(2);

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
        let sid = current_user_sid()?;
        let mutex_name = credential_mutex_name(&sid);
        let _lock = NamedMutexGuard::acquire(&mutex_name, &sid, SECRET_MUTEX_TIMEOUT)?;
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
    fn acquire(name: &str, sid: &str, timeout: Duration) -> io::Result<Self> {
        let name = wide(name);
        let security = SecurityAttributes::from_sddl(&user_system_admin_sddl(sid))?;
        let handle = unsafe { CreateMutexW(security.as_ptr(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_error("CreateMutexW"));
        }
        let timeout_millis = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
        let wait = unsafe { WaitForSingleObject(handle, timeout_millis) };
        if wait == WAIT_TIMEOUT {
            unsafe { CloseHandle(handle) };
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "credential mutex wait timed out",
            ));
        }
        if !matches!(wait, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            let error = last_error("WaitForSingleObject");
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        Ok(Self(handle))
    }
}

fn credential_mutex_name(sid: &str) -> String {
    format!(
        "Global\\AgentMissionControlInstallSecretV1-{}",
        opaque_user_id(sid)
    )
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

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::windows_security::current_user_sid;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};

    use super::{NamedMutexGuard, credential_mutex_name};

    #[test]
    fn credential_mutex_name_is_a_global_stable_opaque_user_identity() {
        let sid = "S-1-5-21-111-222-333-1001";

        let first = credential_mutex_name(sid);
        let second = credential_mutex_name(sid);

        assert_eq!(first, second);
        assert!(first.starts_with("Global\\AgentMissionControlInstallSecretV1-"));
        assert!(!first.contains(sid));
    }

    #[test]
    fn credential_mutex_wait_times_out_fail_closed() {
        let sid = current_user_sid().expect("read test user SID");
        let name = format!(
            "Global\\AgentMissionControlInstallSecretTimeoutTest-{}-{}",
            std::process::id(),
            thread::current().name().unwrap_or("unnamed")
        );
        let holder_name = name.clone();
        let holder_sid = sid.clone();
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let holder = thread::spawn(move || {
            let _guard =
                NamedMutexGuard::acquire(&holder_name, &holder_sid, Duration::from_secs(1))
                    .expect("holder acquires fixture mutex");
            held_tx.send(()).expect("signal held fixture mutex");
            release_rx.recv().expect("release fixture mutex");
        });
        held_rx.recv().expect("wait for held fixture mutex");

        let started = Instant::now();
        let error = match NamedMutexGuard::acquire(&name, &sid, Duration::from_millis(50)) {
            Ok(_) => panic!("contended credential mutex must not be acquired"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(500));
        release_tx.send(()).expect("release held fixture mutex");
        holder.join().expect("join fixture mutex holder");
    }

    #[test]
    fn abandoned_credential_mutex_is_recovered() {
        let sid = current_user_sid().expect("read test user SID");
        let name = format!(
            "Global\\AgentMissionControlInstallSecretAbandonedTest-{}",
            std::process::id()
        );
        let holder_name = name.clone();
        let holder_sid = sid.clone();
        let holder = thread::spawn(move || {
            let guard = NamedMutexGuard::acquire(&holder_name, &holder_sid, Duration::from_secs(1))
                .expect("holder acquires fixture mutex");
            let handle = guard.0 as usize;
            std::mem::forget(guard);
            handle
        });
        let abandoned_handle = holder.join().expect("fixture mutex owner exits");

        let recovered = NamedMutexGuard::acquire(&name, &sid, Duration::from_secs(1))
            .expect("abandoned credential mutex is recoverable");
        drop(recovered);
        unsafe { CloseHandle(abandoned_handle as HANDLE) };
    }
}
