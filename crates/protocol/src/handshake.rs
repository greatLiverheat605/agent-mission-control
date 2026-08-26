use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub const PROTOCOL_VERSION: u32 = 1;
pub const PRODUCT_INSTALL_ID: &str = "mission-control-desktop-v1";
pub const NONCE_BYTES: usize = 32;
pub const NONCE_TTL: Duration = Duration::from_secs(10 * 60);
pub const MAX_CACHED_NONCES: usize = 4096;

type HmacSha256 = Hmac<Sha256>;

pub trait InstallSecretProvider {
    fn install_secret(&self) -> io::Result<Vec<u8>>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Handshake {
    pub install_id: String,
    pub protocol_versions: Vec<u32>,
    pub nonce: Vec<u8>,
    pub proof: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HandshakeAccepted {
    pub protocol_version: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Pong {
    pub supervisor_version: String,
    pub protocol_version: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandRequest {
    pub command: String,
    pub request: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandResponse {
    pub command: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum ClientMessage {
    Handshake(Handshake),
    Ping,
    Command(CommandRequest),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerMessage {
    HandshakeAccepted(HandshakeAccepted),
    Pong(Pong),
    Command(CommandResponse),
    Error(ProtocolError),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtocolErrorCode {
    AuthFailed,
    ReplayedNonce,
    FrameTooLarge,
    IncompatibleProtocol,
    InvalidFrame,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
}

impl ProtocolError {
    pub const fn new(code: ProtocolErrorCode) -> Self {
        Self { code }
    }
}

pub fn handshake_proof(
    secret: &[u8],
    install_id: &str,
    nonce: &[u8],
    protocol_versions: &[u32],
) -> Result<Vec<u8>, hmac::digest::InvalidLength> {
    let mut mac = HmacSha256::new_from_slice(secret)?;
    update_proof(&mut mac, install_id, nonce, protocol_versions);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn update_proof(mac: &mut HmacSha256, install_id: &str, nonce: &[u8], protocol_versions: &[u32]) {
    mac.update(b"mission-control-handshake-v1\0");
    mac.update(&(install_id.len() as u32).to_le_bytes());
    mac.update(install_id.as_bytes());
    mac.update(&(nonce.len() as u32).to_le_bytes());
    mac.update(nonce);
    mac.update(&(protocol_versions.len() as u32).to_le_bytes());
    for version in protocol_versions {
        mac.update(&version.to_le_bytes());
    }
}

pub struct HandshakeVerifier<P> {
    expected_install_id: String,
    secret_provider: P,
    nonces: VecDeque<([u8; NONCE_BYTES], Instant)>,
}

impl<P: InstallSecretProvider> HandshakeVerifier<P> {
    pub fn new(expected_install_id: impl Into<String>, secret_provider: P) -> Self {
        Self {
            expected_install_id: expected_install_id.into(),
            secret_provider,
            nonces: VecDeque::new(),
        }
    }

    pub fn verify_at(
        &mut self,
        handshake: &Handshake,
        now: Instant,
    ) -> Result<HandshakeAccepted, ProtocolError> {
        self.remove_expired(now);

        if !handshake.protocol_versions.contains(&PROTOCOL_VERSION) {
            return Err(ProtocolError::new(ProtocolErrorCode::IncompatibleProtocol));
        }
        if handshake.install_id != self.expected_install_id {
            return Err(ProtocolError::new(ProtocolErrorCode::AuthFailed));
        }
        let nonce: [u8; NONCE_BYTES] = handshake
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| ProtocolError::new(ProtocolErrorCode::AuthFailed))?;
        if self.nonces.iter().any(|(seen, _)| seen == &nonce) {
            return Err(ProtocolError::new(ProtocolErrorCode::ReplayedNonce));
        }
        if self.nonces.len() == MAX_CACHED_NONCES {
            return Err(ProtocolError::new(ProtocolErrorCode::AuthFailed));
        }

        let secret = self
            .secret_provider
            .install_secret()
            .map_err(|_| ProtocolError::new(ProtocolErrorCode::AuthFailed))?;
        let mut mac = HmacSha256::new_from_slice(&secret)
            .map_err(|_| ProtocolError::new(ProtocolErrorCode::AuthFailed))?;
        update_proof(
            &mut mac,
            &handshake.install_id,
            &handshake.nonce,
            &handshake.protocol_versions,
        );
        mac.verify_slice(&handshake.proof)
            .map_err(|_| ProtocolError::new(ProtocolErrorCode::AuthFailed))?;

        self.nonces.push_back((nonce, now));
        Ok(HandshakeAccepted {
            protocol_version: PROTOCOL_VERSION,
        })
    }

    fn remove_expired(&mut self, now: Instant) {
        while self
            .nonces
            .front()
            .is_some_and(|(_, inserted)| now.saturating_duration_since(*inserted) >= NONCE_TTL)
        {
            self.nonces.pop_front();
        }
    }
}
