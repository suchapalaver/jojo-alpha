//! Shared tool schema helpers.

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

fn schema_any(_: &mut SchemaGenerator) -> Schema {
    true.into()
}

/// Wrapper for arbitrary JSON payloads when a tool output is dynamic.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[schemars(schema_with = "schema_any")]
#[ts(type = "any")]
pub struct AnyJson(pub Value);

impl AnyJson {
    pub fn new(value: Value) -> Self {
        Self(value)
    }
}

impl From<Value> for AnyJson {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

impl From<AnyJson> for Value {
    fn from(value: AnyJson) -> Self {
        value.0
    }
}
