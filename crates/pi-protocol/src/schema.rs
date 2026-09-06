use schemars::{schema::RootSchema, schema_for};

use crate::event::SessionEvent;

/// Generate the JSON Schema for the [`SessionEvent`] union.
pub fn session_event_schema() -> RootSchema {
    schema_for!(SessionEvent)
}

/// Serialize the SessionEvent schema as a JSON value.
pub fn session_event_json_schema() -> serde_json::Value {
    serde_json::to_value(session_event_schema()).expect("schema is serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HISTORY_EVENT_TYPES, STREAMING_EVENT_TYPE};

    #[test]
    fn schema_lists_frozen_event_union() {
        let schema = session_event_json_schema();
        let text = schema.to_string();
        for kind in HISTORY_EVENT_TYPES {
            assert!(text.contains(kind), "schema missing history kind {kind}");
        }
        assert!(
            text.contains(STREAMING_EVENT_TYPE),
            "schema should document assistant/chunk as a non-history member"
        );
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
    }
}
