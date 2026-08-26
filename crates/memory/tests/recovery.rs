use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};

use mission_domain::{MissionId, RouteId};
use mission_ledger::EncryptedBlobStore;
use mission_memory::{RecoveryConstraints, RecoveryError, RecoveryInput, build_recovery_package};

fn input(mission_id: MissionId, route_id: RouteId) -> RecoveryInput {
    RecoveryInput {
        mission_id,
        route_id,
        contract_version: 3,
        checkpoint_id: "checkpoint-3".to_owned(),
        ledger_sequence: 18,
        loadout_fingerprint: "loadout-v3".to_owned(),
        context_pack_hash: "context-v3".to_owned(),
        pending_approval_hash: Some("approval-v3".to_owned()),
        permissions: BTreeSet::from(["read_workspace".to_owned()]),
        payload: br#"{"route_state":"paused","pending_work":["review"]}"#.to_vec(),
    }
}

fn constraints(input: &RecoveryInput) -> RecoveryConstraints {
    RecoveryConstraints {
        mission_id: input.mission_id,
        route_id: input.route_id,
        contract_version: input.contract_version,
        ledger_sequence: input.ledger_sequence,
        loadout_fingerprint: input.loadout_fingerprint.clone(),
        context_pack_hash: input.context_pack_hash.clone(),
        pending_approval_hash: input.pending_approval_hash.clone(),
        permissions: input.permissions.clone(),
    }
}

#[test]
fn recovery_package_is_encrypted_and_verifiable() {
    let temp = tempfile::tempdir().expect("temp");
    let store = EncryptedBlobStore::open(temp.path(), [7; 32]).expect("store");
    let mission = MissionId::new();
    let route = RouteId::new();
    let input = input(mission, route);
    let package = build_recovery_package(&store, input.clone()).expect("package");
    let plaintext = package
        .verify(&store, &constraints(&input))
        .expect("verify");
    assert_eq!(plaintext, input.payload);
    assert_eq!(package.manifest.schema_version, 1);
    assert_ne!(
        std::fs::read(store.path_for(&package.blob).expect("blob path")).expect("blob"),
        input.payload
    );
}

#[test]
fn recovery_rejects_tampering_future_sequence_and_permission_expansion() {
    let temp = tempfile::tempdir().expect("temp");
    let store = EncryptedBlobStore::open(temp.path(), [8; 32]).expect("store");
    let mission = MissionId::new();
    let route = RouteId::new();
    let input = input(mission, route);
    let package = build_recovery_package(&store, input.clone()).expect("package");

    let path = store.path_for(&package.blob).expect("blob path");
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open blob");
    file.seek(SeekFrom::End(-1)).expect("seek");
    file.write_all(&[0xff]).expect("tamper");
    assert_eq!(
        package.verify(&store, &constraints(&input)),
        Err(RecoveryError::BlobIntegrity)
    );

    let future_temp = tempfile::tempdir().expect("future temp");
    let future_store = EncryptedBlobStore::open(future_temp.path(), [9; 32]).expect("store");
    let package = build_recovery_package(&future_store, input.clone()).expect("package");
    let mut future = constraints(&input);
    future.ledger_sequence = 17;
    assert_eq!(
        package.verify(&store, &future),
        Err(RecoveryError::SequenceInvalid)
    );

    let mut expanded = input.clone();
    expanded.permissions.insert("write_workspace".to_owned());
    let expanded_temp = tempfile::tempdir().expect("expanded temp");
    let expanded_store = EncryptedBlobStore::open(expanded_temp.path(), [10; 32]).expect("store");
    let expanded_package =
        build_recovery_package(&expanded_store, expanded).expect("expanded package");
    assert_eq!(
        expanded_package.verify(&expanded_store, &constraints(&input)),
        Err(RecoveryError::PermissionExpansion)
    );
}
