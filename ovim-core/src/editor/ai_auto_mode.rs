//! Auto-mode review of model-proposed shell programs.
//!
//! Auto mode runs the deterministic read-only allowlist immediately and sends
//! every other program to Terra. The same review gates a provider-owned dynamic
//! call (Codex app-server) and a local tool batch (direct Codex, OpenAI,
//! Anthropic, Ollama): only an in-scope `allow` verdict runs the program, a
//! `deny` is reported to the model, and everything else pauses for the user.

use super::ai_chat_state::{
    PendingAutoModeClassification, PendingToolApproval, ToolExecutionContinuation,
};
use super::Editor;
use crate::ai::auto_mode::{
    ClassifierDecision, ClassifierRequest, ConversationAuthorizationContext, ShellProposal,
    StaticDisposition,
};
use crate::ai::chat_types::ToolCallInfo;
use crate::ai::tools::ToolResult;
use crate::ai::ToolApprovalMode;

/// What auto mode requires before a model-proposed shell program runs.
pub(super) enum AutoModeShellReview {
    /// Nothing is left to review: the program is on the local read-only
    /// allowlist or touches only temp files this chat created.
    Proceed,
    /// Terra must return an in-scope `allow` verdict first.
    Classify(Box<ClassifierRequest>),
}

impl Editor {
    /// Whether model `bash` calls wait for Terra's review: auto mode without
    /// the per-chat YOLO bypass.
    pub(super) fn auto_mode_reviews_shell(&self) -> bool {
        !self.ai_chat_yolo_mode()
            && self.ai_state.config.tool_approval_mode == ToolApprovalMode::Auto
    }

    /// The shared auto-mode decision for a shell program, whichever provider
    /// path proposed it. `Err` carries a result for the model when the program
    /// cannot be reviewed at all.
    pub(super) fn auto_mode_shell_review(
        &self,
        command: &str,
    ) -> Result<AutoModeShellReview, String> {
        let Some(project_root) = self.ai_effective_project_root() else {
            return Err(self.no_project_root_error());
        };
        if self.current_session_authorizes_temp_shell_command(command) {
            return Ok(AutoModeShellReview::Proceed);
        }
        let request = ClassifierRequest::new(
            ShellProposal {
                command: command.to_string(),
                cwd: project_root.clone(),
                project_root: project_root.clone(),
                requested_capabilities: std::collections::BTreeSet::new(),
            },
            self.shell_authorization_context(&project_root),
        );
        if request
            .dynamic
            .static_analysis
            .disposition
            .requires_model_review()
        {
            Ok(AutoModeShellReview::Classify(Box::new(request)))
        } else {
            debug_assert_eq!(
                request.dynamic.static_analysis.disposition,
                StaticDisposition::LocallySafe
            );
            Ok(AutoModeShellReview::Proceed)
        }
    }

