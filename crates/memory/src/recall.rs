use mission_domain::{EventId, MissionId, RouteId};
use serde::{Deserialize, Serialize};

use crate::{MemoryFreshness, MemoryItem, MemoryScope, MemoryStatus};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecallEvidence {
    pub id: String,
    pub content: String,
    pub source_event_ids: Vec<EventId>,
    pub scope: MemoryScope,
    pub freshness: MemoryFreshness,
    pub version: u64,
    pub score: u32,
    pub matched_terms: Vec<String>,
}

pub fn recall_confirmed(
    items: &[MemoryItem],
    mission_id: MissionId,
    route_id: RouteId,
    query: &str,
    limit: usize,
) -> Vec<RecallEvidence> {
    if limit == 0 {
        return Vec::new();
    }
    let terms = query_terms(query);
    let mut matches: Vec<_> = items
        .iter()
        .filter(|item| item.status == MemoryStatus::Confirmed)
        .filter(|item| item.mission_id == mission_id)
        .filter(|item| item.scope == MemoryScope::Mission || item.route_id == route_id)
        .map(|item| {
            let content = item.content.to_ascii_lowercase();
            let matched_terms = terms
                .iter()
                .filter(|term| {
                    content
                        .split_whitespace()
                        .any(|word| word.contains(term.as_str()))
                })
                .cloned()
                .collect::<Vec<_>>();
            let score = matched_terms.len() as u32;
            RecallEvidence {
                id: item.id.clone(),
                content: item.content.clone(),
                source_event_ids: item.source_event_ids.clone(),
                scope: item.scope,
                freshness: item.freshness,
                version: item.version,
                score,
                matched_terms,
            }
        })
        .filter(|item| terms.is_empty() || item.score > 0)
        .collect();
    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.version.cmp(&right.version))
    });
    matches.truncate(limit);
    matches
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() >= 2)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms
}
