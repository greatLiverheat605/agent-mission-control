use thiserror::Error;

const THROTTLE_PERCENT: u64 = 80;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservedResources {
    pub memory_bytes: u64,
    pub cpu_percent: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceDecision {
    Proceed,
    Throttle { reason: String },
    PauseAtSafeBoundary { reason: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBudget {
    memory_limit_bytes: u64,
    cpu_limit_percent: u8,
}

impl ResourceBudget {
    pub fn new(
        memory_limit_bytes: u64,
        cpu_limit_percent: u8,
    ) -> Result<Self, ResourceBudgetError> {
        if memory_limit_bytes == 0 || !(1..=100).contains(&cpu_limit_percent) {
            return Err(ResourceBudgetError::InvalidLimit);
        }
        Ok(Self {
            memory_limit_bytes,
            cpu_limit_percent,
        })
    }

    pub fn evaluate(self, observed: ObservedResources) -> ResourceDecision {
        if observed.memory_bytes >= self.memory_limit_bytes
            || observed.cpu_percent >= self.cpu_limit_percent
        {
            return ResourceDecision::PauseAtSafeBoundary {
                reason: format!(
                    "resource limit reached: memory {}/{}, cpu {}/{}%",
                    observed.memory_bytes,
                    self.memory_limit_bytes,
                    observed.cpu_percent,
                    self.cpu_limit_percent
                ),
            };
        }

        if observed.memory_bytes.saturating_mul(100)
            >= self.memory_limit_bytes.saturating_mul(THROTTLE_PERCENT)
            || u64::from(observed.cpu_percent).saturating_mul(100)
                >= u64::from(self.cpu_limit_percent).saturating_mul(THROTTLE_PERCENT)
        {
            return ResourceDecision::Throttle {
                reason: format!(
                    "resource pressure: memory {}/{}, cpu {}/{}%",
                    observed.memory_bytes,
                    self.memory_limit_bytes,
                    observed.cpu_percent,
                    self.cpu_limit_percent
                ),
            };
        }

        ResourceDecision::Proceed
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ResourceBudgetError {
    #[error("resource limits must be non-zero and CPU must not exceed 100 percent")]
    InvalidLimit,
}