    pub(super) fn shell_authorization_context(
        &self,
        project_root: &std::path::Path,
    ) -> ConversationAuthorizationContext {
        use crate::ai::auto_mode::{AuthorizedObjective, ExplicitAuthorization};

        let key = self.ai_chat_conversation_key();
        let runtime_nodes = self.ai_state.conversation_runtime_nodes.get(&key);
        let mut recent = self
            .conversation()
            .map(|conversation| {
                conversation
                    .messages()
                    .iter()
                    .zip(conversation.node_ids_for_active_branch())
                    .filter(|(message, _)| message.role == crate::ai::chat_types::ChatRole::User)
                    .map(|(message, node_id)| {
                        let source_id = runtime_nodes
                            .and_then(|nodes| nodes.get(node_id))
                            .map(|reference| reference.event_id.as_str().to_string())
                            .unwrap_or_else(|| format!("ui-node:{node_id}"));
                        (message.content.clone(), source_id)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if recent.len() > 8 {
            recent.drain(..recent.len() - 8);
        }
        let explicit_user_instructions = recent
            .iter()
            .map(|(instruction, source_id)| ExplicitAuthorization {
                instruction: instruction.clone(),
                project_root: project_root.to_path_buf(),
                source_id: source_id.clone(),
            })
            .collect();
        let authorized_objectives = recent
            .last()
            .map(|(objective, source_id)| {
                vec![AuthorizedObjective {
                    objective: objective.clone(),
                    project_root: project_root.to_path_buf(),
                    source_id: source_id.clone(),
                }]
            })
            .unwrap_or_default();
        ConversationAuthorizationContext {
            explicit_user_instructions,
            authorized_objectives,
        }
    }

    /// Route a Codex dynamic `bash` call through the shared auto-mode review.
    pub(super) fn begin_dynamic_bash_auto_mode(
        &mut self,
        call: ToolCallInfo,
        response: tokio::sync::oneshot::Sender<Result<String, String>>,
        turn: crate::agent_runtime::PendingTurnRef,
        tool: crate::agent_runtime::PendingToolRef,
    ) {
        let command = shell_command(&call);
        match self.auto_mode_shell_review(&command) {
            Err(error) => {
                self.finish_dynamic_tool(&turn, &tool, &call, response, ToolResult::Error(error))
            }
            Ok(AutoModeShellReview::Proceed) => {
                self.execute_dynamic_tool_after_policy(turn, tool, call, response, None, false)
            }
            Ok(AutoModeShellReview::Classify(request)) => self.begin_auto_mode_classification(
                call,
                ToolExecutionContinuation::Dynamic {
                    runtime_tool: tool,
                    runtime_turn: turn,
                    response,
                },
                *request,
            ),
        }
    }

    /// Start Terra's review and park the turn on its verdict.
    pub(super) fn begin_auto_mode_classification(
        &mut self,
        call: ToolCallInfo,
        continuation: ToolExecutionContinuation,
        request: ClassifierRequest,
    ) {
        let operation_id = continuation
            .runtime_refs()
            .1
            .map(|tool| tool.operation_id.clone())
            .unwrap_or_default();
        let classifier = self.ai_state.shell_classifier.clone();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = classifier
                .classify(&request, &operation_id)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = result_tx.send(result);
        });
        self.park_ai_turn(PendingAutoModeClassification {
            tool_call: call,
            continuation,
            receiver: result_rx,
        });
        self.set_status_message("Terra is reviewing the proposed shell program");
    }

    pub(super) fn poll_pending_auto_mode_classification(&mut self) -> bool {
        let received = {
            let Some(pending) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.parked_as_mut::<PendingAutoModeClassification>())
            else {
                return false;
            };
            match pending.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    Some(Err("auto-mode classifier stopped without a verdict".into()))
                }
            }
        };
        let pending = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.take_parked_as::<PendingAutoModeClassification>())
            .expect("pending classifier exists");
        let (tool_call, continuation) = (pending.tool_call, pending.continuation);
        let project_root = self.ai_effective_project_root();
        match received.expect("classifier result") {
            Ok(verdict)
                if verdict.decision == ClassifierDecision::Allow
                    && project_root.as_ref() == Some(&verdict.scope.project_root) =>
            {
                self.run_classified_shell(tool_call, continuation);
            }
            Ok(verdict) if verdict.decision == ClassifierDecision::Deny => {
                self.resume_tool_continuation(
                    tool_call,
                    continuation,
                    ToolResult::Error(format!(
                        "auto mode denied shell program: {}",
                        verdict.reason
                    )),
                    "classified shell program",
                );
            }
            Ok(verdict) => self.pause_classified_shell_for_approval(
                tool_call,
                continuation,
                if verdict.decision == ClassifierDecision::Allow {
                    "classifier returned an Allow outside the active repository scope".into()
                } else {
                    verdict.reason
                },
            ),
            Err(error) => {
                crate::log_warn!("ai_auto_mode", "classifier unavailable: {error}");
                self.pause_classified_shell_for_approval(
                    tool_call,
                    continuation,
                    format!("classifier unavailable; explicit confirmation required: {error}"),
                )
            }
        }
        true
    }

    /// Run a shell program whose review is complete, on the path its
    /// continuation came from.
    pub(super) fn run_classified_shell(
        &mut self,
        tool_call: ToolCallInfo,
        continuation: ToolExecutionContinuation,
    ) {
        match continuation {
            ToolExecutionContinuation::Dynamic {
                runtime_tool,
                runtime_turn,
                response,
            } => self.execute_dynamic_tool_after_policy(
                runtime_turn,
                runtime_tool,
                tool_call,
                response,
                None,
                false,
            ),
            ToolExecutionContinuation::Batch {
                runtime_tool,
                remaining_tool_calls,
                model_name,
                ..
            } => {
                self.run_batch_shell_after_policy(
                    tool_call,
                    runtime_tool,
                    remaining_tool_calls,
                    model_name,
                );
            }
        }
    }

    fn pause_classified_shell_for_approval(
        &mut self,
        tool_call: ToolCallInfo,
        continuation: ToolExecutionContinuation,
        reason: String,
    ) {
        match continuation {
            ToolExecutionContinuation::Dynamic {
                runtime_tool,
                runtime_turn,
                response,
            } => self.pause_dynamic_tool_for_approval(
                runtime_turn,
                runtime_tool,
                tool_call,
                response,
                reason,
            ),
            ToolExecutionContinuation::Batch {
                runtime_tool,
                remaining_tool_calls,
                model_name,
                ..
            } => {
                let root = self
                    .ai_effective_project_root()
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let status = format!(
                    "Shell approval required: {reason}. Press Ctrl-Y to allow once or Ctrl-N to deny."
                );
                self.pause_for_tool_approval(PendingToolApproval {
                    tool_call,
                    reason,
                    runtime_tool,
                    runtime_tool_started: true,
                    remaining_tool_calls,
                    model_name,
                    requested_path: root.clone(),
                    approval_root: root,
                    dynamic_response: None,
                    dynamic_turn: None,
                });
                self.set_status_message(status);
            }
        }
    }
}

fn shell_command(call: &ToolCallInfo) -> String {
    call.arguments
        .get("command")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// A classifier that never answers, for tests that only observe routing and
/// must not reach a real provider.
#[cfg(test)]
pub(super) struct HeldClassifier;

#[cfg(test)]
impl crate::ai::auto_classifier::AutoModeClassifier for HeldClassifier {
    fn classify<'a>(
        &'a self,
        _request: &'a ClassifierRequest,
        _operation_id: &'a crate::run_log::OperationId,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<crate::ai::auto_mode::ClassifierVerdict>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(std::future::pending())
    }
}

#[cfg(test)]
#[path = "ai_auto_mode_tests.rs"]
mod tests;
