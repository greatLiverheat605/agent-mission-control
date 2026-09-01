#![allow(dead_code)]

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManifest {
    pub channel: String,
    pub version: String,
    pub artifact_sha256: String,
    pub signer_fingerprint: String,
    #[serde(default)]
    pub signature_scheme: String,
    pub signature: String,
    pub schema_version: u64,
    pub min_schema_version: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateError {
    Unsigned,
    UntrustedSigner,
    InvalidSignature,
    ArtifactHashMismatch,
    ChannelMismatch,
    Downgrade,
    InvalidVersion,
    SchemaIncompatible,
    ActiveMission,
    BackupMissing,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::Unsigned => "UPDATE_UNSIGNED",
            Self::UntrustedSigner => "UPDATE_UNTRUSTED_SIGNER",
            Self::InvalidSignature => "UPDATE_SIGNATURE_INVALID",
            Self::ArtifactHashMismatch => "UPDATE_ARTIFACT_HASH_MISMATCH",
            Self::ChannelMismatch => "UPDATE_CHANNEL_MISMATCH",
            Self::Downgrade => "UPDATE_DOWNGRADE",
            Self::InvalidVersion => "UPDATE_VERSION_INVALID",
            Self::SchemaIncompatible => "UPDATE_SCHEMA_INCOMPATIBLE",
            Self::ActiveMission => "UPDATE_ACTIVE_MISSION",
            Self::BackupMissing => "UPDATE_BACKUP_MISSING",
        };
        formatter.write_str(code)
    }
}

impl std::error::Error for UpdateError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateGuard {
    channel: String,
    current_version: String,
    schema_version: u64,
    trusted_signer: String,
    allow_fixture_signatures: bool,
}

