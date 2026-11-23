//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! The JSON Schema subset that `NectoJSONSchema` checks on the Mac, with the same messages.

use serde_json::Value;

/// Validates `value` against `schema`, returning the first failure as `path: reason`.
pub fn validate(value: &Value, schema: &Value) -> Option<String> {
    check(value, schema, "").map(|(path, reason)| if path.is_empty() { reason } else { format!("{path}: {reason}") })
}

fn check(value: &Value, schema: &Value, path: &str) -> Option<(String, String)> {
    let schema = schema.as_object()?;
    let fail = |path: String, reason: String| Some((path, reason));

    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let matches = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_f64().is_some_and(|n| n.fract() == 0.0),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        };
        if !matches {
            return fail(path.into(), format!("must be of type {kind}"));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return fail(path.into(), "is not one of the allowed values".into());
        }
    }
    let number = |key: &str| schema.get(key).and_then(Value::as_f64);

    match value {
        Value::Number(n) => {
            let n = n.as_f64().unwrap_or_default();
            if let Some(min) = number("minimum").filter(|m| n < *m) {
                return fail(path.into(), format!("must be greater than or equal to {min}"));
            }
            if let Some(max) = number("maximum").filter(|m| n > *m) {
                return fail(path.into(), format!("must be less than or equal to {max}"));
            }
        }
        Value::String(text) => {
            // ponytail: counts scalars; Swift counts grapheme clusters, which only differs for combined emoji.
            let count = text.chars().count() as f64;
            if let Some(min) = number("minLength").filter(|m| count < *m) {
                return fail(path.into(), format!("must be at least {} characters", min as i64));
            }
            if let Some(max) = number("maxLength").filter(|m| count > *m) {
                return fail(path.into(), format!("must be at most {} characters", max as i64));
            }
        }
        Value::Array(items) => {
            let count = items.len() as f64;
            if let Some(min) = number("minItems").filter(|m| count < *m) {
                return fail(path.into(), format!("must contain at least {} items", min as i64));
            }
            if let Some(max) = number("maxItems").filter(|m| count > *m) {
                return fail(path.into(), format!("must contain at most {} items", max as i64));
            }
            if let Some(item_schema) = schema.get("items") {
                for (index, item) in items.iter().enumerate() {
                    if let Some(failure) = check(item, item_schema, &format!("{path}[{index}]")) {
                        return Some(failure);
                    }
                }
            }
        }
        Value::Object(members) => {
            for key in schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                if !members.contains_key(key) {
                    return fail(join(path, key), "is required".into());
                }
            }
            let empty = serde_json::Map::new();
            let properties = schema.get("properties").and_then(Value::as_object).unwrap_or(&empty);
            if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                if let Some(key) = members.keys().find(|k| !properties.contains_key(*k)) {
                    return fail(join(path, key), "is not declared in the schema".into());
                }
            }
            for (key, property) in properties {
                if let Some(member) = members.get(key) {
                    if let Some(failure) = check(member, property, &join(path, key)) {
                        return Some(failure);
                    }
                }
            }
        }
        Value::Null | Value::Bool(_) => {}
    }
    None
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() { key.into() } else { format!("{path}.{key}") }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use serde_json::json;

    #[test]
    fn reports_first_failure_with_path() {
        let schema = json!({
            "type": "object",
            "properties": { "limit": { "type": "number", "minimum": 1 }, "tags": { "type": "array", "items": { "type": "string" } } },
            "required": ["limit"],
            "additionalProperties": false
        });
        assert_eq!(validate(&json!({ "limit": 5 }), &schema), None);
        assert_eq!(validate(&json!({}), &schema).unwrap(), "limit: is required");
        assert_eq!(validate(&json!({ "limit": 0 }), &schema).unwrap(), "limit: must be greater than or equal to 1");
        assert_eq!(validate(&json!({ "limit": 1, "x": 1 }), &schema).unwrap(), "x: is not declared in the schema");
        assert_eq!(validate(&json!({ "limit": 1, "tags": ["a", 2] }), &schema).unwrap(), "tags[1]: must be of type string");
        assert_eq!(validate(&json!([]), &schema).unwrap(), "must be of type object");
        assert_eq!(validate(&json!(1.5), &json!({ "type": "integer" })).unwrap(), "must be of type integer");
    }
}
