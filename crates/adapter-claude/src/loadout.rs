use adapter_core::{AdapterError, ProviderId, StartAgentRequest};

/// Enforce Claude-specific launch invariants at the adapter boundary as a
/// second line of defense after Supervisor validation.
pub fn validate_start_request(request: &StartAgentRequest) -> Result<(), AdapterError> {
    if request.provider != ProviderId::Claude {
        return Err(AdapterError::Protocol(
            "Claude adapter received a non-Claude provider".to_owned(),
        ));
    }
    if request.route_workspace.trim().is_empty() {
        return Err(AdapterError::Protocol(
            "Claude route workspace is required".to_owned(),
        ));
    }
    if request.loadout_fingerprint.trim().is_empty() {
        return Err(AdapterError::Protocol(
            "Claude loadout fingerprint is required".to_owned(),
        ));
    }
    if request.resume_thread_id.is_some() {
        return Err(AdapterError::Unsupported);
    }
    if request
        .loadout
        .as_ref()
        .is_some_and(|loadout| loadout.provider != ProviderId::Claude)
    {
        return Err(AdapterError::Protocol(
            "Claude loadout provider does not match".to_owned(),
        ));
    }
    Ok(())
}
