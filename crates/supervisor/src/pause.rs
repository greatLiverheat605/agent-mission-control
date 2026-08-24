use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PauseState {
    Running,
    PauseRequested { reason: String },
    PauseAcknowledged,
    Paused { reason: String },
    PauseTimedOut,
    ForceTerminationAvailable { confirmation_token: String },
    Terminated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseTransition {
    pub from: PauseState,
    pub to: PauseState,
    pub confirmation_token: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PauseError {
    AlreadyRequested,
    InvalidState,
    InvalidConfirmationToken,
}

impl fmt::Display for PauseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyRequested => "pause is already requested",
            Self::InvalidState => "pause transition is invalid for current state",
            Self::InvalidConfirmationToken => "force termination confirmation token is invalid",
        })
    }
}

impl std::error::Error for PauseError {}

#[derive(Clone, Debug)]
pub struct PauseController {
    state: PauseState,
    token_counter: u64,
}

impl Default for PauseController {
    fn default() -> Self {
        Self {
            state: PauseState::Running,
            token_counter: 0,
        }
    }
}

impl PauseController {
    pub fn state(&self) -> &PauseState {
        &self.state
    }

    pub fn request(&mut self, reason: impl Into<String>) -> Result<PauseTransition, PauseError> {
        let reason = reason.into();
        let from = self.state.clone();
        match &self.state {
            PauseState::Running => {
                self.state = PauseState::PauseRequested { reason };
                Ok(PauseTransition {
                    from,
                    to: self.state.clone(),
                    confirmation_token: None,
                })
            }
            PauseState::PauseRequested { .. } | PauseState::PauseAcknowledged => {
                Err(PauseError::AlreadyRequested)
            }
            _ => Err(PauseError::InvalidState),
        }
    }

    pub fn acknowledge(&mut self) -> Result<PauseTransition, PauseError> {
        let from = self.state.clone();
        if !matches!(self.state, PauseState::PauseRequested { .. }) {
            return Err(PauseError::InvalidState);
        }
        self.state = PauseState::Paused {
            reason: match &from {
                PauseState::PauseRequested { reason } => reason.clone(),
                _ => unreachable!(),
            },
        };
        Ok(PauseTransition {
            from,
            to: self.state.clone(),
            confirmation_token: None,
        })
    }

    pub fn timeout(&mut self) -> Result<PauseTransition, PauseError> {
        let from = self.state.clone();
        if !matches!(self.state, PauseState::PauseRequested { .. }) {
            return Err(PauseError::InvalidState);
        }
        self.token_counter += 1;
        let token = format!("pause-force-{}", self.token_counter);
        self.state = PauseState::ForceTerminationAvailable {
            confirmation_token: token.clone(),
        };
        Ok(PauseTransition {
            from,
            to: self.state.clone(),
            confirmation_token: Some(token),
        })
    }

    pub fn force_terminate(&mut self, token: &str) -> Result<PauseTransition, PauseError> {
        let from = self.state.clone();
        let PauseState::ForceTerminationAvailable { confirmation_token } = &self.state else {
            return Err(PauseError::InvalidState);
        };
        if confirmation_token != token {
            return Err(PauseError::InvalidConfirmationToken);
        }
        self.state = PauseState::Terminated;
        Ok(PauseTransition {
            from,
            to: self.state.clone(),
            confirmation_token: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PauseController, PauseState};

    #[test]
    fn safe_pause_never_kills_until_explicit_force_token() {
        let mut controller = PauseController::default();
        controller.request("user requested").expect("request");
        assert!(matches!(
            controller.state(),
            PauseState::PauseRequested { .. }
        ));
        let receipt = controller.timeout().expect("timeout");
        let token = receipt.confirmation_token.expect("force token");
        assert!(controller.force_terminate("wrong").is_err());
        controller.force_terminate(&token).expect("terminate");
        assert_eq!(controller.state(), &PauseState::Terminated);
    }
}
