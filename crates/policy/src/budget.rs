use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use thiserror::Error;

use crate::ApprovalActor;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BudgetDimension {
    Tokens,
    MoneyMicros,
    WallClock,
    ChangedLines,
    ChangedFiles,
    ModelCalls,
}

impl BudgetDimension {
    const ALL: [Self; 6] = [
        Self::Tokens,
        Self::MoneyMicros,
        Self::WallClock,
        Self::ChangedLines,
        Self::ChangedFiles,
        Self::ModelCalls,
    ];
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetLimits {
    pub tokens: u64,
    pub money_micros: u64,
    pub wall_clock: Duration,
    pub changed_lines: u64,
    pub changed_files: u64,
    pub model_calls: u64,
}

impl BudgetLimits {
    fn value(&self, dimension: BudgetDimension) -> u64 {
        match dimension {
            BudgetDimension::Tokens => self.tokens,
            BudgetDimension::MoneyMicros => self.money_micros,
            BudgetDimension::WallClock => {
                self.wall_clock.as_millis().min(u128::from(u64::MAX)) as u64
            }
            BudgetDimension::ChangedLines => self.changed_lines,
            BudgetDimension::ChangedFiles => self.changed_files,
            BudgetDimension::ModelCalls => self.model_calls,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownUsagePolicy {
    RequireApproval,
    Pause,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsageSample {
    values: BTreeMap<BudgetDimension, u64>,
}

impl UsageSample {
    pub fn tokens(value: u64) -> Self {
        Self::one(BudgetDimension::Tokens, value)
    }

    pub fn money_micros(value: u64) -> Self {
        Self::one(BudgetDimension::MoneyMicros, value)
    }

    pub fn wall_clock(value: Duration) -> Self {
        Self::one(
            BudgetDimension::WallClock,
            value.as_millis().min(u128::from(u64::MAX)) as u64,
        )
    }

    pub fn changed_lines(value: u64) -> Self {
        Self::one(BudgetDimension::ChangedLines, value)
    }

    pub fn changed_files(value: u64) -> Self {
        Self::one(BudgetDimension::ChangedFiles, value)
    }

    pub fn model_calls(value: u64) -> Self {
        Self::one(BudgetDimension::ModelCalls, value)
    }

    pub fn with(mut self, dimension: BudgetDimension, value: u64) -> Self {
        self.values.insert(dimension, value);
        self
    }

    fn one(dimension: BudgetDimension, value: u64) -> Self {
        Self::default().with(dimension, value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsageRecord {
    Sample(UsageSample),
    Unknown(BudgetDimension),
    Correction {
        dimension: BudgetDimension,
        corrected_total: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetSignal {
    Warning(BudgetDimension),
    RequireApproval(BudgetDimension),
    PauseAtSafeBoundary(BudgetDimension),
}

#[derive(Clone, Debug)]
enum Projection {
    Known(u64),
    Unknown,
}

#[derive(Clone, Debug)]
pub struct BudgetTracker {
    contract_version: u64,
    limits: BudgetLimits,
    unknown_policy: UnknownUsagePolicy,
    records: Vec<UsageRecord>,
    projection: BTreeMap<BudgetDimension, Projection>,
    warned: BTreeSet<BudgetDimension>,
}

impl BudgetTracker {
    pub fn new(
        contract_version: u64,
        limits: BudgetLimits,
        unknown_policy: UnknownUsagePolicy,
    ) -> Self {
        let projection = BudgetDimension::ALL
            .into_iter()
            .map(|dimension| (dimension, Projection::Known(0)))
            .collect();
        Self {
            contract_version,
            limits,
            unknown_policy,
            records: Vec::new(),
            projection,
            warned: BTreeSet::new(),
        }
    }

    pub const fn contract_version(&self) -> u64 {
        self.contract_version
    }

    pub fn limits(&self) -> &BudgetLimits {
        &self.limits
    }

    pub fn records(&self) -> &[UsageRecord] {
        &self.records
    }

    pub fn record(&mut self, record: UsageRecord) {
        match &record {
            UsageRecord::Sample(sample) => {
                for (&dimension, &increment) in &sample.values {
                    if let Some(Projection::Known(total)) = self.projection.get_mut(&dimension) {
                        *total = total.saturating_add(increment);
                    }
                }
            }
            UsageRecord::Unknown(dimension) => {
                self.projection.insert(*dimension, Projection::Unknown);
            }
            UsageRecord::Correction {
                dimension,
                corrected_total,
            } => {
                self.projection
                    .insert(*dimension, Projection::Known(*corrected_total));
            }
        }
        self.records.push(record);
    }

    pub fn evaluate_safe_boundary(&mut self) -> Vec<BudgetSignal> {
        let mut signals = Vec::new();
        for dimension in BudgetDimension::ALL {
            match self.projection.get(&dimension) {
                Some(Projection::Unknown) => signals.push(match self.unknown_policy {
                    UnknownUsagePolicy::RequireApproval => BudgetSignal::RequireApproval(dimension),
                    UnknownUsagePolicy::Pause => BudgetSignal::PauseAtSafeBoundary(dimension),
                }),
                Some(Projection::Known(used)) => {
                    let limit = self.limits.value(dimension);
                    if limit == 0 || *used >= limit {
                        signals.push(BudgetSignal::PauseAtSafeBoundary(dimension));
                    } else if used.saturating_mul(5) >= limit.saturating_mul(4)
                        && self.warned.insert(dimension)
                    {
                        signals.push(BudgetSignal::Warning(dimension));
                    }
                }
                None => signals.push(BudgetSignal::PauseAtSafeBoundary(dimension)),
            }
        }
        signals
    }

    pub fn replace_limits(&mut self, change: BudgetChange) -> Result<(), BudgetChangeError> {
        if change.actor != ApprovalActor::User {
            return Err(BudgetChangeError::ForbiddenActor);
        }
        if change.contract_version <= self.contract_version {
            return Err(BudgetChangeError::ContractVersionNotAdvanced);
        }
        self.contract_version = change.contract_version;
        self.limits = change.limits;
        self.warned.clear();
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetChange {
    pub actor: ApprovalActor,
    pub contract_version: u64,
    pub limits: BudgetLimits,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BudgetChangeError {
    #[error("only a user may change mission budget")]
    ForbiddenActor,
    #[error("budget change requires a newer contract version")]
    ContractVersionNotAdvanced,
}
