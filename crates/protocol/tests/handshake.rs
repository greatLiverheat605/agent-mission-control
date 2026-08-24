use std::io::{self, Cursor};
use std::time::{Duration, Instant};

use mission_protocol::frame::{FrameError, MAX_FRAME_SIZE, read_frame, write_frame};
use mission_protocol::handshake::{
    Handshake, HandshakeVerifier, InstallSecretProvider, MAX_CACHED_NONCES, NONCE_BYTES, NONCE_TTL,
    PROTOCOL_VERSION, ProtocolErrorCode, handshake_proof,
};
use serde::{Deserialize, Serialize};

const INSTALL_ID: &str = "9f3628e6-2c77-4815-91cc-213e92e07726";
const FIXTURE_SECRET: &[u8] = b"phase-1-fixture-secret-never-log";

#[derive(Clone, Copy)]
struct FixtureSecret;

impl InstallSecretProvider for FixtureSecret {
    fn install_secret(&self) -> io::Result<Vec<u8>> {
        Ok(FIXTURE_SECRET.to_vec())
    }
}

fn nonce(index: u64) -> Vec<u8> {
    let mut nonce = vec![0_u8; NONCE_BYTES];
    nonce[..size_of::<u64>()].copy_from_slice(&index.to_le_bytes());
    nonce
}

fn signed_handshake(index: u64, versions: Vec<u32>) -> Handshake {
    let nonce = nonce(index);
    let proof = handshake_proof(FIXTURE_SECRET, INSTALL_ID, &nonce, &versions)
        .expect("fixture handshake can be signed");
    Handshake {
        install_id: INSTALL_ID.to_owned(),
        protocol_versions: versions,
        nonce,
        proof,
    }
}

#[test]
fn valid_proof_negotiates_protocol_version_one() {
    let now = Instant::now();
    let mut verifier = HandshakeVerifier::new(INSTALL_ID, FixtureSecret);

    let accepted = verifier
        .verify_at(&signed_handshake(1, vec![PROTOCOL_VERSION]), now)
        .expect("valid handshake is accepted");

    assert_eq!(accepted.protocol_version, PROTOCOL_VERSION);
}

#[test]
fn wrong_proof_is_rejected_without_consuming_the_nonce() {
    let now = Instant::now();
    let mut verifier = HandshakeVerifier::new(INSTALL_ID, FixtureSecret);
    let mut handshake = signed_handshake(2, vec![PROTOCOL_VERSION]);
    handshake.proof[0] ^= 0xff;

    let error = verifier
        .verify_at(&handshake, now)
        .expect_err("wrong proof is rejected");
    assert_eq!(error.code, ProtocolErrorCode::AuthFailed);

    verifier
        .verify_at(&signed_handshake(2, vec![PROTOCOL_VERSION]), now)
        .expect("a failed proof does not consume its nonce");
}

#[test]
fn replayed_nonce_is_rejected_for_ten_minutes() {
    let now = Instant::now();
    let mut verifier = HandshakeVerifier::new(INSTALL_ID, FixtureSecret);
    let handshake = signed_handshake(3, vec![PROTOCOL_VERSION]);
    verifier
        .verify_at(&handshake, now)
        .expect("first nonce use succeeds");

    let error = verifier
        .verify_at(&handshake, now + NONCE_TTL - Duration::from_millis(1))
        .expect_err("nonce cannot be replayed inside the retention window");

    assert_eq!(error.code, ProtocolErrorCode::ReplayedNonce);
}

#[test]
fn expired_nonce_is_removed_without_sleeping() {
    let now = Instant::now();
    let mut verifier = HandshakeVerifier::new(INSTALL_ID, FixtureSecret);
    let handshake = signed_handshake(4, vec![PROTOCOL_VERSION]);
    verifier
        .verify_at(&handshake, now)
        .expect("first nonce use succeeds");

    verifier
        .verify_at(&handshake, now + NONCE_TTL)
        .expect("nonce is reusable once its retention expires");
}

