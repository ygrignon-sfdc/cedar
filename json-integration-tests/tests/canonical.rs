// Test-internal canonical JSON helper.
//
// Mirrors the canonicalizer in the cedar-json-integration-tests generator.
// Both must produce identical output for any given JSON value.

use serde_json::Value;
use std::fmt::Write as _;

pub fn to_canonical_string(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out.push('\n');
    out
}

pub fn canonicalize_text(text: &str) -> Result<String, serde_json::Error> {
    let v: Value = serde_json::from_str(text)?;
    Ok(to_canonical_string(&v))
}

fn write_value(out: &mut String, value: &Value, indent: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            let _ = write!(out, "{n}");
        }
        Value::String(s) => write_json_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            out.push('\n');
            let inner = indent + 1;
            for (i, item) in items.iter().enumerate() {
                push_indent(out, inner);
                write_value(out, item, inner);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            out.push('\n');
            let inner = indent + 1;
            for (i, key) in keys.iter().enumerate() {
                push_indent(out, inner);
                write_json_string(out, key);
                out.push_str(": ");
                write_value(out, &map[*key], inner);
                if i + 1 < keys.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent);
            out.push('}');
        }
    }
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push(' ');
        out.push(' ');
    }
}

fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Canonicalize an entities JSON array in place:
/// - Sort the top-level array by `(uid.type, uid.id)`.
/// - Sort each entity's `parents` array by `(type, id)`.
/// - Inside `attrs` and `tags`, sort every JSON array (Cedar sets are unordered).
///
/// Mirrors the corpus generator's entity canonicalizer. See SCHEMA.md.
pub fn canonicalize_entities(value: &mut Value) {
    let Some(arr) = value.as_array_mut() else {
        return;
    };
    arr.sort_by(|a, b| uid_key(a).cmp(&uid_key(b)));
    for entity in arr {
        if let Some(obj) = entity.as_object_mut() {
            if let Some(parents) = obj.get_mut("parents").and_then(Value::as_array_mut) {
                parents.sort_by(|a, b| entity_ref_key(a).cmp(&entity_ref_key(b)));
            }
            if let Some(attrs) = obj.get_mut("attrs") {
                sort_sets_in_value(attrs);
            }
            if let Some(tags) = obj.get_mut("tags") {
                sort_sets_in_value(tags);
            }
        }
    }
}

fn uid_key(entity: &Value) -> (String, String) {
    entity.get("uid").map(entity_ref_key).unwrap_or_default()
}

fn entity_ref_key(value: &Value) -> (String, String) {
    let ty = value.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    let id = value.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    (ty, id)
}

fn sort_sets_in_value(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for v in items.iter_mut() {
                sort_sets_in_value(v);
            }
            items.sort_by(|a, b| to_canonical_string(a).cmp(&to_canonical_string(b)));
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                sort_sets_in_value(v);
            }
        }
        _ => {}
    }
}
