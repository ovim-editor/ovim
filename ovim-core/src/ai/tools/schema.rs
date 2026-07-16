use serde_json::json;

use super::{ParamType, ToolDefinition};

/// Generate tool schemas in OpenAI function calling format.
pub fn tools_to_openai_schema(tools: &[&ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": params_to_json_schema(&tool.parameters),
                }
            })
        })
        .collect()
}

/// Generate tool schemas in Anthropic tool use format.
pub fn tools_to_anthropic_schema(tools: &[&ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": params_to_json_schema(&tool.parameters),
            })
        })
        .collect()
}

fn params_to_json_schema(params: &[super::ToolParam]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for param in params {
        let schema = param_type_to_schema(&param.param_type, &param.description);
        properties.insert(param.name.clone(), schema);
        if param.required {
            required.push(json!(param.name));
        }
    }

    let mut schema = json!({
        "type": "object",
        "properties": properties,
    });
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

fn param_type_to_schema(param_type: &ParamType, description: &str) -> serde_json::Value {
    match param_type {
        ParamType::String => json!({
            "type": "string",
            "description": description,
        }),
        ParamType::StringArray => json!({
            "type": "array",
            "items": { "type": "string" },
            "description": description,
        }),
        ParamType::Integer => json!({
            "type": "integer",
            "description": description,
        }),
        ParamType::Boolean => json!({
            "type": "boolean",
            "description": description,
        }),
        ParamType::FilePath => json!({
            "type": "string",
            "description": description,
        }),
        ParamType::LineNumber => json!({
            "type": "integer",
            "description": description,
        }),
        ParamType::LineRange => json!({
            "type": "object",
            "description": description,
            "properties": {
                "start": { "type": "integer" },
                "end": { "type": "integer" },
            },
        }),
        ParamType::CodeExplanationSteps => json!({
            "type": "array",
            "description": description,
            "minItems": 1,
            "items": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Project-relative file path."
                    },
                    "start_line": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Required 1-indexed inclusive anchor line."
                    },
                    "end_line": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional 1-indexed inclusive end line for a cohesive block."
                    },
                    "comment": {
                        "type": "string",
                        "description": "Teach one easy-to-understand idea: explain why this location matters, how it connects to the walkthrough, and what depends on it; do not merely paraphrase the code."
                    }
                },
                "required": ["path", "start_line", "comment"]
            }
        }),
        ParamType::ChangeSet => change_set_schema(description),
        ParamType::TalkThroughChangesSteps => talk_through_changes_steps_schema(description),
    }
}

fn change_set_schema(description: &str) -> serde_json::Value {
    let operation = |properties: serde_json::Value, required: serde_json::Value| {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": properties,
            "required": required,
        })
    };
    let common_id = json!({
        "type": "string",
        "minLength": 1,
        "description": "Stable ID referenced by one or more change walkthrough steps."
    });
    let path = json!({ "type": "string", "minLength": 1 });
    let revision = json!({
        "type": "integer",
        "minimum": 0,
        "description": "Authoritative buffer revision on which this operation is based."
    });

    json!({
        "type": "object",
        "description": description,
        "additionalProperties": false,
        "properties": {
            "operations": {
                "type": "array",
                "minItems": 1,
                "maxItems": crate::ai::change_set::MAX_CHANGE_SET_OPERATIONS,
                "items": {
                    "oneOf": [
                        operation(
                            json!({
                                "id": common_id,
                                "type": { "const": "modify" },
                                "path": path,
                                "expected_revision": revision,
                                "patch": { "type": "string", "minLength": 1 }
                            }),
                            json!(["id", "type", "path", "expected_revision", "patch"]),
                        ),
                        operation(
                            json!({
                                "id": common_id,
                                "type": { "const": "create" },
                                "path": path,
                                "expected_revision": revision,
                                "content": { "type": "string" }
                            }),
                            json!(["id", "type", "path", "expected_revision", "content"]),
                        ),
                        operation(
                            json!({
                                "id": common_id,
                                "type": { "const": "delete" },
                                "path": path,
                                "expected_revision": revision
                            }),
                            json!(["id", "type", "path", "expected_revision"]),
                        ),
                        operation(
                            json!({
                                "id": common_id,
                                "type": { "const": "rename" },
                                "from_path": path,
                                "to_path": path,
                                "expected_revision": revision
                            }),
                            json!(["id", "type", "from_path", "to_path", "expected_revision"]),
                        )
                    ]
                }
            }
        },
        "required": ["operations"]
    })
}

