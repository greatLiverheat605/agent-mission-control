#![cfg(windows)]

use mission_protocol::windows_security::{
    SecurityAttributes, opaque_user_id, user_system_admin_sddl,
};

const FIXTURE_SID: &str = "S-1-5-21-111-222-333-1001";

#[test]
fn opaque_user_id_is_a_stable_sid_digest() {
    let id = opaque_user_id(FIXTURE_SID);

    assert_eq!(id.len(), 64);
    assert_eq!(&id[..16], "4c51f3baadf41ed2");
    assert!(!id.contains(FIXTURE_SID));
}

#[test]
fn shared_object_acl_grants_only_system_admins_and_the_user() {
    let sddl = user_system_admin_sddl(FIXTURE_SID);

    assert_eq!(
        sddl,
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;S-1-5-21-111-222-333-1001)"
    );
    SecurityAttributes::from_sddl(&sddl).expect("fixture SDDL is valid");
}
