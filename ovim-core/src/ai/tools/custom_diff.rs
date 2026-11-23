//! Agent contracts for reading and rearranging a Git review.

use super::{SideEffect, StrictJsonSchema, ToolDefinition};
use crate::ai::scope::RequiredScope;
use crate::ai::types::FileScope;
use serde_json::json;

pub const READ_DIFF: &str = "read_diff";
pub const SHOW_CUSTOM_DIFF: &str = "show_custom_diff";

pub fn read_diff_definition() -> ToolDefinition {
    ToolDefinition {
        name: READ_DIFF.into(),
        description: "Read Ovim's Git review using the same comparison as <leader>gd, including its configured pullbase. Finish edits first, then call without snapshot_id to freeze the current comparison and get block IDs for added and removed lines. To read the next page, pass snapshot_id and next_cursor as cursor; continue until next_cursor is null. Read all pages before assigning changes. After further edits, start a fresh snapshot. The comparison base follows Ovim's pullbase setting; this tool does not override it.".into(),
        required_scope: RequiredScope { file_scope: FileScope::Project, shell: false, network: false },
        side_effect: SideEffect::Read,
        custom_input_schema: Some(StrictJsonSchema::new(json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "snapshot_id": { "type": "string", "minLength": 1, "description": "Snapshot ID returned by an earlier read_diff call. Omit to capture the current comparison." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum changed lines returned (default 100)." },
                "cursor": { "type": "string", "minLength": 1, "maxLength": 512, "description": "Opaque next_cursor returned by the previous page. Requires snapshot_id." }
            }
        })).expect("valid read_diff schema")),
        parameters: vec![],
    }
}

pub fn show_custom_diff_definition() -> ToolDefinition {
    ToolDefinition {
        name: SHOW_CUSTOM_DIFF.into(),
        description: "Open a replayable, agent-arranged review of a frozen read_diff snapshot. Read every page first, until next_cursor is null. Each pairing is one ordered section: old owns removed lines, new owns added lines, and both show a replacement; at least one side is required. Give it a short label and optionally a message explaining the change to the reader. Keep unrelated additions and deletions in separate sections. Example: {\"snapshot_id\":\"<id>\",\"title\":\"Review\",\"pairings\":[{\"label\":\"Remove duplicate check\",\"message\":\"The shared guard now handles this case.\",\"old\":{\"block_id\":\"removed_1\"}}]}. Use zero-based offset and positive count to split blocks into disjoint slices; pair lengths may differ. Assign each changed line at most once. For a copy or extraction, related_to can point to a source range outside the section; it explains provenance but never owns lines or proves equivalence. Unassigned changes, metadata, and binary changes remain visible automatically. The GUI Guided view and terminal show labels and messages; Files keeps canonical file order. The review opens immediately, and all code comes from the frozen snapshot.".into(),
        required_scope: RequiredScope { file_scope: FileScope::Project, shell: false, network: false },
        side_effect: SideEffect::Navigation,
        custom_input_schema: Some(StrictJsonSchema::new(json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "snapshot_id": { "type": "string", "minLength": 1 },
                "title": { "type": "string", "minLength": 1, "maxLength": 200 },
                "pairings": {
                    "type": "array", "maxItems": 1000,
                    "items": {
                        "type": "object", "additionalProperties": false,
                        "properties": {
                            "label": { "type": "string", "maxLength": 200, "description": "Short, single-line section heading." },
                            "message": { "type": "string", "minLength": 1, "maxLength": 2000, "description": "Optional plain text note for the reader, shown above this section in Guided and terminal views. May include newlines and tabs; blank text and unsafe controls are rejected." },
                            "old": { "$ref": "#/$defs/reference" },
                            "new": { "$ref": "#/$defs/reference" },
                            "related_to": { "$ref": "#/$defs/reference", "description": "Explanatory reference to a removed or added range outside this section. Does not own its lines or imply equality." }
                        },
                        "anyOf": [{"required": ["old"]}, {"required": ["new"]}]
                    }
                }
            },
            "required": ["snapshot_id", "title", "pairings"],
            "$defs": {
                "reference": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "block_id": { "type": "string", "minLength": 1 },
                        "offset": { "type": "integer", "minimum": 0 },
                        "count": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["block_id"]
                }
            }
        })).expect("valid show_custom_diff schema")),
        parameters: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn section_schema_and_runtime_shape_agree() {
        let definition = show_custom_diff_definition();
        let schema = definition.custom_input_schema.unwrap();
        let reference = json!({"block_id": "removed_1", "offset": 0, "count": 1});
        for section in [
            json!({"old": reference}),
            json!({"new": reference}),
            json!({"old": reference, "new": reference}),
            json!({"new": reference, "related_to": reference}),
            json!({"old": reference, "message": "Review caller\nthen remove shim"}),
        ] {
            schema
                .validate_instance(
                    &json!({"snapshot_id":"s", "title":"Review", "pairings":[section]}),
                )
                .unwrap();
            let parsed: crate::native_diff::DiffPairing =
                serde_json::from_value(section.clone()).unwrap();
            assert_eq!(
                parsed.related_to.is_some(),
                section.get("related_to").is_some()
            );
        }
        for section in [
            json!({}),
            json!({"label":"Deleted"}),
            json!({"related_to":reference}),
            json!({"old":null}),
            json!({"old":reference,"kind":"context"}),
            json!({"old":reference,"text":"invented"}),
            json!({"old":reference,"related_to":{"block_id":"x","count":0}}),
            json!({"old":reference,"message":"x".repeat(2001)}),
        ] {
            assert!(schema
                .validate_instance(
                    &json!({"snapshot_id":"s", "title":"Review", "pairings":[section]})
                )
                .is_err());
        }
    }
}
