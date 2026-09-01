use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

const DEFAULT_MAX_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RedactionAudit {
    pub replacement_count: usize,
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionResult {
    pub value: Value,
    pub audit: RedactionAudit,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RedactionError {
    #[error("REDACTION_LIMIT_EXCEEDED")]
    LimitExceeded,
    #[error("redaction input is not a valid JSON value")]
    InvalidInput,
}

#[derive(Clone, Debug)]
pub struct Redactor {
    max_bytes: usize,
    max_depth: usize,
}

impl Default for Redactor {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

impl Redactor {
    pub fn new(max_bytes: usize, max_depth: usize) -> Self {
        Self {
            max_bytes,
            max_depth,
        }
    }

    /// Redaction is deliberately the only public transformation entry point.
    pub fn redact_event(&self, value: Value) -> Result<RedactionResult, RedactionError> {
        let input_size = serde_json::to_vec(&value)
            .map_err(|_| RedactionError::InvalidInput)?
            .len();
        if input_size > self.max_bytes {
            return Err(RedactionError::LimitExceeded);
        }
        let mut audit = RedactionAudit::default();
        let value = self.redact_value(value, 0, &mut audit)?;
        let output_size = serde_json::to_vec(&value)
            .map_err(|_| RedactionError::InvalidInput)?
            .len();
        if output_size > self.max_bytes {
            return Err(RedactionError::LimitExceeded);
        }
        Ok(RedactionResult { value, audit })
    }

    fn redact_value(
        &self,
        value: Value,
        depth: usize,
        audit: &mut RedactionAudit,
    ) -> Result<Value, RedactionError> {
        if depth > self.max_depth {
            return Err(RedactionError::LimitExceeded);
        }
        Ok(match value {
            Value::Object(object) => Value::Object(self.redact_object(object, depth, audit)?),
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| self.redact_value(value, depth + 1, audit))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Value::String(text) => Value::String(redact_text(&text, audit)),
            other => other,
        })
    }

    fn redact_object(
        &self,
        object: Map<String, Value>,
        depth: usize,
        audit: &mut RedactionAudit,
    ) -> Result<Map<String, Value>, RedactionError> {
        let mut output = Map::new();
        for (key, value) in object {
            if is_sensitive_field(&key) {
                let category = field_category(&key);
                let bytes = serde_json::to_vec(&value).map_err(|_| RedactionError::InvalidInput)?;
                output.insert(key, Value::String(marker(category, &bytes, audit)));
            } else {
                output.insert(key, self.redact_value(value, depth + 1, audit)?);
            }
        }
        Ok(output)
    }
}

fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r"-----BEGIN [^-]+-----[\s\S]+?-----END [^-]+-----").unwrap(),
                "private_key",
            ),
            (
                Regex::new(r"(?i)https?://[^/\s:@]+:[^/\s@]+@[^\s/]+").unwrap(),
                "url_userinfo",
            ),
            (
                Regex::new(r"(?i)(?:cookie|set-cookie):\s*[^\r\n]+").unwrap(),
                "cookie",
            ),
            (
                Regex::new(r"(?i)bearer\s+[A-Za-z0-9._~+/-]+=*").unwrap(),
                "bearer",
            ),
            (Regex::new(r"(?i)basic\s+[A-Za-z0-9+/=]+").unwrap(), "basic"),
            (
                Regex::new(r"sk-ant-[A-Za-z0-9_-]{16,}").unwrap(),
                "anthropic_key",
            ),
            (Regex::new(r"sk-[A-Za-z0-9_-]{16,}").unwrap(), "openai_key"),
            (
                Regex::new(r"gh[pousr]_[A-Za-z0-9]{20,}").unwrap(),
                "github_token",
            ),
            (Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(), "aws_key"),
            (
                Regex::new(r"(?i)(?:OPENAI|ANTHROPIC|AWS|GITHUB)[A-Z0-9_]*\s*=\s*[^\s,;]+")
                    .unwrap(),
                "env_secret",
            ),
        ]
    })
}

fn redact_text(text: &str, audit: &mut RedactionAudit) -> String {
    let mut output = text.to_owned();
    for (pattern, category) in patterns() {
        output = pattern
            .replace_all(&output, |captures: &regex::Captures<'_>| {
                marker(category, captures[0].as_bytes(), audit)
            })
            .into_owned();
    }
    output
}

fn is_sensitive_field(key: &str) -> bool {
    let key = normalize_key(key);
    [
        "token",
        "secret",
        "password",
        "authorization",
        "cookie",
        "api_key",
        "apikey",
        "private_key",
        "credential",
    ]
    .iter()
    .any(|needle| key == *needle || key.ends_with(&format!("_{needle}")))
}

fn field_category(key: &str) -> &'static str {
    let key = normalize_key(key);
    if key.contains("password") {
        "password"
    } else if key.contains("token") {
        "token"
    } else if key.contains("private") {
        "private_key"
    } else if key.contains("cookie") {
        "cookie"
    } else if key.contains("authorization") {
        "authorization"
    } else {
        "secret_field"
    }
}

fn normalize_key(key: &str) -> String {
    let mut normalized = String::with_capacity(key.len() + 4);
    let mut previous_lowercase = false;
    for character in key.chars() {
        if character.is_ascii_uppercase() {
            if previous_lowercase {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
            previous_lowercase = false;
        } else {
            normalized.push(character.to_ascii_lowercase());
            previous_lowercase = character.is_ascii_lowercase() || character.is_ascii_digit();
        }
    }
    for plural in [
        "tokens",
        "secrets",
        "passwords",
        "authorizations",
        "cookies",
        "api_keys",
        "apikeys",
        "private_keys",
        "credentials",
    ] {
        if normalized.ends_with(plural) {
            normalized.truncate(normalized.len() - 1);
            break;
        }
    }
    normalized
}

fn marker(category: &str, secret: &[u8], audit: &mut RedactionAudit) -> String {
    let digest = Sha256::digest(secret);
    let prefix: String = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    audit.replacement_count += 1;
    if !audit.categories.iter().any(|value| value == category) {
        audit.categories.push(category.to_owned());
    }
    format!("[REDACTED:{category}:{prefix}]")
}