fn talk_through_changes_steps_schema(description: &str) -> serde_json::Value {
    json!({
        "type": "array",
        "description": description,
        "minItems": 1,
        "maxItems": crate::ai::change_set::MAX_TALK_THROUGH_STEPS,
        "items": {
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "type": { "const": "code" },
                        "path": { "type": "string", "minLength": 1 },
                        "revision": { "type": "integer", "minimum": 0 },
                        "start_line": { "type": "integer", "minimum": 1 },
                        "end_line": { "type": "integer", "minimum": 1 },
                        "comment": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Teach why this base-code reference constrains or motivates a proposed change."
                        }
                    },
                    "required": ["type", "path", "revision", "start_line", "comment"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "type": { "const": "change" },
                        "operation_id": {
                            "type": "string",
                            "minLength": 1,
                            "description": "ID of an operation in change_set.operations."
                        },
                        "comment": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Teach how and why this proposed edit responds to the established constraint."
                        }
                    },
                    "required": ["type", "operation_id", "comment"]
                }
            ]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::scope::RequiredScope;
    use crate::ai::tools::{SideEffect, ToolParam};
    use crate::ai::types::FileScope;

    fn test_tool() -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read file content.".to_string(),
            required_scope: RequiredScope {
                file_scope: FileScope::File,
                shell: false,
                network: false,
            },
            side_effect: SideEffect::Read,
            parameters: vec![
                ToolParam {
                    name: "start_line".to_string(),
                    param_type: ParamType::LineNumber,
                    required: false,
                    description: "Start line (1-indexed).".to_string(),
                },
                ToolParam {
                    name: "end_line".to_string(),
                    param_type: ParamType::LineNumber,
                    required: false,
                    description: "End line (1-indexed).".to_string(),
                },
            ],
        }
    }

    fn test_tool_with_required() -> ToolDefinition {
        ToolDefinition {
            name: "search".to_string(),
            description: "Search files.".to_string(),
            required_scope: RequiredScope {
                file_scope: FileScope::Project,
                shell: false,
                network: false,
            },
            side_effect: SideEffect::Read,
            parameters: vec![ToolParam {
                name: "query".to_string(),
                param_type: ParamType::String,
                required: true,
                description: "Search query.".to_string(),
            }],
        }
    }

    #[test]
    fn openai_schema_shape() {
        let tool = test_tool();
        let schemas = tools_to_openai_schema(&[&tool]);
        assert_eq!(schemas.len(), 1);

        let s = &schemas[0];
        assert_eq!(s["type"], "function");
        assert_eq!(s["function"]["name"], "read_file");
        assert_eq!(s["function"]["parameters"]["type"], "object");
        assert!(s["function"]["parameters"]["properties"]["start_line"].is_object());
        assert!(s["function"]["parameters"]["properties"]["end_line"].is_object());
        // No required params, so "required" key should be absent
        assert!(s["function"]["parameters"].get("required").is_none());
    }

    #[test]
    fn anthropic_schema_shape() {
        let tool = test_tool();
        let schemas = tools_to_anthropic_schema(&[&tool]);
        assert_eq!(schemas.len(), 1);

        let s = &schemas[0];
        assert_eq!(s["name"], "read_file");
        assert_eq!(s["input_schema"]["type"], "object");
        assert!(s["input_schema"]["properties"]["start_line"].is_object());
    }

    #[test]
    fn required_params_included() {
        let tool = test_tool_with_required();
        let schemas = tools_to_openai_schema(&[&tool]);
        let s = &schemas[0];
        let required = s["function"]["parameters"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "query");
    }

    #[test]
    fn param_types_map_correctly() {
        let schema = param_type_to_schema(&ParamType::String, "desc");
        assert_eq!(schema["type"], "string");

        let schema = param_type_to_schema(&ParamType::Integer, "desc");
        assert_eq!(schema["type"], "integer");

        let schema = param_type_to_schema(&ParamType::StringArray, "desc");
        assert_eq!(schema["type"], "array");
        assert_eq!(schema["items"]["type"], "string");

        let schema = param_type_to_schema(&ParamType::Boolean, "desc");
        assert_eq!(schema["type"], "boolean");

        let schema = param_type_to_schema(&ParamType::LineRange, "desc");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["start"].is_object());
        assert!(schema["properties"]["end"].is_object());
    }

    #[test]
    fn change_set_schema_discriminates_all_operation_types() {
        let schema = param_type_to_schema(&ParamType::ChangeSet, "staged operations");
        let operations = &schema["properties"]["operations"];
        assert_eq!(operations["maxItems"], 12);
        let variants = operations["items"]["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 4);
        assert_eq!(variants[0]["properties"]["type"]["const"], "modify");
        assert_eq!(variants[1]["properties"]["type"]["const"], "create");
        assert_eq!(variants[2]["properties"]["type"]["const"], "delete");
        assert_eq!(variants[3]["properties"]["type"]["const"], "rename");
        assert_eq!(variants[0]["properties"]["expected_revision"]["minimum"], 0);
    }

    #[test]
    fn talk_through_schema_separates_code_and_change_steps() {
        let schema = param_type_to_schema(&ParamType::TalkThroughChangesSteps, "pedagogical steps");
        assert_eq!(schema["maxItems"], 20);
        let variants = schema["items"]["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["properties"]["type"]["const"], "code");
        assert_eq!(variants[1]["properties"]["type"]["const"], "change");
        assert!(variants[0]["properties"]["start_line"].is_object());
        assert!(variants[1]["properties"]["operation_id"].is_object());
    }
}
