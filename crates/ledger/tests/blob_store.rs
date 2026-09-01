use std::fs;

use mission_ledger::{
    BlobStoreError, EncryptedBlobStore, EncryptedLedger, InMemoryKeyStore, KeyStore,
};

#[test]
fn encrypted_blob_store_deduplicates_verifies_and_tracks_references() {
    let root = std::env::temp_dir().join(format!("mission-blobs-{}", uuid::Uuid::new_v4()));
    let payload = vec![b'd'; 2 * 1024 * 1024];
    let store = EncryptedBlobStore::open(&root, [3_u8; 32]).expect("open blob store");
    let first = store.put(&payload, "text/x-diff").expect("write blob");
    let second = store.put(&payload, "text/x-diff").expect("dedupe blob");
    assert_eq!(first, second);
    let files = fs::read_dir(root.join(&first.hash[..2]).join(&first.hash[2..4]))
        .expect("read shard")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "blob"))
        .count();
    assert_eq!(files, 1);
    assert_eq!(store.read(&first).expect("read blob"), payload);
    store.retain(&first).expect("retain first");
    store.retain(&first).expect("retain second");
    assert!(
        !store
            .delete_if_unreferenced(&first)
            .expect("keep referenced")
    );
    assert!(!store.release(&first).expect("release first"));
    assert!(store.release(&first).expect("release second"));
    assert!(
        store
            .delete_if_unreferenced(&first)
            .expect("delete after zero")
    );
    assert!(!store.path_for(&first).expect("path").exists());
    drop(store);
    fs::remove_dir_all(root).expect("remove temp root");
}

#[test]
fn corrupted_blob_fails_closed_without_partial_plaintext() {
    let root = std::env::temp_dir().join(format!("mission-blobs-{}", uuid::Uuid::new_v4()));
    let store = EncryptedBlobStore::open(&root, [4_u8; 32]).expect("open blob store");
    let reference = store
        .put(b"sensitive diff", "text/plain")
        .expect("write blob");
    let path = store.path_for(&reference).expect("blob path");
    let mut bytes = fs::read(&path).expect("read ciphertext");
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, bytes).expect("corrupt ciphertext");
    assert!(matches!(
        store.read(&reference),
        Err(BlobStoreError::IntegrityFailed)
    ));
    drop(store);
    fs::remove_dir_all(root).expect("remove temp root");
}

#[test]
fn ledger_backed_blob_store_uses_the_ledger_metadata_database() {
    let root = std::env::temp_dir().join(format!("mission-ledger-blobs-{}", uuid::Uuid::new_v4()));
    let ledger_path = root.join("ledger.db");
    let blob_root = root.join("blobs");
    let key_store = InMemoryKeyStore::default();
    let ledger =
        EncryptedLedger::open(&ledger_path, "install", key_store.clone()).expect("open ledger");
    let key = key_store
        .load_database_key("install")
        .expect("load database key");
    let store = EncryptedBlobStore::open_for_ledger(&blob_root, &ledger_path, key)
        .expect("open ledger-backed blob store");
    let reference = store.put(b"linked", "text/plain").expect("write blob");
    store.retain(&reference).expect("retain blob");
    assert!(!blob_root.join("metadata.db").exists());
    drop(store);
    drop(ledger);
    fs::remove_dir_all(root).expect("remove temp root");
}

#[test]
fn recovery_report_detects_incomplete_temp_writes_without_deleting_them() {
    let root =
        std::env::temp_dir().join(format!("mission-blobs-recovery-{}", uuid::Uuid::new_v4()));
    let store = EncryptedBlobStore::open(&root, [8_u8; 32]).expect("open blob store");
    let temp = root.join("aa").join("bb");
    fs::create_dir_all(&temp).expect("create shard");
    let temp_file = temp.join(".partial.tmp");
    fs::write(&temp_file, b"partial").expect("write temp");
    let report = store.recovery_report().expect("recovery report");
    assert!(report.recovery_required);
    assert_eq!(report.temporary_files.len(), 1);
    assert!(temp_file.exists());
    drop(store);
    fs::remove_dir_all(root).expect("remove temp root");
}
