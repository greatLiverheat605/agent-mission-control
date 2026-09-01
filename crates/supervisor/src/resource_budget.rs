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
pub struct ResourcePressure {
    pub memory_bytes: u64,
    pub cpu_percent: u8,
    pub disk_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBudget {
    memory_limit_bytes: u64,
    cpu_limit_percent: u8,
    disk_limit_bytes: Option<u64>,
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
            disk_limit_bytes: None,
        })
    }

    pub fn with_disk_limit(mut self, disk_limit_bytes: u64) -> Result<Self, ResourceBudgetError> {
        if disk_limit_bytes == 0 {
            return Err(ResourceBudgetError::InvalidLimit);
        }
        self.disk_limit_bytes = Some(disk_limit_bytes);
        Ok(self)
    }

    pub fn evaluate(self, observed: ObservedResources) -> ResourceDecision {
        self.evaluate_pressure(ResourcePressure {
            memory_bytes: observed.memory_bytes,
            cpu_percent: observed.cpu_percent,
            disk_bytes: 0,
        })
    }

    pub fn evaluate_pressure(self, observed: ResourcePressure) -> ResourceDecision {
        let disk_critical = self
            .disk_limit_bytes
            .is_some_and(|limit| observed.disk_bytes >= limit);
        let disk_throttle = self.disk_limit_bytes.is_some_and(|limit| {
            observed.disk_bytes.saturating_mul(100) >= limit.saturating_mul(THROTTLE_PERCENT)
        });
        if observed.memory_bytes >= self.memory_limit_bytes
            || observed.cpu_percent >= self.cpu_limit_percent
            || disk_critical
        {
            return ResourceDecision::PauseAtSafeBoundary {
                reason: format!(
                    "resource limit reached: memory {}/{}, cpu {}/{}%, disk {}/{}",
                    observed.memory_bytes,
                    self.memory_limit_bytes,
                    observed.cpu_percent,
                    self.cpu_limit_percent,
                    observed.disk_bytes,
                    self.disk_limit_bytes.unwrap_or(0)
                ),
            };
        }

        if observed.memory_bytes.saturating_mul(100)
            >= self.memory_limit_bytes.saturating_mul(THROTTLE_PERCENT)
            || u64::from(observed.cpu_percent).saturating_mul(100)
                >= u64::from(self.cpu_limit_percent).saturating_mul(THROTTLE_PERCENT)
            || disk_throttle
        {
            return ResourceDecision::Throttle {
                reason: format!(
                    "resource pressure: memory {}/{}, cpu {}/{}%, disk {}/{}",
                    observed.memory_bytes,
                    self.memory_limit_bytes,
                    observed.cpu_percent,
                    self.cpu_limit_percent,
                    observed.disk_bytes,
                    self.disk_limit_bytes.unwrap_or(0)
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
