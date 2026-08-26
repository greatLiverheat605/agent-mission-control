use mission_domain::{MissionId, RouteId};
use mission_policy::{
    ActionClass, ApprovalAction, ApprovalActor, ApprovalError, ApprovalRequest,
    ApprovalResolution, ApprovalScope, ApprovalState, ApprovalSubject,
};

fn subject() -> ApprovalSubject {
    ApprovalSubject {
        mission_id: MissionId::new(),
        route_id: RouteId::new(),
        action_digest: "sha256:write-1".to_owned(),
        action_class: ActionClass::Write,
        contract_version: 3,
        loadout_fingerprint: "loadout-v1".to_owned(),
    }
}

fn request(scope: ApprovalScope) -> ApprovalRequest {
    ApprovalRequest::new(
        "approval-1",
        subject(),
        scope,
        ApprovalActor::Supervisor,
        1_000,
    )
    .expect("supervisor can request approval")
}

fn resolve(request: &mut ApprovalRequest, decision: ApprovalAction, now_ms: u64) {
    let resolution = ApprovalResolution {
        approval_id: request.id().to_owned(),
        expected_revision: request.revision(),
        actor: ApprovalActor::User,
        decision,
        subject: request.subject().clone(),
        now_ms,
    };
    request.resolve(resolution).expect("resolve approval");
}

#[test]
fn once_approval_is_consumed_and_cannot_be_replayed() {
    let mut approval = request(ApprovalScope::Once);
    resolve(&mut approval, ApprovalAction::Approve, 100);
    assert_eq!(approval.state(), ApprovalState::Approved);

    let approved_subject = approval.subject().clone();
    approval
        .authorize(&approved_subject, 101)
        .expect("first dispatch is authorized");
    assert_eq!(approval.state(), ApprovalState::Consumed);
    assert_eq!(
        approval.authorize(&approved_subject, 102),
        Err(ApprovalError::NotApproved)
    );
}

#[test]
fn route_action_scope_accepts_only_matching_authority_context() {
    let mut approval = request(ApprovalScope::RouteActionClass(ActionClass::Write));
    resolve(&mut approval, ApprovalAction::Approve, 100);

    let mut next_write = approval.subject().clone();
    next_write.action_digest = "sha256:write-2".to_owned();
    approval
        .authorize(&next_write, 101)
        .expect("same route action class is authorized");
    assert_eq!(approval.state(), ApprovalState::Approved);

    for changed in [
        {
            let mut value = next_write.clone();
            value.mission_id = MissionId::new();
            value
        },
        {
            let mut value = next_write.clone();
            value.route_id = RouteId::new();
            value
        },
        {
            let mut value = next_write.clone();
            value.contract_version += 1;
            value
        },
        {
            let mut value = next_write.clone();
            value.loadout_fingerprint = "loadout-v2".to_owned();
            value
        },
        {
            let mut value = next_write.clone();
            value.action_class = ActionClass::Build;
            value
        },
    ] {
        assert_eq!(
            approval.authorize(&changed, 101),
            Err(ApprovalError::ContextMismatch)
        );
    }
}

#[test]
fn expiration_denial_revocation_and_duplicate_resolve_fail_closed() {
    let mut expired = request(ApprovalScope::Once);
    let late = ApprovalResolution {
        approval_id: expired.id().to_owned(),
        expected_revision: expired.revision(),
        actor: ApprovalActor::User,
        decision: ApprovalAction::Approve,
        subject: expired.subject().clone(),
        now_ms: 1_001,
    };
    assert_eq!(expired.resolve(late), Err(ApprovalError::Expired));
    assert_eq!(expired.state(), ApprovalState::Expired);

    let mut denied = request(ApprovalScope::Once);
    resolve(&mut denied, ApprovalAction::Deny, 100);
    assert_eq!(denied.state(), ApprovalState::Denied);
    let duplicate = ApprovalResolution {
        approval_id: denied.id().to_owned(),
        expected_revision: 0,
        actor: ApprovalActor::User,
        decision: ApprovalAction::Approve,
        subject: denied.subject().clone(),
        now_ms: 101,
    };
    assert_eq!(
        denied.resolve(duplicate),
        Err(ApprovalError::RevisionConflict {
            expected: 0,
            actual: 1,
        })
    );

    let mut revoked = request(ApprovalScope::Once);
    resolve(&mut revoked, ApprovalAction::Approve, 100);
    revoked.revoke(ApprovalActor::User).expect("user revoke");
    assert_eq!(revoked.state(), ApprovalState::Revoked);
}

#[test]
fn agent_cannot_request_resolve_or_revoke_approval() {
    assert_eq!(
        ApprovalRequest::new(
            "approval-agent",
            subject(),
            ApprovalScope::Once,
            ApprovalActor::Agent,
            1_000,
        ),
        Err(ApprovalError::ForbiddenActor)
    );

    let mut approval = request(ApprovalScope::Once);
    let agent_resolution = ApprovalResolution {
        approval_id: approval.id().to_owned(),
        expected_revision: approval.revision(),
        actor: ApprovalActor::Agent,
        decision: ApprovalAction::Approve,
        subject: approval.subject().clone(),
        now_ms: 100,
    };
    assert_eq!(
        approval.resolve(agent_resolution),
        Err(ApprovalError::ForbiddenActor)
    );
    assert_eq!(
        approval.revoke(ApprovalActor::Agent),
        Err(ApprovalError::ForbiddenActor)
    );
}
