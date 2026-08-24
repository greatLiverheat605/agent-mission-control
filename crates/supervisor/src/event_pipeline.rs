use std::fmt::Display;
use std::sync::mpsc::Sender;

use mission_domain::{EventEnvelope, MissionId, ProjectionError, ReadModel, reduce, replay};
use mission_ledger::{EncryptedLedger, LedgerError};
use thiserror::Error;

pub trait AppendOnlyLedger {
    type Error: Display;

    fn append(&mut self, event: &EventEnvelope) -> Result<(), Self::Error>;
    fn replay(&self, mission_id: &MissionId) -> Result<Vec<EventEnvelope>, Self::Error>;
}

impl AppendOnlyLedger for EncryptedLedger {
    type Error = LedgerError;

    fn append(&mut self, event: &EventEnvelope) -> Result<(), Self::Error> {
        EncryptedLedger::append(self, event)
    }

    fn replay(&self, mission_id: &MissionId) -> Result<Vec<EventEnvelope>, Self::Error> {
        self.replay_events(mission_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PipelineStatus {
    Running,
    Paused { reason: String },
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("ledger append/replay failed: {0}")]
    Ledger(String),
    #[error("projection failed after append: {0}")]
    Projection(#[from] ProjectionError),
    #[error("pipeline is paused: {0}")]
    Paused(String),
}

pub struct EventPipeline<L> {
    ledger: L,
    model: ReadModel,
    status: PipelineStatus,
    subscribers: Vec<Sender<ReadModel>>,
}

impl<L: AppendOnlyLedger> EventPipeline<L> {
    pub fn recover(ledger: L, mission_id: &MissionId) -> Result<Self, PipelineError> {
        let events = ledger
            .replay(mission_id)
            .map_err(|error| PipelineError::Ledger(error.to_string()))?;
        let model = replay(events).map_err(PipelineError::Projection)?;
        Ok(Self {
            ledger,
            model,
            status: PipelineStatus::Running,
            subscribers: Vec::new(),
        })
    }

    pub fn with_empty(ledger: L) -> Self {
        Self {
            ledger,
            model: ReadModel::default(),
            status: PipelineStatus::Running,
            subscribers: Vec::new(),
        }
    }

    pub fn append(&mut self, event: EventEnvelope) -> Result<ReadModel, PipelineError> {
        if let PipelineStatus::Paused { reason } = &self.status {
            return Err(PipelineError::Paused(reason.clone()));
        }
        if let Err(error) = self.ledger.append(&event) {
            let reason = error.to_string();
            self.status = PipelineStatus::Paused {
                reason: reason.clone(),
            };
            return Err(PipelineError::Ledger(reason));
        }
        let next = match reduce(&self.model, &event) {
            Ok(next) => next,
            Err(error) => {
                let reason = error.to_string();
                self.status = PipelineStatus::Paused {
                    reason: reason.clone(),
                };
                return Err(PipelineError::Projection(error));
            }
        };
        self.model = next;
        self.subscribers
            .retain(|subscriber| subscriber.send(self.model.clone()).is_ok());
        Ok(self.model.clone())
    }

    pub fn subscribe(&mut self, sender: Sender<ReadModel>) {
        let _ = sender.send(self.model.clone());
        self.subscribers.push(sender);
    }

    pub fn model(&self) -> &ReadModel {
        &self.model
    }

    pub fn status(&self) -> &PipelineStatus {
        &self.status
    }

    pub fn ledger(&self) -> &L {
        &self.ledger
    }

    pub fn ledger_mut(&mut self) -> &mut L {
        &mut self.ledger
    }
}
