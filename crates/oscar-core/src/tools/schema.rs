use super::{ToolError, identifier};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error, MapAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};

/// Deliberately limited flat-object schema. No nested values, refs, regexes, or
/// remote schema resolution. Text bounds count UTF-8 bytes, not characters.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputType {
    Text { max_bytes: usize },
    Integer { min: i64, max: i64 },
    Boolean,
    Choice { values: Vec<String> },
}
#[derive(Debug, Clone, Serialize)]
pub struct Field {
    pub required: bool,
    pub value_type: InputType,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct InputSchema {
    pub fields: BTreeMap<String, Field>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
}
impl fmt::Debug for InputValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InputValue([redacted])")
    }
}
/// Only the registry can create validated input. Hosts/tools may inspect values,
/// but Debug does not expose arguments and no Deserialize constructor is public.
#[derive(Clone, Serialize)]
#[serde(transparent)]
pub struct ValidatedInput(BTreeMap<String, InputValue>);
impl ValidatedInput {
    pub fn get(&self, name: &str) -> Option<&InputValue> {
        self.0.get(name)
    }
}
impl fmt::Debug for ValidatedInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ValidatedInput([redacted])")
    }
}
pub(super) struct RawInput(BTreeMap<String, InputValue>);
impl<'de> Deserialize<'de> for RawInput {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = RawInput;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a flat argument object with unique keys")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut fields = BTreeMap::new();
                while let Some(name) = map.next_key::<String>()? {
                    if fields.len() >= 32 || !identifier(&name) || fields.contains_key(&name) {
                        return Err(M::Error::custom("invalid or duplicate argument key"));
                    }
                    fields.insert(name, map.next_value()?);
                }
                Ok(RawInput(fields))
            }
        }
        d.deserialize_map(Fields)
    }
}
impl InputSchema {
    pub(super) fn validate_definition(&self) -> Result<(), ToolError> {
        if self.fields.len() > 32 {
            return Err(ToolError::InvalidDefinition);
        }
        for (name, field) in &self.fields {
            if !identifier(name) {
                return Err(ToolError::InvalidDefinition);
            }
            let valid = match &field.value_type {
                InputType::Text { max_bytes } => (1..=1_048_576).contains(max_bytes),
                InputType::Integer { min, max } => min <= max,
                InputType::Boolean => true,
                InputType::Choice { values } => {
                    !values.is_empty()
                        && values.len() <= 32
                        && values.iter().all(|v| !v.is_empty() && v.len() <= 256)
                        && values
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == values.len()
                }
            };
            if !valid {
                return Err(ToolError::InvalidDefinition);
            }
        }
        Ok(())
    }
    pub(super) fn validate(&self, raw: RawInput) -> Result<ValidatedInput, ToolError> {
        if raw.0.keys().any(|name| !self.fields.contains_key(name)) {
            return Err(ToolError::InvalidArguments);
        }
        for (name, field) in &self.fields {
            let Some(value) = raw.0.get(name) else {
                if field.required {
                    return Err(ToolError::InvalidArguments);
                }
                continue;
            };
            let valid = match (&field.value_type, value) {
                (InputType::Text { max_bytes }, InputValue::Text(v)) => v.len() <= *max_bytes,
                (InputType::Integer { min, max }, InputValue::Integer(v)) => {
                    (*min..=*max).contains(v)
                }
                (InputType::Boolean, InputValue::Boolean(_)) => true,
                (InputType::Choice { values }, InputValue::Text(v)) => values.contains(v),
                _ => false,
            };
            if !valid {
                return Err(ToolError::InvalidArguments);
            }
        }
        Ok(ValidatedInput(raw.0))
    }
}
