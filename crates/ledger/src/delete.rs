use mission_domain::MissionId;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

use crate::retention::usage;
use crate::{EncryptedLedger, LedgerError};

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("ledger: {0}")]
    Ledger(#[from] LedgerError),
    #[error("database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("mission was not found")]
    MissionNotFound,
    #[error("lifecycle plan no longer matches committed data")]
    PlanMismatch,
    #[error("lifecycle plan was already applied")]
    PlanAlreadyApplied,
    #[error("lifecycle serialization failed: {0}")]
    Serialization(String),
    #[error("lifecycle redaction failed: {0}")]
    Redaction(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlobImpact {
    pub hash: String,
    pub size: u64,
    pub media_type: String,
    pub ref_count: u64,
    pub will_remove: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteImpactPlan {
    pub mission_id: MissionId,
    pub event_count: u64,
    pub bytes: u64,
    pub blob_refs: Vec<BlobImpact>,
    pub impact_hash: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuditReceipt {
    pub receipt_id: String,
    pub operation: String,
    pub mission_id: String,
    pub plan_hash: String,
    pub created_at: String,
    pub impact: Value,
}

pub(crate) fn plan_hash(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("lifecycle values serialize"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn audit_receipt(
    operation: &str,
    mission_id: &MissionId,
    plan_hash: &str,
    impact: Value,
) -> AuditReceipt {
    AuditReceipt {
        receipt_id: Uuid::now_v7().to_string(),
        operation: operation.to_owned(),
        mission_id: mission_id.to_string(),
        plan_hash: plan_hash.to_owned(),
        created_at: utc_now_rfc3339(),
        impact,
    }
}

/// Return a UTC RFC3339 timestamp without pulling a date/time dependency into the ledger.
pub(crate) fn utc_now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;

    // Civil date conversion from Unix days (Gregorian calendar).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * doy + 2) / 153;
    let day = doy - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

impl EncryptedLedger {
    pub fn delete_impact(
        &self,
        mission_id: &MissionId,
    ) -> Result<DeleteImpactPlan, LifecycleError> {
        let usage = usage(self.connection(), Some(mission_id))?;
        if usage.event_count == 0 {
            return Err(LifecycleError::MissionNotFound);
        }
        let mut blob_refs = Vec::new();
        let mut statement = self.connection().prepare(
            "SELECT b.blob_hash, b.size, b.media_type, b.ref_count FROM blob_refs b JOIN mission_blob_refs m ON m.blob_hash = b.blob_hash WHERE m.mission_id = ?1 ORDER BY b.blob_hash",
        )?;
        for row in statement.query_map([mission_id.to_string()], |row| {
            let count = row.get::<_, i64>(3)?;
            Ok(BlobImpact {
                hash: row.get(0)?,
                size: u64::try_from(row.get::<_, i64>(1)?)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
                media_type: row.get(2)?,
                ref_count: u64::try_from(count)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
                will_remove: count == 1,
            })
        })? {
            blob_refs.push(row?);
        }
        let mut plan = DeleteImpactPlan {
            mission_id: *mission_id,
            event_count: usage.event_count,
            bytes: usage.total_bytes,
            blob_refs,
            impact_hash: String::new(),
            created_at: crate::delete::utc_now_rfc3339(),
        };
        let mut hash_material = serde_json::to_value(&plan)
            .map_err(|error| LifecycleError::Serialization(error.to_string()))?;
        // Timestamps describe when a preview was generated and must not make an
        // otherwise unchanged plan impossible to confirm on the next request.
        if let Some(object) = hash_material.as_object_mut() {
            object.remove("created_at");
        }
        plan.impact_hash = plan_hash(&hash_material);
        Ok(plan)
    }

    pub fn delete_mission(
        &mut self,
        plan: &DeleteImpactPlan,
    ) -> Result<AuditReceipt, LifecycleError> {
        let already: Option<String> = self.connection().query_row(
            "SELECT receipt_id FROM lifecycle_audit WHERE operation = 'delete' AND plan_hash = ?1", [plan.impact_hash.as_str()], |row| row.get(0),
        ).optional()?;
        if already.is_some() {
            return Err(LifecycleError::PlanAlreadyApplied);
        }
        let current = self.delete_impact(&plan.mission_id)?;
        if current.event_count != plan.event_count
            || current.bytes != plan.bytes
            || current.impact_hash != plan.impact_hash
        {
            return Err(LifecycleError::PlanMismatch);
        }
        let tx = self.connection_mut().transaction()?;
        tx.execute(
            "DELETE FROM events WHERE mission_id = ?1",
            [plan.mission_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM mission_blob_refs WHERE mission_id = ?1",
            [plan.mission_id.to_string()],
        )?;
        for blob in &plan.blob_refs {
            tx.execute(
                "UPDATE blob_refs SET ref_count = max(ref_count - 1, 0) WHERE blob_hash = ?1",
                [&blob.hash],
            )?;
        }
        tx.execute(
            "INSERT INTO mission_lifecycle(mission_id, archived, deleted, archived_at, archive_plan_hash) VALUES (?1, 0, 1, NULL, NULL) ON CONFLICT(mission_id) DO UPDATE SET deleted = 1",
            [plan.mission_id.to_string()],
        )?;
        let receipt = audit_receipt(
            "delete",
            &plan.mission_id,
            &plan.impact_hash,
            serde_json::to_value(plan).expect("delete plan serializes"),
        );
        tx.execute(
            "INSERT INTO lifecycle_audit(receipt_id, operation, mission_id, plan_hash, created_at, receipt_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![receipt.receipt_id, receipt.operation, receipt.mission_id, receipt.plan_hash, receipt.created_at, serde_json::to_string(&receipt).expect("receipt serializes")],
        )?;
        tx.commit()?;
        Ok(receipt)
    }
}
