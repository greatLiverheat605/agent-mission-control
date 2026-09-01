use std::fs;

use mission_ledger::EncryptedBlobStore;
use mission_workspace::{NonGitSnapshotter, SnapshotOptions};

#[test]
fn non_git_snapshot_is_encrypted_deduplicated_scoped_and_restored_to_temp() {
    let temp = tempfile::tempdir().expect("temp");
    let source = temp.path().join("source");
    let blobs = temp.path().join("blobs");
    let restore_parent = temp.path().join("restore");
    fs::create_dir_all(source.join("nested")).expect("source");
    fs::create_dir_all(source.join("secret")).expect("secret");
    fs::create_dir_all(source.join("node_modules/pkg")).expect("ignored");
    fs::write(source.join("a.txt"), b"same plaintext").expect("a");
    fs::write(source.join("nested/b.txt"), b"same plaintext").expect("b");
    fs::write(source.join("secret/key.txt"), b"forbidden").expect("secret file");
    fs::write(source.join("node_modules/pkg/index.js"), b"ignored").expect("ignored file");

    let outside = temp.path().join("outside.txt");
    fs::write(&outside, b"outside").expect("outside");
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, source.join("escape.txt")).is_ok();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, source.join("escape.txt")).is_ok();

    let store = EncryptedBlobStore::open(&blobs, [7_u8; 32]).expect("store");
    let snapshotter = NonGitSnapshotter::new(&store);
    let snapshot = snapshotter
        .create(
            &source,
            "snapshot-001",
            SnapshotOptions {
                forbidden_scopes: vec!["secret".into()],
            },
        )
        .expect("snapshot");

    assert_eq!(snapshot.manifest.entries.len(), 2);
    assert_eq!(
        snapshot.manifest.entries[0].blob.hash,
        snapshot.manifest.entries[1].blob.hash
    );
    assert!(
        snapshot
            .manifest
            .entries
            .iter()
            .all(|entry| !entry.relative_path.contains("secret")
                && !entry.relative_path.contains("node_modules"))
    );
    if linked {
        assert!(
            snapshot
                .manifest
                .entries
                .iter()
                .all(|entry| entry.relative_path != "escape.txt")
        );
    }
    let encrypted = fs::read(
        store
            .path_for(&snapshot.manifest.entries[0].blob)
            .expect("blob path"),
    )
    .expect("encrypted blob");
    assert!(
        !encrypted
            .windows(b"same plaintext".len())
            .any(|window| window == b"same plaintext")
    );

    let prepared = snapshotter
        .prepare_restore(&snapshot, &restore_parent)
        .expect("prepare restore");
    assert_eq!(
        fs::read(prepared.path.join("a.txt")).expect("restored a"),
        b"same plaintext"
    );
    assert_eq!(
        fs::read(prepared.path.join("nested/b.txt")).expect("restored b"),
        b"same plaintext"
    );
    assert_eq!(prepared.manifest_hash, snapshot.manifest.root_hash);
}
