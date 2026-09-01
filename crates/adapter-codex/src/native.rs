use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(flatten)]
    pub fields: serde_json::Map<String, Value>,
}

impl NativeEvent {
    pub fn raw_value(&self) -> Value {
        let mut object = self.fields.clone();
        object.insert("type".to_owned(), Value::String(self.event_type.clone()));
        if let Some(id) = &self.id {
            object.insert("id".to_owned(), Value::String(id.clone()));
        }
        Value::Object(object)
    }
}

#[derive(Debug, Error)]
pub enum NativeParseError {
    #[error("native event is not a JSON object")]
    NotObject,
    #[error("native event has no type discriminator")]
    MissingType,
    #[error("invalid native JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("native event exceeds protocol limits")]
    LimitExceeded,
}

pub fn parse_native_line(line: &str) -> Result<NativeEvent, NativeParseError> {
    if line.len() > 4 * 1024 * 1024 {
        return Err(NativeParseError::LimitExceeded);
    }
    let value: Value = serde_json::from_str(line)?;
    if json_depth(&value) > 64 {
        return Err(NativeParseError::LimitExceeded);
    }
    let object = value.as_object().ok_or(NativeParseError::NotObject)?;
    let event_type = object
        .get("type")
        .or_else(|| object.get("method"))
        .and_then(Value::as_str)
        .ok_or(NativeParseError::MissingType)?;
    if event_type.is_empty() || event_type.len() > 256 {
        return Err(NativeParseError::LimitExceeded);
    }
    let mut fields = object.clone();
    fields.remove("type");
    let id = fields
        .remove("id")
        .and_then(|value| value.as_str().map(ToOwned::to_owned));
    Ok(NativeEvent {
        event_type: event_type.to_owned(),
        id,
        fields,
    })
}

fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.iter().map(json_depth).max().unwrap_or(0) + 1,
        Value::Object(values) => values.values().map(json_depth).max().unwrap_or(0) + 1,
        _ => 0,
    }
}
