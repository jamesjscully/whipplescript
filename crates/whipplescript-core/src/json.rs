//! Generic JSON/string parsing utilities shared across the platform.
//!
//! These are provider- and package-agnostic helpers over `serde_json::Value`.
//! They live in the leaf `whipplescript-core` crate so both the CLI surfaces and
//! the wasm-kernel-hostable package registry validators (which cannot call back
//! into the CLI binary) can reach them.

use serde_json::Value;

/// Read a required non-empty string field, or an actionable error naming
/// `owner` and `field`.
pub fn required_json_string(value: &Value, field: &str, owner: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{owner} must have non-empty `{field}` string"))
}

/// Read an optional non-empty string field.
pub fn optional_json_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

/// Read the first present non-empty string among `fields`.
pub fn optional_json_string_any(value: &Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .find_map(|field| optional_json_string(value, field))
}

/// Read an optional string array, dropping empty entries.
pub fn optional_json_string_array(value: &Value, field: &str) -> Option<Vec<String>> {
    value.get(field).and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter(|item| !item.trim().is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    })
}

/// Read a required array field by reference, or an actionable error.
pub fn require_json_array_field<'a>(
    value: &'a Value,
    field: &str,
    owner: &str,
) -> Result<&'a Vec<Value>, String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{owner}.{field} must be an array"))
}

/// Join `values` as a comma-separated list of backtick-quoted tokens, for
/// "expected one of ..." diagnostics.
pub fn quoted_platform_values<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    values
        .into_iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
