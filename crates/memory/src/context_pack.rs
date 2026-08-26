use mission_domain::{EventId, MissionId, RouteId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{MemoryItem, MemoryStatus};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Contract,
    RouteCheckpoint,
    Approval,
    Blocker,
    Risk,
    Evidence,
    ConfirmedMemory,
}

impl ContextKind {
    fn priority(self) -> u8 {
        match self {
            Self::Contract => 0,
            Self::RouteCheckpoint => 1,
            Self::Approval | Self::Blocker | Self::Risk => 2,
            Self::Evidence => 3,
            Self::ConfirmedMemory => 4,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextCandidate {
    pub id: String,
    pub kind: ContextKind,
    pub content: String,
    pub source_event_ids: Vec<EventId>,
    pub version: u64,
}

impl ContextCandidate {
    pub fn new(
        id: impl Into<String>,
        kind: ContextKind,
        content: impl Into<String>,
        source_event_ids: Vec<EventId>,
        version: u64,
    ) -> Result<Self, ContextPackError> {
        if kind == ContextKind::ConfirmedMemory {
            return Err(ContextPackError::UnconfirmedMemory);
        }
        let candidate = Self {
            id: id.into(),
            kind,
            content: content.into(),
            source_event_ids,
            version,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    pub fn from_memory(item: &MemoryItem) -> Result<Self, ContextPackError> {
        if item.status != MemoryStatus::Confirmed {
            return Err(ContextPackError::UnconfirmedMemory);
        }
        let candidate = Self {
            id: item.id.clone(),
            kind: ContextKind::ConfirmedMemory,
            content: item.content.clone(),
            source_event_ids: item.source_event_ids.clone(),
            version: item.version,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    fn validate(&self) -> Result<(), ContextPackError> {
        if self.id.trim().is_empty()
            || self.id.len() > 512
            || self.content.trim().is_empty()
            || self.source_event_ids.is_empty()
            || self.version == 0
        {
            return Err(ContextPackError::InvalidCandidate);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: String,
    pub kind: ContextKind,
    pub content: String,
    pub source_event_ids: Vec<EventId>,
    pub version: u64,
    pub token_estimate: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    Budget,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExcludedContext {
    pub id: String,
    pub reason: ExclusionReason,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextPack {
    pub mission_id: MissionId,
    pub route_id: RouteId,
    pub max_tokens: u32,
    pub items: Vec<ContextItem>,
    pub excluded: Vec<ExcludedContext>,
    pub hash: String,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ContextPackError {
    #[error("context pack budget must be greater than zero")]
    BudgetInvalid,
    #[error("context candidate is invalid")]
    InvalidCandidate,
    #[error("only confirmed memory may enter a context pack")]
    UnconfirmedMemory,
}

pub fn build_context_pack(
    mission_id: MissionId,
    route_id: RouteId,
    max_tokens: u32,
    candidates: &[ContextCandidate],
) -> Result<ContextPack, ContextPackError> {
    if max_tokens == 0 {
        return Err(ContextPackError::BudgetInvalid);
    }
    for candidate in candidates {
        candidate.validate()?;
    }
    let mut ordered = candidates.to_vec();
    ordered.sort_by(|left, right| {
        left.kind
            .priority()
            .cmp(&right.kind.priority())
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.version.cmp(&right.version))
    });
    let mut used = 0_u32;
    let mut items = Vec::new();
    let mut excluded = Vec::new();
    for candidate in ordered {
        let token_estimate = estimate_tokens(&candidate.content);
        if used.saturating_add(token_estimate) <= max_tokens {
            used = used.saturating_add(token_estimate);
            items.push(ContextItem {
                id: candidate.id,
                kind: candidate.kind,
                content: candidate.content,
                source_event_ids: candidate.source_event_ids,
                version: candidate.version,
                token_estimate,
            });
        } else {
            excluded.push(ExcludedContext {
                id: candidate.id,
                reason: ExclusionReason::Budget,
            });
        }
    }
    let mut pack = ContextPack {
        mission_id,
        route_id,
        max_tokens,
        items,
        excluded,
        hash: String::new(),
    };
    pack.hash = digest(&pack)?;
    Ok(pack)
}

fn estimate_tokens(content: &str) -> u32 {
    content.chars().count().div_ceil(4).max(1) as u32
}

fn digest(pack: &ContextPack) -> Result<String, ContextPackError> {
    let bytes = serde_json::to_vec(&(
        pack.mission_id,
        pack.route_id,
        pack.max_tokens,
        &pack.items,
        &pack.excluded,
    ))
    .map_err(|_| ContextPackError::InvalidCandidate)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