#[test]
fn nonce_cache_fails_closed_at_4096_entries() {
    let now = Instant::now();
    let mut verifier = HandshakeVerifier::new(INSTALL_ID, FixtureSecret);
    for index in 0..MAX_CACHED_NONCES as u64 {
        verifier
            .verify_at(&signed_handshake(index, vec![PROTOCOL_VERSION]), now)
            .expect("cache accepts entries up to its fixed limit");
    }

    let overflow = verifier
        .verify_at(
            &signed_handshake(MAX_CACHED_NONCES as u64, vec![PROTOCOL_VERSION]),
            now,
        )
        .expect_err("cache rejects new nonces instead of evicting live entries");
    assert_eq!(overflow.code, ProtocolErrorCode::AuthFailed);

    let replay = verifier
        .verify_at(&signed_handshake(0, vec![PROTOCOL_VERSION]), now)
        .expect_err("the oldest live nonce remains protected");
    assert_eq!(replay.code, ProtocolErrorCode::ReplayedNonce);

    verifier
        .verify_at(
            &signed_handshake(MAX_CACHED_NONCES as u64, vec![PROTOCOL_VERSION]),
            now + NONCE_TTL,
        )
        .expect("expiry deterministically frees cache capacity");
}

#[test]
fn incompatible_version_is_rejected() {
    let mut verifier = HandshakeVerifier::new(INSTALL_ID, FixtureSecret);

    let error = verifier
        .verify_at(&signed_handshake(5, vec![2, 3]), Instant::now())
        .expect_err("non-overlapping protocol versions are rejected");

    assert_eq!(error.code, ProtocolErrorCode::IncompatibleProtocol);
}

#[test]
fn invalid_install_id_or_nonce_length_is_an_auth_failure() {
    let now = Instant::now();
    let mut verifier = HandshakeVerifier::new(INSTALL_ID, FixtureSecret);
    let mut wrong_install = signed_handshake(6, vec![PROTOCOL_VERSION]);
    wrong_install.install_id = "different-install".to_owned();
    assert_eq!(
        verifier
            .verify_at(&wrong_install, now)
            .expect_err("install ID is fixed")
            .code,
        ProtocolErrorCode::AuthFailed
    );

    let mut short_nonce = signed_handshake(7, vec![PROTOCOL_VERSION]);
    short_nonce.nonce.pop();
    assert_eq!(
        verifier
            .verify_at(&short_nonce, now)
            .expect_err("nonce is exactly 32 bytes")
            .code,
        ProtocolErrorCode::AuthFailed
    );
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ExampleFrame {
    message: String,
}

#[test]
fn frame_is_little_endian_length_prefixed_utf8_json() {
    let value = ExampleFrame {
        message: "hello".to_owned(),
    };
    let mut encoded = Vec::new();

    write_frame(&mut encoded, &value).expect("frame can be encoded");

    let json = serde_json::to_vec(&value).expect("fixture serializes");
    assert_eq!(&encoded[..4], &(json.len() as u32).to_le_bytes());
    assert_eq!(&encoded[4..], json);
    assert_eq!(
        read_frame::<_, ExampleFrame>(&mut Cursor::new(encoded)).expect("frame can be decoded"),
        value
    );
}

#[test]
fn oversized_frame_is_rejected_before_reading_its_body() {
    let mut only_a_header = Cursor::new(((MAX_FRAME_SIZE + 1) as u32).to_le_bytes());

    let error = read_frame::<_, serde_json::Value>(&mut only_a_header)
        .expect_err("oversized length is rejected without reading a payload");

    assert!(matches!(error, FrameError::FrameTooLarge));
}

#[test]
fn oversized_outbound_json_is_rejected() {
    let value = "x".repeat(MAX_FRAME_SIZE + 1);

    let error =
        write_frame(&mut Vec::new(), &value).expect_err("oversized outbound JSON is rejected");

    assert!(matches!(error, FrameError::FrameTooLarge));
}

#[test]
fn malformed_payloads_return_fixed_errors() {
    let invalid_utf8 = vec![1, 0, 0, 0, 0xff];
    assert!(matches!(
        read_frame::<_, serde_json::Value>(&mut invalid_utf8.as_slice()),
        Err(FrameError::InvalidUtf8)
    ));

    let invalid_json = vec![1, 0, 0, 0, b'{'];
    assert!(matches!(
        read_frame::<_, serde_json::Value>(&mut invalid_json.as_slice()),
        Err(FrameError::InvalidJson)
    ));
}
