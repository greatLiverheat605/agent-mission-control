#![cfg(windows)]

use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use mission_protocol::credential::{INSTALL_SECRET_BYTES, WindowsCredentialInstallSecret};
use mission_protocol::handshake::InstallSecretProvider;
use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
use windows_sys::Win32::Security::Credentials::{
    CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
};

static TEST_TARGET_NONCE: AtomicU64 = AtomicU64::new(0);

struct CredentialCleanup(Vec<u16>);

impl Drop for CredentialCleanup {
    fn drop(&mut self) {
        unsafe { CredDeleteW(self.0.as_ptr(), CRED_TYPE_GENERIC, 0) };
    }
}

#[test]
fn install_secret_is_stable_and_exactly_32_bytes() {
    let target = format!(
        "Agent Mission Control/Test/{}/{}",
        std::process::id(),
        TEST_TARGET_NONCE.fetch_add(1, Ordering::Relaxed)
    );
    let target_wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let cleanup = CredentialCleanup(target_wide.clone());
    let provider = WindowsCredentialInstallSecret::for_test_target(target)
        .expect("accept unique non-secret test target");

    let first = provider
        .install_secret()
        .expect("read or create install secret");
    let second = provider
        .install_secret()
        .expect("read install secret again");

    assert_eq!(first.len(), INSTALL_SECRET_BYTES);
    assert_eq!(second, first);

    drop(cleanup);
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    assert_eq!(
        unsafe { CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) },
        0
    );
    assert!(credential.is_null());
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(ERROR_NOT_FOUND as i32)
    );
    if !credential.is_null() {
        unsafe { CredFree(credential.cast()) };
    }
}
