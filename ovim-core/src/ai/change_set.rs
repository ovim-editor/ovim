use std::collections::{HashMap, HashSet};

use serde::Deserialize;

pub const MAX_CHANGE_SET_OPERATIONS: usize = 12;
pub const MAX_TALK_THROUGH_STEPS: usize = 20;
pub const MAX_CHANGE_SET_FILES: usize = 8;
pub const MAX_CHANGE_SET_PAYLOAD_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ChangeSetProposal {
    pub operations: Vec<ChangeSetOperation>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChangeSetOperation {
    Modify {
        id: String,
        path: String,
        expected_revision: usize,
        patch: String,
    },
    Create {
        id: String,
        path: String,
        expected_revision: usize,
        content: String,
    },
    Delete {
        id: String,
        path: String,
        expected_revision: usize,
    },
    Rename {
        id: String,
        from_path: String,
        to_path: String,
        expected_revision: usize,
    },
}

impl ChangeSetOperation {
    pub fn id(&self) -> &str {
        match self {
            Self::Modify { id, .. }
            | Self::Create { id, .. }
            | Self::Delete { id, .. }
            | Self::Rename { id, .. } => id,
        }
    }

    fn source_path(&self) -> &str {
        match self {
            Self::Modify { path, .. } | Self::Create { path, .. } | Self::Delete { path, .. } => {
                path
            }
            Self::Rename { from_path, .. } => from_path,
        }
    }

    fn paths(&self) -> impl Iterator<Item = &str> {
        let (first, second) = match self {
            Self::Modify { path, .. } | Self::Create { path, .. } | Self::Delete { path, .. } => {
                (path.as_str(), None)
            }
            Self::Rename {
                from_path, to_path, ..
            } => (from_path.as_str(), Some(to_path.as_str())),
        };
        std::iter::once(first).chain(second)
    }

    fn payload_bytes(&self) -> usize {
        match self {
            Self::Modify { patch, .. } => patch.len(),
            Self::Create { content, .. } => content.len(),
            Self::Delete { .. } | Self::Rename { .. } => 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TalkThroughChangesProposal {
    pub title: String,
    pub change_set: ChangeSetProposal,
    pub steps: Vec<TalkThroughStep>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TalkThroughStep {
    Code {
        path: String,
        revision: usize,
        start_line: usize,
        #[serde(default)]
        end_line: Option<usize>,
        comment: String,
    },
    Change {
        operation_id: String,
        comment: String,
    },
}

impl TalkThroughChangesProposal {
    pub fn validate_structure(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("title must not be empty".to_string());
        }
        if self.change_set.operations.is_empty() {
            return Err("change_set.operations must contain at least one operation".to_string());
        }
        if self.change_set.operations.len() > MAX_CHANGE_SET_OPERATIONS {
            return Err(format!(
                "change_set has {} operations; at most {MAX_CHANGE_SET_OPERATIONS} are allowed",
                self.change_set.operations.len()
            ));
        }
        if self.steps.is_empty() {
            return Err("steps must contain at least one walkthrough step".to_string());
        }
        if self.steps.len() > MAX_TALK_THROUGH_STEPS {
            return Err(format!(
                "walkthrough has {} steps; at most {MAX_TALK_THROUGH_STEPS} are allowed",
                self.steps.len()
            ));
        }

        let mut operations_by_id = HashMap::new();
        let mut source_paths = HashSet::new();
        let mut affected_paths = HashMap::new();
        let mut payload_bytes = 0usize;

        for operation in &self.change_set.operations {
            let id = operation.id().trim();
            if id.is_empty() {
                return Err("operation IDs must not be empty".to_string());
            }
            if id != operation.id() {
                return Err(format!(
                    "operation ID `{}` must not have leading or trailing whitespace",
                    operation.id()
                ));
            }
            if operations_by_id.insert(id, operation).is_some() {
                return Err(format!("duplicate operation ID `{id}`"));
            }
            if operation.paths().any(|path| path.trim().is_empty()) {
                return Err(format!("operation `{id}` contains an empty path"));
            }
            if !source_paths.insert(operation.source_path()) {
                return Err(format!(
                    "multiple content-changing operations target base path `{}`; combine them into one operation",
                    operation.source_path()
                ));
            }
            for path in operation.paths() {
                if let Some(previous_id) = affected_paths.insert(path, id) {
                    if previous_id == id {
                        return Err(format!(
                            "operation `{id}` uses `{path}` as both source and destination"
                        ));
                    }
                    return Err(format!(
                        "operations `{previous_id}` and `{id}` both affect path `{path}`"
                    ));
                }
            }
            payload_bytes = payload_bytes.saturating_add(operation.payload_bytes());
        }

        if affected_paths.len() > MAX_CHANGE_SET_FILES {
            return Err(format!(
                "change_set affects {} paths; at most {MAX_CHANGE_SET_FILES} are allowed",
                affected_paths.len()
            ));
        }
        if payload_bytes > MAX_CHANGE_SET_PAYLOAD_BYTES {
            return Err(format!(
                "change_set payload is {payload_bytes} bytes; at most {MAX_CHANGE_SET_PAYLOAD_BYTES} bytes are allowed"
            ));
        }

        let mut referenced_operations = HashSet::new();
        for (index, step) in self.steps.iter().enumerate() {
            match step {
                TalkThroughStep::Code {
                    path,
                    start_line,
                    end_line,
                    comment,
                    ..
                } => {
                    if path.trim().is_empty() {
                        return Err(format!("code step {} has an empty path", index + 1));
                    }
                    if *start_line == 0 {
                        return Err(format!(
                            "code step {} start_line must be at least 1",
                            index + 1
                        ));
                    }
                    if end_line.is_some_and(|end| end < *start_line) {
                        return Err(format!(
                            "code step {} end_line must be greater than or equal to start_line",
                            index + 1
                        ));
                    }
                    if comment.trim().is_empty() {
                        return Err(format!("code step {} has an empty comment", index + 1));
                    }
                }
                TalkThroughStep::Change {
                    operation_id,
                    comment,
                } => {
                    if !operations_by_id.contains_key(operation_id.as_str()) {
                        return Err(format!(
                            "change step {} references unknown operation `{operation_id}`",
                            index + 1
                        ));
                    }
                    if comment.trim().is_empty() {
                        return Err(format!("change step {} has an empty comment", index + 1));
                    }
                    referenced_operations.insert(operation_id.as_str());
                }
            }
        }

        if referenced_operations.len() != operations_by_id.len() {
            let mut missing = operations_by_id
                .keys()
                .filter(|id| !referenced_operations.contains(**id))
                .copied()
                .collect::<Vec<_>>();
            missing.sort_unstable();
            return Err(format!(
                "every operation must have a change step; missing {}",
                missing
                    .into_iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal() -> TalkThroughChangesProposal {
        serde_json::from_value(serde_json::json!({
            "title": "Extract validation",
            "change_set": {
                "operations": [
                    {
                        "id": "modify-handler",
                        "type": "modify",
                        "path": "src/handler.rs",
                        "expected_revision": 4,
                        "patch": "*** Begin Patch\n*** End Patch"
                    },
                    {
                        "id": "create-validator",
                        "type": "create",
                        "path": "src/validator.rs",
                        "expected_revision": 0,
                        "content": "pub fn validate() {}\n"
                    }
                ]
            },
            "steps": [
                {
                    "type": "code",
                    "path": "src/handler.rs",
                    "revision": 4,
                    "start_line": 10,
                    "end_line": 14,
                    "comment": "The handler currently owns validation."
                },
                {
                    "type": "change",
                    "operation_id": "modify-handler",
                    "comment": "Delegate the rule to a focused module."
                },
                {
                    "type": "change",
                    "operation_id": "create-validator",
                    "comment": "The new module gives the rule one owner."
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn parses_and_validates_a_talk_through_proposal() {
        let proposal = proposal();
        assert_eq!(proposal.change_set.operations.len(), 2);
        assert!(proposal.validate_structure().is_ok());
    }

    #[test]
    fn rejects_duplicate_operation_ids() {
        let mut proposal = proposal();
        let duplicate = proposal.change_set.operations[0].clone();
        proposal.change_set.operations.push(duplicate);
        assert_eq!(
            proposal.validate_structure().unwrap_err(),
            "duplicate operation ID `modify-handler`"
        );
    }

    #[test]
    fn rejects_multiple_operations_for_one_base_path() {
        let mut proposal = proposal();
        proposal
            .change_set
            .operations
            .push(ChangeSetOperation::Delete {
                id: "delete-handler".to_string(),
                path: "src/handler.rs".to_string(),
                expected_revision: 4,
            });
        assert!(proposal
            .validate_structure()
            .unwrap_err()
            .contains("multiple content-changing operations target base path"));
    }

    #[test]
    fn rejects_destination_collisions_between_operations() {
        let mut proposal = proposal();
        proposal
            .change_set
            .operations
            .push(ChangeSetOperation::Rename {
                id: "rename-handler".to_string(),
                from_path: "src/old_handler.rs".to_string(),
                to_path: "src/validator.rs".to_string(),
                expected_revision: 2,
            });

        assert_eq!(
            proposal.validate_structure().unwrap_err(),
            "operations `create-validator` and `rename-handler` both affect path `src/validator.rs`"
        );
    }

    #[test]
    fn rejects_rename_to_the_same_path() {
        let mut proposal = proposal();
        proposal.change_set.operations[0] = ChangeSetOperation::Rename {
            id: "rename-handler".to_string(),
            from_path: "src/handler.rs".to_string(),
            to_path: "src/handler.rs".to_string(),
            expected_revision: 4,
        };

        assert_eq!(
            proposal.validate_structure().unwrap_err(),
            "operation `rename-handler` uses `src/handler.rs` as both source and destination"
        );
    }

    #[test]
    fn rejects_operation_ids_with_outer_whitespace() {
        let mut proposal = proposal();
        let ChangeSetOperation::Modify { id, .. } = &mut proposal.change_set.operations[0] else {
            panic!("expected modify operation");
        };
        *id = " modify-handler ".to_string();

        assert_eq!(
            proposal.validate_structure().unwrap_err(),
            "operation ID ` modify-handler ` must not have leading or trailing whitespace"
        );
    }

    #[test]
    fn rejects_unknown_change_step_operation() {
        let mut proposal = proposal();
        let TalkThroughStep::Change { operation_id, .. } = &mut proposal.steps[1] else {
            panic!("expected change step");
        };
        *operation_id = "missing".to_string();
        assert!(proposal
            .validate_structure()
            .unwrap_err()
            .contains("references unknown operation `missing`"));
    }

    #[test]
    fn requires_every_operation_to_appear_in_the_walkthrough() {
        let mut proposal = proposal();
        proposal.steps.pop();
        assert_eq!(
            proposal.validate_structure().unwrap_err(),
            "every operation must have a change step; missing `create-validator`"
        );
    }

    #[test]
    fn rejects_backwards_code_ranges() {
        let mut proposal = proposal();
        let TalkThroughStep::Code { end_line, .. } = &mut proposal.steps[0] else {
            panic!("expected code step");
        };
        *end_line = Some(9);
        assert!(proposal
            .validate_structure()
            .unwrap_err()
            .contains("end_line must be greater than or equal to start_line"));
    }
}
