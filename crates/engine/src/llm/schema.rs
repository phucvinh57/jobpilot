//! JSON-schema helpers: validate model output, strip code fences, adapt schemas per provider.
//!
//! Callers pass ONE schema to [`super::Llm::complete_json`] (plain JSON Schema, any draft the
//! `jsonschema` crate understands). Model output is always validated against that original
//! schema; only the copy sent to a provider is adapted:
//!
//! - **Anthropic** structured outputs require `additionalProperties: false` on every object and
//!   reject some constraints (`minimum`, `maxLength`, ...). [`for_anthropic`] adds the former and
//!   strips the latter. The stripped constraints are still enforced locally by the
//!   validate-and-retry loop.
//! - **OpenAI-compatible** endpoints get the schema as written (`strict: false`).

use anyhow::{anyhow, Result};
use serde_json::{Map, Value};

/// How many validation errors are quoted back to the model on a retry.
const MAX_REPORTED_ERRORS: usize = 10;

/// Keywords Anthropic's structured outputs do not accept; enforced locally instead.
const ANTHROPIC_UNSUPPORTED: &[&str] = &[
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "maxItems",
    "uniqueItems",
    "minContains",
    "maxContains",
    "minProperties",
    "maxProperties",
];

/// Strip a surrounding ```` ```json ... ``` ```` (or bare ```` ``` ````) fence and whitespace.
/// Text without a fence is returned trimmed.
pub fn strip_code_fences(text: &str) -> &str {
    let t = text.trim();
    let Some(rest) = t.strip_prefix("```") else { return t };
    // Drop the info string (`json`, `JSON`, ...) up to the first newline.
    let body = match rest.split_once('\n') {
        Some((info, body)) if !info.trim().contains(char::is_whitespace) => body,
        _ => rest,
    };
    let body = body.trim_end();
    body.strip_suffix("```").unwrap_or(body).trim()
}

/// Parse model text as JSON (tolerating code fences) and validate it against `validator`.
/// `Err` carries a human-readable problem list that is fed back to the model on a retry.
pub fn parse_and_validate(validator: &jsonschema::Validator, text: &str) -> std::result::Result<Value, String> {
    let body = strip_code_fences(text);
    if body.is_empty() {
        return Err("the answer was empty; reply with a JSON document".into());
    }
    let value: Value = serde_json::from_str(body).map_err(|e| format!("the answer is not valid JSON ({e})"))?;
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .take(MAX_REPORTED_ERRORS + 1)
        .map(|e| {
            let path = e.instance_path().to_string();
            let path = if path.is_empty() { "(root)".to_string() } else { path };
            format!("- {path}: {e}")
        })
        .collect();
    if errors.is_empty() {
        return Ok(value);
    }
    let mut msg = String::from("the JSON does not match the schema:\n");
    for e in errors.iter().take(MAX_REPORTED_ERRORS) {
        msg.push_str(e);
        msg.push('\n');
    }
    if errors.len() > MAX_REPORTED_ERRORS {
        msg.push_str("- ...and more\n");
    }
    Err(msg.trim_end().to_string())
}

/// Compile `schema` (fails early with a readable message when the schema itself is invalid).
pub fn compile(schema: &Value) -> Result<jsonschema::Validator> {
    jsonschema::validator_for(schema).map_err(|e| anyhow!("invalid JSON schema: {e}"))
}

/// Copy of `schema` that Anthropic structured outputs accept: `additionalProperties: false` on
/// every object schema (an explicit value is kept only if already `false`), unsupported
/// constraints removed, `minItems` limited to 0/1. Walks `properties`, `items`, `anyOf`/`oneOf`/
/// `allOf`, `$defs`/`definitions`, and friends.
pub fn for_anthropic(schema: &Value) -> Value {
    let mut out = schema.clone();
    normalize_anthropic(&mut out);
    out
}

