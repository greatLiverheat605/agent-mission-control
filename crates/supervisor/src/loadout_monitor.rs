use adapter_core::LoadoutSnapshot;
use mission_domain::MissionId;
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadoutChange {
    pub previous_fingerprint: String,
    pub next_fingerprint: String,
}

#[derive(Clone, Debug, Default)]
pub struct LoadoutMonitor {
    frozen: HashMap<String, String>,
}

impl LoadoutMonitor {
    pub fn freeze(
        &mut self,
        mission_id: &MissionId,
        snapshot: &LoadoutSnapshot,
    ) -> Result<String, &'static str> {
        let fingerprint = fingerprint(snapshot)?;
        self.frozen
            .insert(mission_id.to_string(), fingerprint.clone());
        Ok(fingerprint)
    }

    pub fn freeze_fingerprint(
        &mut self,
        mission_id: &MissionId,
        fingerprint: impl Into<String>,
    ) -> Result<(), &'static str> {
        let fingerprint = fingerprint.into();
        if fingerprint.trim().is_empty() || fingerprint.len() > 32_768 {
            return Err("LOADOUT_INVALID");
        }
        self.frozen.insert(mission_id.to_string(), fingerprint);
        Ok(())
    }

    pub fn check_before_model_request(
        &self,
        mission_id: &MissionId,
        snapshot: &LoadoutSnapshot,
    ) -> Result<Option<LoadoutChange>, &'static str> {
        let next = fingerprint(snapshot)?;
        let previous = self
            .frozen
            .get(&mission_id.to_string())
            .ok_or("LOADOUT_NOT_FROZEN")?;
        if previous == &next {
            return Ok(None);
        }
        Ok(Some(LoadoutChange {
            previous_fingerprint: previous.clone(),
            next_fingerprint: next,
        }))
    }

    pub fn update_after_change(
        &mut self,
        mission_id: &MissionId,
        snapshot: &LoadoutSnapshot,
    ) -> Result<(), &'static str> {
        let next = fingerprint(snapshot)?;
        self.frozen.insert(mission_id.to_string(), next);
        Ok(())
    }

    pub fn check_fingerprint(
        &self,
        mission_id: &MissionId,
        next: &str,
    ) -> Result<Option<LoadoutChange>, &'static str> {
        if next.trim().is_empty() || next.len() > 32_768 {
            return Err("LOADOUT_INVALID");
        }
        let previous = self
            .frozen
            .get(&mission_id.to_string())
            .ok_or("LOADOUT_NOT_FROZEN")?;
        if previous == next {
            return Ok(None);
        }
        Ok(Some(LoadoutChange {
            previous_fingerprint: previous.clone(),
            next_fingerprint: next.to_owned(),
        }))
    }
}

pub fn fingerprint(snapshot: &LoadoutSnapshot) -> Result<String, &'static str> {
    if snapshot.provider.as_str().is_empty()
        || snapshot
            .fingerprint_material()
            .iter()
            .any(|value| value.len() > 4096)
    {
        return Err("LOADOUT_INVALID");
    }
    Ok(snapshot.fingerprint_material().join("\u{1f}"))
}

#[cfg(test)]
mod tests {
    use super::LoadoutMonitor;
    use adapter_core::{LoadoutSnapshot, ProviderId};
    use mission_domain::MissionId;

    fn snapshot(config: &str) -> LoadoutSnapshot {
        LoadoutSnapshot {
            provider: ProviderId::Claude,
            model: Some("claude-sonnet".to_owned()),
            config_fingerprint: config.to_owned(),
            hooks_fingerprint: "hooks".to_owned(),
            skills_fingerprint: "skills".to_owned(),
            plugins_fingerprint: "plugins".to_owned(),
            mcp_fingerprint: "mcp".to_owned(),
        }
    }

    #[test]
    fn freezes_and_detects_next_request_change() {
        let mission = MissionId::new();
        let mut monitor = LoadoutMonitor::default();
        monitor.freeze(&mission, &snapshot("v1")).expect("freeze");
        assert!(
            monitor
                .check_before_model_request(&mission, &snapshot("v1"))
                .expect("check")
                .is_none()
        );
        let change = monitor
            .check_before_model_request(&mission, &snapshot("v2"))
            .expect("check")
            .expect("change");
        assert!(change.previous_fingerprint.contains("v1"));
        assert!(change.next_fingerprint.contains("v2"));
    }

    #[test]
    fn rejects_unfrozen_or_oversized_fingerprints() {
        let mission = MissionId::new();
        let monitor = LoadoutMonitor::default();
        assert_eq!(
            monitor.check_fingerprint(&mission, "next"),
            Err("LOADOUT_NOT_FROZEN")
        );

        let mut monitor = LoadoutMonitor::default();
        monitor
            .freeze_fingerprint(&mission, "frozen")
            .expect("freeze");
        assert_eq!(
            monitor.check_fingerprint(&mission, &"x".repeat(32_769)),
            Err("LOADOUT_INVALID")
        );
    }
}
