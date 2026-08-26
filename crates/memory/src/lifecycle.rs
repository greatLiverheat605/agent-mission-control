use std::collections::BTreeMap;

use mission_domain::{EventConfidence, EventEnvelope, EventId, EventKind, EventLinks, EventSource};
use serde::{Deserialize, Serialize};

use crate::{MemoryAuthor, MemoryError, MemoryItem, MemoryKind, MemoryStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAction {
    Created,
    Confirmed,
    Edited,
    Narrowed,
    Deferred,
    Invalidated,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryMutation {
    pub action: MemoryAction,
    pub actor: MemoryAuthor,
    pub item: MemoryItem,
    pub event: EventEnvelope,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    items: BTreeMap<String, MemoryItem>,
    history: Vec<MemoryMutation>,
    next_sequence: u64,
}

impl MemoryStore {
    pub fn insert(&mut self, item: MemoryItem) -> Result<(), MemoryError> {
        item.validate()?;
        if item.status != MemoryStatus::Candidate {
            return Err(MemoryError::InvalidTransition);
        }
        if self.items.contains_key(&item.id) {
            return Err(MemoryError::DuplicateId);
        }
        self.items.insert(item.id.clone(), item.clone());
        self.append_mutation(MemoryAction::Created, item.author, item);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&MemoryItem> {
        self.items.get(id)
    }

    pub fn history(&self) -> &[MemoryMutation] {
        &self.history
    }

    pub fn confirm(&mut self, id: &str, actor: MemoryAuthor) -> Result<MemoryItem, MemoryError> {
        self.mutate(
            id,
            actor,
            MemoryAction::Confirmed,
            MemoryStatus::Confirmed,
            None,
            |item| {
                if item.kind == MemoryKind::Inference {
                    return Err(MemoryError::InferenceCannotBeConfirmed);
                }
                if !matches!(
                    item.status,
                    MemoryStatus::Candidate | MemoryStatus::Deferred
                ) {
                    return Err(MemoryError::InvalidTransition);
                }
                Ok(())
            },
        )
    }

    pub fn edit(
        &mut self,
        id: &str,
        actor: MemoryAuthor,
        content: impl Into<String>,
    ) -> Result<MemoryItem, MemoryError> {
        self.mutate_with_content(id, actor, MemoryAction::Edited, content.into())
    }

    pub fn narrow(
        &mut self,
        id: &str,
        actor: MemoryAuthor,
        content: impl Into<String>,
    ) -> Result<MemoryItem, MemoryError> {
        self.mutate_with_content(id, actor, MemoryAction::Narrowed, content.into())
    }

    pub fn defer(&mut self, id: &str, actor: MemoryAuthor) -> Result<MemoryItem, MemoryError> {
        self.mutate(
            id,
            actor,
            MemoryAction::Deferred,
            MemoryStatus::Deferred,
            None,
            |item| {
                if !matches!(
                    item.status,
                    MemoryStatus::Candidate | MemoryStatus::Confirmed
                ) {
                    return Err(MemoryError::InvalidTransition);
                }
                Ok(())
            },
        )
    }

    pub fn invalidate(&mut self, id: &str, actor: MemoryAuthor) -> Result<MemoryItem, MemoryError> {
        self.mutate(
            id,
            actor,
            MemoryAction::Invalidated,
            MemoryStatus::Invalidated,
            None,
            |item| {
                if matches!(
                    item.status,
                    MemoryStatus::Invalidated | MemoryStatus::Rejected
                ) {
                    return Err(MemoryError::InvalidTransition);
                }
                Ok(())
            },
        )
    }

    pub fn reject(&mut self, id: &str, actor: MemoryAuthor) -> Result<MemoryItem, MemoryError> {
        self.mutate(
            id,
            actor,
            MemoryAction::Rejected,
            MemoryStatus::Rejected,
            None,
            |item| {
                if !matches!(
                    item.status,
                    MemoryStatus::Candidate | MemoryStatus::Deferred
                ) {
                    return Err(MemoryError::InvalidTransition);
                }
                Ok(())
            },
        )
    }

    fn mutate_with_content(
        &mut self,
        id: &str,
        actor: MemoryAuthor,
        action: MemoryAction,
        content: String,
    ) -> Result<MemoryItem, MemoryError> {
        if content.trim().is_empty() {
            return Err(MemoryError::InvalidItem);
        }
        self.mutate(
            id,
            actor,
            action,
            MemoryStatus::Candidate,
            Some(content),
            |item| {
                if matches!(
                    item.status,
                    MemoryStatus::Invalidated | MemoryStatus::Rejected
                ) {
                    return Err(MemoryError::InvalidTransition);
                }
                Ok(())
            },
        )
    }

    fn mutate<F>(
        &mut self,
        id: &str,
        actor: MemoryAuthor,
        action: MemoryAction,
        status: MemoryStatus,
        content: Option<String>,
        validate: F,
    ) -> Result<MemoryItem, MemoryError>
    where
        F: FnOnce(&MemoryItem) -> Result<(), MemoryError>,
    {
        if actor != MemoryAuthor::User {
            return Err(MemoryError::ForbiddenActor);
        }
        let current = self.items.get(id).cloned().ok_or(MemoryError::NotFound)?;
        validate(&current)?;
        let mut next = current;
        if let Some(content) = content {
            next.content = content;
        }
        next.status = status;
        next.version = next
            .version
            .checked_add(1)
            .ok_or(MemoryError::InvalidItem)?;
        next.author = actor;
        next.validate()?;
        self.items.insert(id.to_owned(), next.clone());
        self.append_mutation(action, actor, next.clone());
        Ok(next)
    }

    fn append_mutation(&mut self, action: MemoryAction, actor: MemoryAuthor, item: MemoryItem) {
        self.next_sequence = self.next_sequence.saturating_add(1);
        let mut event = EventEnvelope::new(
            EventId::new(),
            item.mission_id,
            item.route_id,
            self.next_sequence,
            EventKind::MemoryItemChanged,
            serde_json::json!({"action": action, "item": item}),
        );
        event.source = if actor == MemoryAuthor::User {
            EventSource::User
        } else {
            EventSource::Supervisor
        };
        event.confidence = if item.status == MemoryStatus::Confirmed {
            EventConfidence::Confirmed
        } else {
            EventConfidence::Observed
        };
        event.links = EventLinks {
            parent_event_id: None,
            source_event_ids: item.source_event_ids.clone(),
        };
        self.history.push(MemoryMutation {
            action,
            actor,
            item,
            event,
        });
    }
}
