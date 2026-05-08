//! Schema JSON normalization. Mirrors the corpus repo's
//! tools/generator/src/normalize.rs. See that repo's SCHEMA.md for the
//! contract every implementation MUST apply before comparison.

use serde_json::{json, Value};

const EXTENSION_NAMES: &[&str] = &["decimal", "datetime", "duration", "ipaddr"];

/// Normalize a policy / residual JSON value in place. Applies (post-order)
///   1. `like`-pattern literal coalescing
///   2. Boolean-constant simplification
/// See SCHEMA.md.
pub fn normalize_policy(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                normalize_policy(v);
            }
            if let Some(like) = map.get_mut("like") {
                if let Some(pattern) = like
                    .as_object_mut()
                    .and_then(|m| m.get_mut("pattern"))
                    .and_then(Value::as_array_mut)
                {
                    coalesce_pattern(pattern);
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                normalize_policy(v);
            }
        }
        _ => {}
    }
    if let Some(replacement) = simplify_boolean(value) {
        *value = replacement;
    }
}

fn coalesce_pattern(items: &mut Vec<Value>) {
    let mut out: Vec<Value> = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        if let Some(s) = literal_str(&item) {
            if let Some(prev) = out.last_mut() {
                if let Some(prev_s) = literal_str(prev) {
                    let merged = format!("{prev_s}{s}");
                    *prev = json!({ "Literal": merged });
                    continue;
                }
            }
            out.push(json!({ "Literal": s }));
        } else {
            out.push(item);
        }
    }
    *items = out;
}

fn literal_str(value: &Value) -> Option<String> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get("Literal")?.as_str().map(|s| s.to_string())
}

/// See SCHEMA.md ("boolean-constant simplification") for the rule list.
fn simplify_boolean(value: &Value) -> Option<Value> {
    let map = value.as_object()?;
    if map.len() != 1 {
        return None;
    }
    let (op, payload) = map.iter().next()?;
    let op = op.as_str();
    if op != "&&" && op != "||" {
        return None;
    }
    let payload = payload.as_object()?;
    let left = payload.get("left")?;
    let right = payload.get("right")?;

    let left_const = bool_constant(left);
    let right_const = bool_constant(right);

    match op {
        "&&" => {
            if right_const == Some(true) {
                return Some(left.clone());
            }
            if left_const == Some(true) {
                return Some(right.clone());
            }
            if left_const == Some(false) {
                return Some(json!({ "Value": false }));
            }
        }
        "||" => {
            if right_const == Some(false) {
                return Some(left.clone());
            }
            if left_const == Some(false) {
                return Some(right.clone());
            }
            if left_const == Some(true) {
                return Some(json!({ "Value": true }));
            }
        }
        _ => {}
    }
    None
}

fn bool_constant(value: &Value) -> Option<bool> {
    let map = value.as_object()?;
    if map.len() != 1 {
        return None;
    }
    map.get("Value")?.as_bool()
}

pub fn normalize_schema(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(rewrite) = entity_or_common_rewrite(map) {
                *value = rewrite;
                return;
            }
            for v in map.values_mut() {
                normalize_schema(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                normalize_schema(v);
            }
        }
        _ => {}
    }
}

fn entity_or_common_rewrite(map: &serde_json::Map<String, Value>) -> Option<Value> {
    if map.get("type").and_then(Value::as_str)? != "EntityOrCommon" {
        return None;
    }
    let name = map.get("name").and_then(Value::as_str)?;
    let mut rewritten: serde_json::Map<String, Value> = match name {
        "Bool" => serde_json::Map::from_iter([("type".to_string(), json!("Boolean"))]),
        "Long" => serde_json::Map::from_iter([("type".to_string(), json!("Long"))]),
        "String" => serde_json::Map::from_iter([("type".to_string(), json!("String"))]),
        n if EXTENSION_NAMES.contains(&n) => serde_json::Map::from_iter([
            ("type".to_string(), json!("Extension")),
            ("name".to_string(), json!(n)),
        ]),
        _ => return None,
    };
    for (k, v) in map {
        if k == "type" || k == "name" {
            continue;
        }
        rewritten.insert(k.clone(), v.clone());
    }
    Some(Value::Object(rewritten))
}
