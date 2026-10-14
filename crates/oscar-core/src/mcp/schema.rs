use crate::error::{McpError, OscarError};
use crate::tools::{Field, InputSchema, InputType};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn bad() -> OscarError {
    OscarError::Mcp(McpError::UnsupportedSchema)
}
/// Translate only constraints that OSCAR can enforce. Never discard validation
/// keywords or treat a server's annotations as a permission declaration.
pub(super) fn convert(value: &Value, max_bytes: usize) -> Result<InputSchema, OscarError> {
    let root = value.as_object().ok_or_else(bad)?;
    if root.get("type").and_then(Value::as_str) != Some("object")
        || root.keys().any(|k| {
            ![
                "type",
                "properties",
                "required",
                "additionalProperties",
                "title",
                "description",
                "$schema",
            ]
            .contains(&k.as_str())
        })
        || root
            .get("additionalProperties")
            .is_some_and(|v| !v.is_boolean())
    {
        return Err(bad());
    }
    let empty = serde_json::Map::new();
    let properties = match root.get("properties") {
        Some(p) => p.as_object().ok_or_else(bad)?,
        None => &empty,
    };
    if properties.len() > 32 {
        return Err(bad());
    }
    let mut required = BTreeSet::new();
    if let Some(values) = root.get("required") {
        for name in values.as_array().ok_or_else(bad)? {
            let name = name.as_str().ok_or_else(bad)?;
            if !required.insert(name) || !properties.contains_key(name) {
                return Err(bad());
            }
        }
    }
    let mut fields = BTreeMap::new();
    for (name, value) in properties {
        let field = value.as_object().ok_or_else(bad)?;
        let kind = field.get("type").and_then(Value::as_str).ok_or_else(bad)?;
        let validation: Vec<_> = field
            .keys()
            .filter(|k| !["type", "description", "title", "default"].contains(&k.as_str()))
            .map(String::as_str)
            .collect();
        let value_type = match kind {
            "string" if validation.iter().all(|k| *k == "enum") && field.contains_key("enum") => {
                let values = field["enum"]
                    .as_array()
                    .ok_or_else(bad)?
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned).ok_or_else(bad))
                    .collect::<Result<Vec<_>, _>>()?;
                InputType::Choice { values }
            }
            "string" if validation.iter().all(|k| *k == "maxLength") => {
                let limit = match field.get("maxLength") {
                    Some(n) => usize::try_from(n.as_u64().ok_or_else(bad)?)
                        .map_err(|_| bad())?
                        .min(max_bytes),
                    None => max_bytes,
                };
                if limit == 0 {
                    return Err(bad());
                }
                InputType::Text { max_bytes: limit }
            }
            "integer"
                if validation
                    .iter()
                    .all(|k| ["minimum", "maximum"].contains(k)) =>
            {
                let min = match field.get("minimum") {
                    Some(n) => n.as_i64().ok_or_else(bad)?,
                    None => i64::MIN,
                };
                let max = match field.get("maximum") {
                    Some(n) => n.as_i64().ok_or_else(bad)?,
                    None => i64::MAX,
                };
                if min > max {
                    return Err(bad());
                }
                InputType::Integer { min, max }
            }
            "boolean" if validation.is_empty() => InputType::Boolean,
            _ => return Err(bad()),
        };
        fields.insert(
            name.clone(),
            Field {
                required: required.contains(name.as_str()),
                value_type,
            },
        );
    }
    Ok(InputSchema { fields })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn scalar_constraints_translate_and_unknown_constraints_are_rejected() {
        let schema = convert(&json!({"type":"object","properties":{"s":{"type":"string","maxLength":20},"i":{"type":"integer","minimum":-1,"maximum":2},"b":{"type":"boolean"},"e":{"type":"string","enum":["a","b"]}},"required":["s"]}),100).unwrap();
        assert_eq!(schema.fields.len(), 4);
        assert!(schema.fields["s"].required);
        assert!(matches!(
            schema.fields["s"].value_type,
            InputType::Text { max_bytes: 20 }
        ));
        for unsupported in [
            json!({"type":"array"}),
            json!({"type":"number"}),
            json!({"type":"object"}),
            json!({"type":"string","pattern":".*"}),
            json!({"type":"string","minLength":1}),
            json!({"$ref":"https://invalid.example/schema"}),
            json!({"type":["string","null"]}),
            json!({"type":"integer","multipleOf":2}),
        ] {
            assert!(
                convert(
                    &json!({"type":"object","properties":{"x":unsupported}}),
                    100
                )
                .is_err()
            );
        }
        assert!(
            convert(
                &json!({"type":"object","properties":{},"required":["missing"]}),
                100
            )
            .is_err()
        );
        assert!(convert(&json!({"type":"object","properties":{},"oneOf":[]}), 100).is_err());
    }
}
