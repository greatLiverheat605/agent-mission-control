use std::collections::{BTreeMap, HashSet};

use crate::delete::{AuditReceipt, LifecycleError, audit_receipt, plan_hash, utc_now_rfc3339};
use crate::{EncryptedLedger, LedgerError};
use mission_domain::MissionId;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageBudget {
    pub project_limit_bytes: Option<u64>,
    pub global_limit_bytes: Option<u64>,
}

impl StorageBudget {
    pub const fn new(project_limit_bytes: Option<u64>, global_limit_bytes: Option<u64>) -> Self {
        Self {
            project_limit_bytes,
            global_limit_bytes,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageUsage {
    pub mission_id: Option<String>,
    pub event_count: u64,
    pub event_bytes: u64,
    pub blob_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MissionStorage {
    pub mission_id: MissionId,
    pub event_count: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetentionPlan {
    pub budget: StorageBudget,
    pub project_usage: StorageUsage,
    pub global_usage: StorageUsage,
    pub over_budget: bool,
    pub automatic_deletion: bool,
    pub candidate_missions: Vec<MissionStorage>,
    pub impact_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArchivePlan {
    pub mission_id: MissionId,
    pub event_count: u64,
    pub bytes: u64,
    pub impact_hash: String,
    pub created_at: String,
}

pub(crate) fn usage(
    connection: &Connection,
    mission_id: Option<&MissionId>,
) -> Result<StorageUsage, LedgerError> {
    let mission = mission_id.map(ToString::to_string);
    let (event_count, event_bytes): (i64, i64) = match mission.as_deref() {
        Some(mission) => connection.query_row(
            "SELECT count(*), coalesce(sum(length(payload) + length(coalesce(raw_evidence, ''))), 0) FROM events WHERE mission_id = ?1",
            [mission],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?,
        None => connection.query_row(
            "SELECT count(*), coalesce(sum(length(payload) + length(coalesce(raw_evidence, ''))), 0) FROM events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?,
    };
    let blob_bytes: i64 = match mission.as_deref() {
        Some(mission) => connection.query_row(
            "SELECT coalesce(sum(b.size), 0) FROM blob_refs b JOIN mission_blob_refs m ON m.blob_hash = b.blob_hash WHERE m.mission_id = ?1",
            [mission],
            |row| row.get(0),
        ).optional()?.unwrap_or(0),
        None => connection.query_row(
            "SELECT coalesce(sum(size), 0) FROM blob_refs WHERE ref_count > 0",
            [],
            |row| row.get(0),
        )?,
    };
    let event_count = u64::try_from(event_count)
        .map_err(|_| LedgerError::IntegrityFailed("negative event count".to_owned()))?;
    let event_bytes = u64::try_from(event_bytes)
        .map_err(|_| LedgerError::IntegrityFailed("negative event bytes".to_owned()))?;
    let blob_bytes = u64::try_from(blob_bytes)
        .map_err(|_| LedgerError::IntegrityFailed("negative blob bytes".to_owned()))?;
    Ok(StorageUsage {
        mission_id: mission,
        event_count,
        event_bytes,
        blob_bytes,
        total_bytes: event_bytes.saturating_add(blob_bytes),
    })
}

impl EncryptedLedger {
    pub fn storage_usage(
        &self,
        mission_id: Option<&MissionId>,
    ) -> Result<StorageUsage, LedgerError> {
        usage(self.connection(), mission_id)
    }

    pub fn retention_plan(&self, budget: &StorageBudget) -> Result<RetentionPlan, LedgerError> {
        self.retention_plan_for_project(budget, None)
    }

    pub fn retention_plan_for_project(
        &self,
        budget: &StorageBudget,
        project_root: Option<&str>,
    ) -> Result<RetentionPlan, LedgerError> {
        let global_usage = usage(self.connection(), None)?;
        let project_missions = project_root
            .map(|root| self.missions_for_project(root))
            .transpose()?;
        let project_usage = match project_missions.as_ref() {
            Some(missions) => {
                missions
                    .iter()
                    .try_fold(StorageUsage::default(), |mut total, mission| {
                        let mission_id: MissionId = mission.parse().map_err(|error| {
                            LedgerError::IntegrityFailed(format!(
                                "invalid mission id {mission}: {error}"
                            ))
                        })?;
                        let item = usage(self.connection(), Some(&mission_id))?;
                        total.event_count = total.event_count.saturating_add(item.event_count);
                        total.event_bytes = total.event_bytes.saturating_add(item.event_bytes);
                        total.blob_bytes = total.blob_bytes.saturating_add(item.blob_bytes);
                        total.total_bytes = total.total_bytes.saturating_add(item.total_bytes);
                        Ok::<_, LedgerError>(total)
                    })?
            }
            None => global_usage.clone(),
        };
        let mut statement = self.connection().prepare(
            "SELECT mission_id, count(*), coalesce(sum(length(payload) + length(coalesce(raw_evidence, ''))), 0) FROM events GROUP BY mission_id ORDER BY mission_id",
        )?;
        let candidate_missions = statement
            .query_map([], |row| {
                let mission: String = row.get(0)?;
                Ok(MissionStorage {
                    mission_id: mission.parse().map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?,
                    event_count: u64::try_from(row.get::<_, i64>(1)?).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?,
                    bytes: u64::try_from(row.get::<_, i64>(2)?).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|mission| {
                project_missions
                    .as_ref()
                    .is_none_or(|missions| missions.contains(&mission.mission_id.to_string()))
            })
            .collect::<Vec<_>>();
        let project_over = budget
            .project_limit_bytes
            .is_some_and(|limit| project_usage.total_bytes > limit);
        let global_over = budget
            .global_limit_bytes
            .is_some_and(|limit| global_usage.total_bytes > limit);
        let mut material = BTreeMap::new();
        material.insert(
            "budget",
            serde_json::to_value(budget).expect("budget serializes"),
        );
        material.insert(
            "project_usage",
            serde_json::to_value(&project_usage).expect("usage serializes"),
        );
        material.insert(
            "global_usage",
            serde_json::to_value(&global_usage).expect("usage serializes"),
        );
        material.insert(
            "candidate_missions",
            serde_json::to_value(&candidate_missions).expect("missions serialize"),
        );
        material.insert(
            "project_root",
            serde_json::Value::String(project_root.unwrap_or("").to_owned()),
        );
        Ok(RetentionPlan {
            budget: budget.clone(),
            project_usage,
            global_usage,
            over_budget: project_over || global_over,
            automatic_deletion: false,
            candidate_missions,
            impact_hash: plan_hash(&serde_json::to_value(material).expect("plan serializes")),
        })
    }

    fn missions_for_project(&self, project_root: &str) -> Result<HashSet<String>, LedgerError> {
        let mut statement = self
            .connection()
            .prepare("SELECT mission_id, payload FROM events WHERE kind = 'mission_created'")?;
        let mut missions = HashSet::new();
        for row in statement.query_map([], |row| {
            let mission: String = row.get(0)?;
            let payload: String = row.get(1)?;
            Ok((mission, payload))
        })? {
            let (mission, payload) = row?;
            let payload: serde_json::Value = serde_json::from_str(&payload).map_err(|error| {
                LedgerError::IntegrityFailed(format!("invalid mission_created payload: {error}"))
            })?;
            if payload
                .get("project_root")
                .and_then(serde_json::Value::as_str)
                == Some(project_root)
            {
                missions.insert(mission);
            }
        }
        Ok(missions)
    }

    pub fn archive_plan(&self, mission_id: MissionId) -> Result<ArchivePlan, LifecycleError> {
        let usage = usage(self.connection(), Some(&mission_id))?;
        if usage.event_count == 0 {
            return Err(LifecycleError::MissionNotFound);
        }
        let mut plan = ArchivePlan {
            mission_id,
            event_count: usage.event_count,
            bytes: usage.total_bytes,
            impact_hash: String::new(),
            created_at: utc_now_rfc3339(),
        };
        let mut hash_material = serde_json::to_value(&plan).expect("archive plan serializes");
        if let Some(object) = hash_material.as_object_mut() {
            object.remove("created_at");
        }
        plan.impact_hash = plan_hash(&hash_material);
        Ok(plan)
    }

    pub fn archive(&mut self, plan: &ArchivePlan) -> Result<AuditReceipt, LifecycleError> {
        let current = self.archive_plan(plan.mission_id)?;
        if current.event_count != plan.event_count
            || current.bytes != plan.bytes
            || current.impact_hash != plan.impact_hash
        {
            return Err(LifecycleError::PlanMismatch);
        }
        let already: Option<String> = self.connection().query_row(
            "SELECT receipt_id FROM lifecycle_audit WHERE operation = 'archive' AND plan_hash = ?1",
            [plan.impact_hash.as_str()], |row| row.get(0),
        ).optional()?;
        if already.is_some() {
            return Err(LifecycleError::PlanAlreadyApplied);
        }
        let archived: i64 = self
            .connection()
            .query_row(
                "SELECT archived FROM mission_lifecycle WHERE mission_id = ?1",
                [plan.mission_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if archived != 0 {
            return Err(LifecycleError::PlanAlreadyApplied);
        }
        let tx = self.connection_mut().transaction()?;
        tx.execute(
            "INSERT INTO mission_lifecycle(mission_id, archived, deleted, archived_at, archive_plan_hash) VALUES (?1, 1, 0, ?2, ?3) ON CONFLICT(mission_id) DO UPDATE SET archived = 1, archived_at = excluded.archived_at, archive_plan_hash = excluded.archive_plan_hash",
            params![plan.mission_id.to_string(), plan.created_at, plan.impact_hash],
        )?;
        let receipt = audit_receipt(
            "archive",
            &plan.mission_id,
            &plan.impact_hash,
            serde_json::to_value(plan).expect("archive plan serializes"),
        );
        tx.execute(
            "INSERT INTO lifecycle_audit(receipt_id, operation, mission_id, plan_hash, created_at, receipt_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![receipt.receipt_id, receipt.operation, receipt.mission_id, receipt.plan_hash, receipt.created_at, serde_json::to_string(&receipt).expect("receipt serializes")],
        )?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn is_archived(&self, mission_id: &MissionId) -> Result<bool, LedgerError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT archived FROM mission_lifecycle WHERE mission_id = ?1",
                [mission_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
            != 0)
    }
}
