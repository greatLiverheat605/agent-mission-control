use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use thiserror::Error;

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::ptr;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError, LocalFree};
#[cfg(windows)]
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW, CredWriteW,
};
#[cfg(windows)]
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

#[derive(Debug, Error)]
pub enum KeyStoreError {
    #[error("key store unavailable")]
    Unavailable,
    #[error("stored database key has invalid length")]
    InvalidKey,
}

pub trait KeyStore: Send + Sync + Clone + 'static {
    fn load_or_create_database_key(&self, install_id: &str) -> Result<[u8; 32], KeyStoreError>;
    fn load_database_key(&self, install_id: &str) -> Result<[u8; 32], KeyStoreError>;
}

#[derive(Clone, Default)]
pub struct InMemoryKeyStore {
    keys: Arc<Mutex<HashMap<String, [u8; 32]>>>,
}

impl InMemoryKeyStore {
    pub fn insert(&self, install_id: impl Into<String>, key: [u8; 32]) {
        self.keys
            .lock()
            .expect("key store mutex poisoned")
            .insert(install_id.into(), key);
    }
}

impl KeyStore for InMemoryKeyStore {
    fn load_or_create_database_key(&self, install_id: &str) -> Result<[u8; 32], KeyStoreError> {
        let mut keys = self.keys.lock().map_err(|_| KeyStoreError::Unavailable)?;
        if let Some(key) = keys.get(install_id) {
            return Ok(*key);
        }
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).map_err(|_| KeyStoreError::Unavailable)?;
        keys.insert(install_id.to_owned(), key);
        Ok(key)
    }

    fn load_database_key(&self, install_id: &str) -> Result<[u8; 32], KeyStoreError> {
        self.keys
            .lock()
            .map_err(|_| KeyStoreError::Unavailable)?
            .get(install_id)
            .copied()
            .ok_or(KeyStoreError::Unavailable)
    }
}

#[cfg(windows)]
#[derive(Clone, Default)]
pub struct WindowsCredentialKeyStore;

#[cfg(windows)]
impl KeyStore for WindowsCredentialKeyStore {
    fn load_or_create_database_key(&self, install_id: &str) -> Result<[u8; 32], KeyStoreError> {
        match self.load_database_key(install_id) {
            Ok(key) => Ok(key),
            Err(KeyStoreError::Unavailable) if unsafe { GetLastError() } == ERROR_NOT_FOUND => {
                let mut key = [0_u8; 32];
                getrandom::fill(&mut key).map_err(|_| KeyStoreError::Unavailable)?;
                self.save_database_key(install_id, &key)?;
                Ok(key)
            }
            Err(error) => Err(error),
        }
    }

    fn load_database_key(&self, install_id: &str) -> Result<[u8; 32], KeyStoreError> {
        let target = target_name(install_id);
        let mut credential = ptr::null_mut();
        let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if ok == 0 {
            return Err(KeyStoreError::Unavailable);
        }
        let encrypted = unsafe {
            let value = &*credential;
            std::slice::from_raw_parts(value.CredentialBlob, value.CredentialBlobSize as usize)
                .to_vec()
        };
        unsafe { CredFree(credential.cast()) };
        unprotect(&encrypted)
    }
}

#[cfg(windows)]
impl WindowsCredentialKeyStore {
    fn save_database_key(&self, install_id: &str, key: &[u8; 32]) -> Result<(), KeyStoreError> {
        let protected = protect(key)?;
        let mut target = target_name(install_id);
        let mut blob = protected;
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        let ok = unsafe { CredWriteW(&credential, 0) };
        if ok == 0 {
            return Err(KeyStoreError::Unavailable);
        }
        Ok(())
    }
}

#[cfg(windows)]
fn target_name(install_id: &str) -> Vec<u16> {
    format!("MissionControl/DatabaseKey/{install_id}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn protect(key: &[u8; 32]) -> Result<Vec<u8>, KeyStoreError> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: key.len() as u32,
        pbData: key.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(KeyStoreError::Unavailable);
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData.cast::<c_void>()) };
    Ok(result)
}

#[cfg(windows)]
fn unprotect(encrypted: &[u8]) -> Result<[u8; 32], KeyStoreError> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 || output.cbData != 32 {
        if !output.pbData.is_null() {
            unsafe { LocalFree(output.pbData.cast::<c_void>()) };
        }
        return Err(KeyStoreError::InvalidKey);
    }
    let mut key = [0_u8; 32];
    unsafe {
        key.copy_from_slice(std::slice::from_raw_parts(output.pbData, 32));
        LocalFree(output.pbData.cast::<c_void>());
    }
    Ok(key)
}