impl UpdateGuard {
    pub fn new(
        channel: impl Into<String>,
        current_version: impl Into<String>,
        schema_version: u64,
        trusted_signer: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            current_version: current_version.into(),
            schema_version,
            trusted_signer: trusted_signer.into(),
            allow_fixture_signatures: false,
        }
    }

    #[cfg(test)]
    fn for_fixture_tests(
        channel: impl Into<String>,
        current_version: impl Into<String>,
        schema_version: u64,
        trusted_signer: impl Into<String>,
    ) -> Self {
        let mut guard = Self::new(channel, current_version, schema_version, trusted_signer);
        guard.allow_fixture_signatures = true;
        guard
    }

    pub fn validate(
        &self,
        manifest: &UpdateManifest,
        artifact: &[u8],
        active_mission: bool,
    ) -> Result<(), UpdateError> {
        self.validate_common(manifest, artifact, active_mission)?;
        if manifest.signature_scheme != "fixture-sha256-v1" || !self.allow_fixture_signatures {
            // Production signatures are verified by Tauri's updater before this guard is
            // called. Keeping the deterministic fixture scheme test-only prevents a hash
            // from being mistaken for a private-key signature.
            return Err(UpdateError::InvalidSignature);
        }
        let actual_hash = sha256_hex(artifact);
        let expected_signature = detached_signature(&self.trusted_signer, &actual_hash);
        if !constant_time_eq(manifest.signature.as_bytes(), expected_signature.as_bytes()) {
            return Err(UpdateError::InvalidSignature);
        }
        Ok(())
    }

    /// Validate the manifest after Tauri updater has verified its asymmetric signature.
    /// `signature_verified` must only come from that updater, never from manifest data.
    pub fn validate_verified(
        &self,
        manifest: &UpdateManifest,
        artifact: &[u8],
        active_mission: bool,
        signature_verified: bool,
    ) -> Result<(), UpdateError> {
        self.validate_common(manifest, artifact, active_mission)?;
        if manifest.signature_scheme != "tauri-updater-v1" || !signature_verified {
            return Err(UpdateError::InvalidSignature);
        }
        Ok(())
    }

    fn validate_common(
        &self,
        manifest: &UpdateManifest,
        artifact: &[u8],
        active_mission: bool,
    ) -> Result<(), UpdateError> {
        if manifest.channel != self.channel {
            return Err(UpdateError::ChannelMismatch);
        }
        if compare_versions(&manifest.version, &self.current_version)?
            != std::cmp::Ordering::Greater
        {
            return Err(UpdateError::Downgrade);
        }
        if manifest.schema_version == 0
            || manifest.min_schema_version > self.schema_version
            || manifest.min_schema_version > manifest.schema_version
        {
            return Err(UpdateError::SchemaIncompatible);
        }
        if manifest.signer_fingerprint != self.trusted_signer {
            return Err(UpdateError::UntrustedSigner);
        }
        if manifest.signature.trim().is_empty() {
            return Err(UpdateError::Unsigned);
        }
        let actual_hash = sha256_hex(artifact);
        if !constant_time_eq(manifest.artifact_sha256.as_bytes(), actual_hash.as_bytes()) {
            return Err(UpdateError::ArtifactHashMismatch);
        }
        if active_mission {
            return Err(UpdateError::ActiveMission);
        }
        Ok(())
    }

    pub fn require_backup(&self, backup: &Path) -> Result<(), UpdateError> {
        match std::fs::metadata(backup) {
            Ok(metadata) if metadata.is_file() => Ok(()),
            _ => Err(UpdateError::BackupMissing),
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn detached_signature(signer_fingerprint: &str, artifact_sha256: &str) -> String {
    sha256_hex(
        format!("mission-control-update-v1:{signer_fingerprint}:{artifact_sha256}").as_bytes(),
    )
}

fn compare_versions(left: &str, right: &str) -> Result<std::cmp::Ordering, UpdateError> {
    let parse = |value: &str| {
        let mut parts = value.split('.');
        let major = parts
            .next()
            .ok_or(UpdateError::InvalidVersion)?
            .parse::<u64>()
            .map_err(|_| UpdateError::InvalidVersion)?;
        let minor = parts
            .next()
            .ok_or(UpdateError::InvalidVersion)?
            .parse::<u64>()
            .map_err(|_| UpdateError::InvalidVersion)?;
        let patch = parts
            .next()
            .ok_or(UpdateError::InvalidVersion)?
            .parse::<u64>()
            .map_err(|_| UpdateError::InvalidVersion)?;
        if parts.next().is_some() {
            return Err(UpdateError::InvalidVersion);
        }
        Ok((major, minor, patch))
    };
    Ok(parse(left)?.cmp(&parse(right)?))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn manifest(artifact: &[u8]) -> UpdateManifest {
        let artifact_hash = sha256_hex(artifact);
        UpdateManifest {
            channel: "preview".to_owned(),
            version: "0.2.0".to_owned(),
            artifact_sha256: artifact_hash.clone(),
            signer_fingerprint: "TEST-SIGNER".to_owned(),
            signature_scheme: "fixture-sha256-v1".to_owned(),
            signature: detached_signature("TEST-SIGNER", &artifact_hash),
            schema_version: 1,
            min_schema_version: 1,
        }
    }

    #[test]
    fn rejects_unsigned_wrong_key_and_hash_mismatch() {
        let artifact = b"codex-preview";
        let mut unsigned = manifest(artifact);
        unsigned.signature.clear();
        assert_eq!(
            UpdateGuard::for_fixture_tests("preview", "0.1.0", 1, "TEST-SIGNER")
                .validate(&unsigned, artifact, false)
                .unwrap_err(),
            UpdateError::Unsigned
        );

        let mut wrong_key = manifest(artifact);
        wrong_key.signer_fingerprint = "OTHER-SIGNER".to_owned();
        assert_eq!(
            UpdateGuard::for_fixture_tests("preview", "0.1.0", 1, "TEST-SIGNER")
                .validate(&wrong_key, artifact, false)
                .unwrap_err(),
            UpdateError::UntrustedSigner
        );

        let mut wrong_hash = manifest(artifact);
        wrong_hash.artifact_sha256 = sha256_hex(b"different");
        assert_eq!(
            UpdateGuard::for_fixture_tests("preview", "0.1.0", 1, "TEST-SIGNER")
                .validate(&wrong_hash, artifact, false)
                .unwrap_err(),
            UpdateError::ArtifactHashMismatch
        );
    }

    #[test]
    fn rejects_downgrade_channel_schema_and_active_mission() {
        let artifact = b"codex-preview";
        let guard = UpdateGuard::for_fixture_tests("preview", "0.2.0", 1, "TEST-SIGNER");

        let mut downgrade = manifest(artifact);
        downgrade.version = "0.1.9".to_owned();
        downgrade.signature = detached_signature("TEST-SIGNER", &downgrade.artifact_sha256);
        assert_eq!(
            guard.validate(&downgrade, artifact, false).unwrap_err(),
            UpdateError::Downgrade
        );

        let mut channel = manifest(artifact);
        channel.channel = "stable".to_owned();
        channel.signature = detached_signature("TEST-SIGNER", &channel.artifact_sha256);
        assert_eq!(
            guard.validate(&channel, artifact, false).unwrap_err(),
            UpdateError::ChannelMismatch
        );

        let mut schema = manifest(artifact);
        schema.version = "0.3.0".to_owned();
        schema.min_schema_version = 2;
        schema.signature = detached_signature("TEST-SIGNER", &schema.artifact_sha256);
        assert_eq!(
            guard.validate(&schema, artifact, false).unwrap_err(),
            UpdateError::SchemaIncompatible
        );

        assert_eq!(
            UpdateGuard::for_fixture_tests("preview", "0.1.0", 1, "TEST-SIGNER")
                .validate(&manifest(artifact), artifact, true)
                .unwrap_err(),
            UpdateError::ActiveMission
        );
    }

    #[test]
    fn requires_backup_before_install_and_accepts_compatible_manifest() {
        let artifact = b"codex-preview";
        let guard = UpdateGuard::for_fixture_tests("preview", "0.1.0", 1, "TEST-SIGNER");
        assert!(guard.validate(&manifest(artifact), artifact, false).is_ok());

        let path =
            std::env::temp_dir().join(format!("mission-update-backup-{}", std::process::id()));
        assert_eq!(
            guard.require_backup(&path).unwrap_err(),
            UpdateError::BackupMissing
        );
        fs::write(&path, b"backup").unwrap();
        assert!(guard.require_backup(&path).is_ok());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn production_guard_requires_tauri_verified_signature() {
        let artifact = b"codex-preview";
        let mut tauri_manifest = manifest(artifact);
        tauri_manifest.signature_scheme = "tauri-updater-v1".to_owned();
        tauri_manifest.signature = "tauri-detached-signature".to_owned();
        let guard = UpdateGuard::new("preview", "0.1.0", 1, "TEST-SIGNER");
        assert_eq!(
            guard
                .validate(&tauri_manifest, artifact, false)
                .unwrap_err(),
            UpdateError::InvalidSignature
        );
        assert!(
            guard
                .validate_verified(&tauri_manifest, artifact, false, true)
                .is_ok()
        );
        assert_eq!(
            guard
                .validate_verified(&tauri_manifest, artifact, false, false)
                .unwrap_err(),
            UpdateError::InvalidSignature
        );
    }
}
