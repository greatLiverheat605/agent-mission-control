use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaudeNativeEvent {
    pub event_type: String,
    pub raw: Value,
}

#[derive(Debug, Error)]
pub enum ClaudeParseError {
    #[error("stream line is empty")]
    Empty,
    #[error("stream line is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("stream event is not a JSON object")]
    NotObject,
    #[error("stream event is missing a string type")]
    MissingType,
}

pub fn parse_stream_line(line: &str) -> Result<ClaudeNativeEvent, ClaudeParseError> {
    if line.trim().is_empty() {
        return Err(ClaudeParseError::Empty);
    }
    let raw: Value = serde_json::from_str(line)?;
    let object = raw.as_object().ok_or(ClaudeParseError::NotObject)?;
    let event_type = object
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ClaudeParseError::MissingType)?
        .to_owned();
    Ok(ClaudeNativeEvent { event_type, raw })
}
