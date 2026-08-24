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
}

pub fn parse_native_line(line: &str) -> Result<NativeEvent, NativeParseError> {
    let value: Value = serde_json::from_str(line)?;
    let object = value.as_object().ok_or(NativeParseError::NotObject)?;
    let event_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(NativeParseError::MissingType)?;
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
