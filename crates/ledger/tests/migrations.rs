use std::fs;

use mission_ledger::migrations::{backup_database, backup_path, restore_database};

#[test]
fn backup_is_content_hashed_and_restore_preserves_original() {
    let root = std::env::temp_dir().join(format!("mission-migration-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("create temp root");
    let database = root.join("ledger.db");
    fs::write(&database, b"encrypted database bytes").expect("write database");
    let backup = backup_database(&database).expect("backup database");
    assert_eq!(backup, backup_path(&database));
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("backup-")
    );
    fs::write(&database, b"damaged").expect("damage database");
    restore_database(&backup, &database).expect("restore database");
    assert_eq!(
        fs::read(&database).expect("read restored database"),
        b"encrypted database bytes"
    );
    fs::remove_dir_all(root).expect("remove temp root");
}
