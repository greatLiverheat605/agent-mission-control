use mission_domain::{ContractPatch, MissionContract, PatchActor, VersionConflict};

#[test]
fn contract_patch_requires_expected_version_and_emits_field_diff() {
    let contract = MissionContract::default();
    let (next, diff) = contract
        .apply_patch(
            1,
            ContractPatch {
                goal: Some("ship the feature".to_owned()),
                ..ContractPatch::default()
            },
            PatchActor::User,
        )
        .expect("apply current contract");
    assert_eq!(next.version, 2);
    assert_eq!(diff.fields[0].field, "goal");
    assert_eq!(diff.from_version, 1);
    assert_eq!(diff.to_version, 2);

    assert_eq!(
        next.apply_patch(1, ContractPatch::default(), PatchActor::User),
        Err(VersionConflict::ExpectedVersion {
            expected: 1,
            actual: 2,
        })
    );
}

#[test]
fn agent_cannot_mutate_contract_even_with_current_version() {
    let error = MissionContract::default()
        .apply_patch(1, ContractPatch::default(), PatchActor::Agent)
        .expect_err("agent mutation must be rejected");
    assert_eq!(error, VersionConflict::AgentMutationForbidden);
}
