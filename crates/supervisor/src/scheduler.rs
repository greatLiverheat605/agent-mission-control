use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use mission_domain::MissionId;
use thiserror::Error;

use crate::resource_budget::ResourceDecision;

const FOREGROUND_WEIGHT: usize = 3;
const BACKGROUND_WEIGHT: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    max_active: usize,
}

impl SchedulerConfig {
    pub fn new(max_active: usize) -> Result<Self, SchedulerError> {
        if !(1..=5).contains(&max_active) {
            return Err(SchedulerError::InvalidConcurrency);
        }
        Ok(Self { max_active })
    }

    pub const fn max_active(self) -> usize {
        self.max_active
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { max_active: 3 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionAuthorization {
    PolicyAllowed,
    ApprovalConsumed,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissionScheduleState {
    Queued,
    Active,
    Paused,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledAction {
    pub mission_id: MissionId,
    pub action_id: String,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub stdout_channel: String,
}

#[derive(Debug)]
struct MissionQueue {
    state: MissionScheduleState,
    foreground: bool,
    actions: VecDeque<ScheduledAction>,
    resource_reason: Option<String>,
}

#[derive(Debug)]
pub struct MissionScheduler {
    config: SchedulerConfig,
    missions: BTreeMap<MissionId, MissionQueue>,
    registration_order: Vec<MissionId>,
    schedule: Vec<MissionId>,
    cursor: usize,
}

impl MissionScheduler {
    pub fn new(config: SchedulerConfig) -> Result<Self, SchedulerError> {
        SchedulerConfig::new(config.max_active)?;
        Ok(Self {
            config,
            missions: BTreeMap::new(),
            registration_order: Vec::new(),
            schedule: Vec::new(),
            cursor: 0,
        })
    }

    pub fn register(
        &mut self,
        mission_id: MissionId,
        foreground: bool,
    ) -> Result<(), SchedulerError> {
        if self.missions.contains_key(&mission_id) {
            return Err(SchedulerError::MissionAlreadyRegistered);
        }
        self.missions.insert(
            mission_id,
            MissionQueue {
                state: MissionScheduleState::Queued,
                foreground,
                actions: VecDeque::new(),
                resource_reason: None,
            },
        );
        self.registration_order.push(mission_id);
        Ok(())
    }

    pub fn enqueue(
        &mut self,
        action: ScheduledAction,
        authorization: ActionAuthorization,
    ) -> Result<(), SchedulerError> {
        if authorization == ActionAuthorization::Denied {
            return Err(SchedulerError::ActionNotApproved);
        }
        let mission = self
            .missions
            .get_mut(&action.mission_id)
            .ok_or(SchedulerError::MissionNotRegistered)?;
        if mission.state == MissionScheduleState::Completed {
            return Err(SchedulerError::MissionCompleted);
        }
        mission.actions.push_back(action);
        Ok(())
    }

    pub fn activate_ready(&mut self) {
        let mut available = self.config.max_active.saturating_sub(self.active_count());
        if available == 0 {
            return;
        }
        for mission_id in &self.registration_order {
            let Some(mission) = self.missions.get_mut(mission_id) else {
                continue;
            };
            if available > 0
                && mission.state == MissionScheduleState::Queued
                && !mission.actions.is_empty()
            {
                mission.state = MissionScheduleState::Active;
                available -= 1;
            }
        }
        self.rebuild_schedule();
    }

    pub fn pause(
        &mut self,
        mission_id: MissionId,
        _reason: impl Into<String>,
    ) -> Result<(), SchedulerError> {
        let mission = self
            .missions
            .get_mut(&mission_id)
            .ok_or(SchedulerError::MissionNotRegistered)?;
        if mission.state == MissionScheduleState::Completed {
            return Err(SchedulerError::MissionCompleted);
        }
        mission.state = MissionScheduleState::Paused;
        self.rebuild_schedule();
        self.activate_ready();
        Ok(())
    }

    pub fn complete(&mut self, mission_id: MissionId) -> Result<(), SchedulerError> {
        let mission = self
            .missions
            .get_mut(&mission_id)
            .ok_or(SchedulerError::MissionNotRegistered)?;
        mission.state = MissionScheduleState::Completed;
        self.rebuild_schedule();
        self.activate_ready();
        Ok(())
    }

    pub fn apply_resource_decision(
        &mut self,
        mission_id: MissionId,
        decision: ResourceDecision,
    ) -> Result<(), SchedulerError> {
        match decision {
            ResourceDecision::Proceed => {
                self.mission_mut(mission_id)?.resource_reason = None;
            }
            ResourceDecision::Throttle { reason } => {
                self.mission_mut(mission_id)?.resource_reason = Some(reason);
            }
            ResourceDecision::PauseAtSafeBoundary { reason } => {
                self.mission_mut(mission_id)?.resource_reason = Some(reason.clone());
                self.pause(mission_id, reason)?;
            }
        }
        Ok(())
    }

    pub fn active_count(&self) -> usize {
        self.missions
            .values()
            .filter(|mission| mission.state == MissionScheduleState::Active)
            .count()
    }

    pub fn state(&self, mission_id: MissionId) -> Option<MissionScheduleState> {
        self.missions.get(&mission_id).map(|mission| mission.state)
    }

    pub fn queued_actions(&self, mission_id: MissionId) -> usize {
        self.missions
            .get(&mission_id)
            .map_or(0, |mission| mission.actions.len())
    }

    pub fn resource_reason(&self, mission_id: MissionId) -> Option<&str> {
        self.missions
            .get(&mission_id)
            .and_then(|mission| mission.resource_reason.as_deref())
    }

    fn mission_mut(&mut self, mission_id: MissionId) -> Result<&mut MissionQueue, SchedulerError> {
        self.missions
            .get_mut(&mission_id)
            .ok_or(SchedulerError::MissionNotRegistered)
    }

    fn rebuild_schedule(&mut self) {
        self.schedule.clear();
        for mission_id in &self.registration_order {
            let Some(mission) = self.missions.get(mission_id) else {
                continue;
            };
            if mission.state != MissionScheduleState::Active {
                continue;
            }
            let weight = if mission.foreground {
                FOREGROUND_WEIGHT
            } else {
                BACKGROUND_WEIGHT
            };
            self.schedule
                .extend(std::iter::repeat_n(*mission_id, weight));
        }
        self.cursor = 0;
    }
}

impl Iterator for MissionScheduler {
    type Item = ScheduledAction;

    fn next(&mut self) -> Option<Self::Item> {
        for _ in 0..self.schedule.len() {
            let mission_id = self.schedule[self.cursor];
            self.cursor = (self.cursor + 1) % self.schedule.len();
            let mission = self.missions.get_mut(&mission_id)?;
            if mission.state == MissionScheduleState::Active
                && let Some(action) = mission.actions.pop_front()
            {
                return Some(action);
            }
        }
        None
    }
}

impl Default for MissionScheduler {
    fn default() -> Self {
        Self::new(SchedulerConfig::default()).expect("default scheduler config is valid")
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SchedulerError {
    #[error("mission concurrency must be between 1 and 5")]
    InvalidConcurrency,
    #[error("mission is already registered")]
    MissionAlreadyRegistered,
    #[error("mission is not registered")]
    MissionNotRegistered,
    #[error("action has not passed policy or consumed approval")]
    ActionNotApproved,
    #[error("completed mission cannot be scheduled")]
    MissionCompleted,
}