fn normalize_anthropic(node: &mut Value) {
    match node {
        Value::Object(map) => {
            for k in ANTHROPIC_UNSUPPORTED {
                map.remove(*k);
            }
            if let Some(n) = map.get("minItems").and_then(Value::as_u64) {
                if n > 1 {
                    map.remove("minItems");
                }
            }
            if is_object_schema(map) {
                map.insert("additionalProperties".into(), Value::Bool(false));
            }
            // `properties` / `$defs` / `definitions` map names to schemas; the rest are schema
            // values or arrays of schemas. Property NAMES must not be treated as keywords.
            for key in ["properties", "patternProperties", "$defs", "definitions"] {
                if let Some(Value::Object(children)) = map.get_mut(key) {
                    for child in children.values_mut() {
                        normalize_anthropic(child);
                    }
                }
            }
            for key in ["items", "additionalItems", "not", "if", "then", "else", "contains", "propertyNames"] {
                if let Some(child) = map.get_mut(key) {
                    normalize_anthropic(child);
                }
            }
            for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
                if let Some(Value::Array(children)) = map.get_mut(key) {
                    for child in children {
                        normalize_anthropic(child);
                    }
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_anthropic),
        _ => {}
    }
}

fn is_object_schema(map: &Map<String, Value>) -> bool {
    match map.get("type") {
        Some(Value::String(t)) => t == "object",
        Some(Value::Array(ts)) => ts.iter().any(|t| t == "object"),
        _ => map.contains_key("properties"),
    }
}

/// Appendix for providers/modes without native schema support (`json_object` mode).
pub fn schema_instructions(schema: &Value) -> String {
    format!(
        "Respond with a single JSON object (no prose, no code fences) that conforms to this JSON Schema:\n{}",
        serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_fences() {
        assert_eq!(strip_code_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fences("  ```JSON\n{\"a\":1}\n```  \n"), "{\"a\":1}");
        assert_eq!(strip_code_fences("```\n[1,2]\n```"), "[1,2]");
        assert_eq!(strip_code_fences("{\"a\":1}"), "{\"a\":1}");
        // Unterminated fence (truncated reply) still yields the body.
        assert_eq!(strip_code_fences("```json\n{\"a\":1}"), "{\"a\":1}");
        // One-line fence without a newline.
        assert_eq!(strip_code_fences("```{\"a\":1}```"), "{\"a\":1}");
    }

    #[test]
    fn validates_with_readable_errors() {
        let schema = json!({
            "type": "object",
            "properties": {"ok": {"type": "boolean"}, "n": {"type": "integer", "minimum": 3}},
            "required": ["ok"]
        });
        let v = compile(&schema).unwrap();
        assert_eq!(parse_and_validate(&v, "```json\n{\"ok\": true}\n```").unwrap(), json!({"ok": true}));

        let err = parse_and_validate(&v, r#"{"ok": "yes", "n": 1}"#).unwrap_err();
        assert!(err.contains("/ok"), "{err}");
        assert!(err.contains("/n"), "{err}");

        let err = parse_and_validate(&v, "I cannot do that").unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");
        assert!(parse_and_validate(&v, "   ").unwrap_err().contains("empty"));
    }

    #[test]
    fn invalid_schema_is_reported() {
        let err = compile(&json!({"type": "nonsense"})).unwrap_err().to_string();
        assert!(err.contains("invalid JSON schema"), "{err}");
    }

    #[test]
    fn anthropic_normalization() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "maxLength": 10},
                "age": {"type": "integer", "minimum": 0},
                "tags": {"type": "array", "minItems": 3, "maxItems": 5, "items": {
                    "type": "object", "properties": {"k": {"type": "string"}}}},
                "either": {"anyOf": [{"type": "object", "properties": {}}, {"type": "null"}]},
                "properties": {"type": "string"}
            },
            "$defs": {"d": {"properties": {"x": {"type": "number", "multipleOf": 2}}}}
        });
        let out = for_anthropic(&schema);
        assert_eq!(out["additionalProperties"], json!(false));
        assert_eq!(out["properties"]["name"], json!({"type": "string"}));
        assert_eq!(out["properties"]["age"], json!({"type": "integer"}));
        assert!(out["properties"]["tags"].get("minItems").is_none());
        assert!(out["properties"]["tags"].get("maxItems").is_none());
        assert_eq!(out["properties"]["tags"]["items"]["additionalProperties"], json!(false));
        assert_eq!(out["properties"]["either"]["anyOf"][0]["additionalProperties"], json!(false));
        assert!(out["properties"]["either"]["anyOf"][1].get("additionalProperties").is_none());
        assert_eq!(out["$defs"]["d"]["additionalProperties"], json!(false));
        assert_eq!(out["$defs"]["d"]["properties"]["x"], json!({"type": "number"}));
        // A property called "properties" is a property, not a keyword.
        assert_eq!(out["properties"]["properties"], json!({"type": "string"}));
        // The input is untouched.
        assert!(schema.get("additionalProperties").is_none());
        // minItems 1 survives.
        let one = for_anthropic(&json!({"type": "array", "minItems": 1, "items": {"type": "string"}}));
        assert_eq!(one["minItems"], json!(1));
    }
}
