//! Lifecycle of an AI turn parked on one tool interaction: what `activity()`
//! reports for each parked kind, and how cancelling the turn tears each kind
//! down (tool results, runtime records, response channels, background tasks).

use super::ai_chat_state::{
    CodeExplanationContinuation, CodeExplanationInteraction, ParkedTurn,
    PendingAutoModeClassification, PendingBackgroundTool, PendingCodeExplanation,
    PendingShellExecution, PendingSubagentControl, PendingToolApproval, PendingWalkthroughCall,
    ShellKillHandle, ShellTranscript, ShellTranscriptPhase, ToolExecutionContinuation,
};
use super::{AiChatActivity, Editor};
use crate::agent_runtime::{PendingToolRef, PendingTurnRef};
use crate::ai::chat_types::{ChatOpts, ChatRole, ToolCallInfo};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::oneshot;

type DynamicResponse = oneshot::Receiver<Result<String, String>>;
type DynamicCall = (
    PendingTurnRef,
    PendingToolRef,
    oneshot::Sender<Result<String, String>>,
);

fn chat_editor() -> Editor {
    let mut editor = Editor::default();
    editor
        .open_ai_chat(ChatOpts {
            name: "chat".to_string(),
            allow_edits: true,
            ..Default::default()
        })
        .expect("open chat");
    editor
}

fn call(id: &str, name: &str) -> ToolCallInfo {
    ToolCallInfo {
        id: id.into(),
        name: name.into(),
        arguments: serde_json::json!({}),
    }
}

/// Commits an assistant message whose two tool_use blocks the parked turn
/// still owes results for.
fn committed_batch(editor: &mut Editor, name: &str) -> (ToolCallInfo, ToolCallInfo) {
    let first = call("tool-1", name);
    let follow_up = call("tool-2", "read_file");
    editor
        .conversation_mut()
        .unwrap()
        .append_assistant_message_with_tools(
            String::new(),
            "test".into(),
            vec![first.clone(), follow_up.clone()],
        );
    (first, follow_up)
}

fn batch(remaining: Vec<ToolCallInfo>) -> ToolExecutionContinuation {
    ToolExecutionContinuation::Batch {
        runtime_tool: None,
        runtime_turn: None,
        remaining_tool_calls: remaining,
        model_name: "test".into(),
    }
}

/// A provider-owned dynamic tool call on a live runtime turn.
fn dynamic_call(
    editor: &mut Editor,
    call: &ToolCallInfo,
) -> (
    PendingTurnRef,
    PendingToolRef,
    oneshot::Sender<Result<String, String>>,
    DynamicResponse,
) {
    let turn = editor.begin_ai_runtime_turn("dynamic").unwrap();
    let tool = editor.ai_runtime_record_tool_intent(&turn, call).unwrap();
    editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn.clone()));
    let (response, receiver) = oneshot::channel();
    (turn, tool, response, receiver)
}

fn dynamic(
    editor: &mut Editor,
    call: &ToolCallInfo,
) -> (ToolExecutionContinuation, DynamicResponse) {
    let (runtime_turn, runtime_tool, response, receiver) = dynamic_call(editor, call);
    (
        ToolExecutionContinuation::Dynamic {
            runtime_tool,
            runtime_turn,
            response,
        },
        receiver,
    )
}

fn walkthrough_batch(remaining: Vec<ToolCallInfo>) -> CodeExplanationContinuation {
    CodeExplanationContinuation::Tool(batch(remaining))
}

fn walkthrough_dynamic(
    editor: &mut Editor,
    call: &ToolCallInfo,
) -> (CodeExplanationContinuation, DynamicResponse) {
    let (continuation, receiver) = dynamic(editor, call);
    (CodeExplanationContinuation::Tool(continuation), receiver)
}

/// A task that only finishes by being aborted.
fn parked_task() -> (tokio::task::JoinHandle<()>, tokio::task::AbortHandle) {
    let task = tokio::spawn(std::future::pending::<()>());
    let abort = task.abort_handle();
    (task, abort)
}

fn approval(
    tool_call: ToolCallInfo,
    remaining: Vec<ToolCallInfo>,
    dynamic: Option<DynamicCall>,
) -> PendingToolApproval {
    let (dynamic_turn, runtime_tool, dynamic_response) = match dynamic {
        Some((turn, tool, response)) => (Some(turn), Some(tool), Some(response)),
        None => (None, None, None),
    };
    PendingToolApproval {
        tool_call,
        reason: "policy".into(),
        runtime_tool,
        runtime_tool_started: false,
        remaining_tool_calls: remaining,
        model_name: "test".into(),
        requested_path: PathBuf::from("."),
        approval_root: PathBuf::from("."),
        dynamic_response,
        dynamic_turn,
    }
}

fn shell(
    editor: &mut Editor,
    tool_call: ToolCallInfo,
    continuation: ToolExecutionContinuation,
) -> (PendingShellExecution, Arc<ShellKillHandle>) {
    let (_result_tx, receiver) = oneshot::channel();
    let (_progress_tx, progress) = tokio::sync::mpsc::unbounded_channel();
    let kill = Arc::new(ShellKillHandle::default());
    let (task, _) = parked_task();
    let mut transcript =
        ShellTranscript::new(tool_call.clone(), "sleep 60".into(), PathBuf::from("/tmp"));
    transcript.phase = ShellTranscriptPhase::Running;
    editor
        .ai_state
        .chat
        .as_mut()
        .unwrap()
        .shell_transcripts
        .insert(tool_call.id.clone(), transcript);
    (
        PendingShellExecution {
            tool_call,
            continuation,
            receiver,
            progress,
            task,
            kill: kill.clone(),
        },
        kill,
    )
}

/// Open a walkthrough view; `continuation` is the call it blocks, if any.
fn open_walkthrough(
    editor: &mut Editor,
    tool_call: ToolCallInfo,
    continuation: Option<CodeExplanationContinuation>,
) {
    let chat = editor.ai_state.chat.as_mut().unwrap();
    if let Some(continuation) = continuation {
        assert!(chat
            .park(PendingWalkthroughCall {
                tool_call: tool_call.clone(),
                continuation,
            })
            .is_ok());
    }
    chat.pending_code_explanation = Some(PendingCodeExplanation {
        tool_call,
        steps: Vec::new(),
        current: 0,
        answer_scroll: 0,
        visible_exchange: None,
        threads: Vec::new(),
        interaction: CodeExplanationInteraction::Navigating,
        original_active_buffer_id: chat.active_buffer_id,
        presentation_buffer_id: None,
        replay: false,
    });
    chat.waiting = false;
}

/// Park the turn the way each kind's producer does.
fn park(editor: &mut Editor, parked: ParkedTurn) {
    let chat = editor.ai_state.chat.as_mut().unwrap();
    chat.waiting = matches!(
        parked,
        ParkedTurn::Shell(_) | ParkedTurn::Background(_) | ParkedTurn::SubagentControl(_)
    );
    let result = match parked {
        ParkedTurn::Approval(pending) => chat.park(pending),
        ParkedTurn::Classifying(pending) => chat.park(pending),
        ParkedTurn::Shell(pending) => chat.park(pending),
        ParkedTurn::Background(pending) => chat.park(pending),
        ParkedTurn::SubagentControl(pending) => chat.park(pending),
        ParkedTurn::CodeExplanation(pending) => chat.park(pending),
    };
    assert!(result.is_ok(), "the turn was already parked");
}

fn background(tool_call: ToolCallInfo, continuation: ToolExecutionContinuation) -> ParkedTurn {
    let (_tx, receiver) = oneshot::channel();
    let (task, _) = parked_task();
    ParkedTurn::Background(PendingBackgroundTool {
        tool_call,
        continuation,
        receiver,
        task,
    })
}

fn subagent(
    tool_call: ToolCallInfo,
    continuation: ToolExecutionContinuation,
) -> (ParkedTurn, tokio::task::AbortHandle) {
    let (_tx, receiver) = oneshot::channel();
    let (task, abort) = parked_task();
    (
        ParkedTurn::SubagentControl(PendingSubagentControl {
            tool_call,
            continuation,
            receiver,
            task,
        }),
        abort,
    )
}

fn classification(editor: &mut Editor) -> (ParkedTurn, DynamicResponse) {
    let tool_call = call("classify", "bash");
    let (runtime_turn, runtime_tool, dynamic_response, receiver) = dynamic_call(editor, &tool_call);
    let (_tx, verdict) = oneshot::channel();
    (
        ParkedTurn::Classifying(PendingAutoModeClassification {
            tool_call,
            continuation: ToolExecutionContinuation::Dynamic {
                runtime_tool,
                runtime_turn,
                response: dynamic_response,
            },
            receiver: verdict,
        }),
        receiver,
    )
}

fn tool_results(editor: &Editor) -> Vec<(String, String)> {
    editor
        .conversation()
        .unwrap()
        .messages()
        .iter()
        .filter(|message| message.role == ChatRole::Tool)
        .map(|message| {
            (
                message.tool_call_id.clone().unwrap_or_default(),
                message.content.clone(),
            )
        })
        .collect()
}

/// Every committed tool_use in the batch is closed with a cancellation result.
fn assert_batch_closed(editor: &Editor) {
    let results = tool_results(editor);
    let ids: Vec<_> = results.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["tool-1", "tool-2"]);
    assert!(results
        .iter()
        .all(|(_, content)| content.contains("Execution cancelled")));
}

fn assert_cancelled_to_idle(editor: &mut Editor) {
    assert!(editor.cancel_ai_chat_generation());
    assert_eq!(editor.ai_chat_activity(), AiChatActivity::Idle);
    assert!(editor
        .ai_chat_messages()
        .iter()
        .any(|message| message.content == "Generation stopped by user."));
}

fn assert_dynamic_cancelled(mut response: DynamicResponse) {
    assert_eq!(
        response.try_recv().expect("provider was answered"),
        Err("cancelled by user".to_string())
    );
}

// ---------------------------------------------------------------------------
// activity() per parked kind
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn activity_reports_tool_approval_over_the_live_provider_turn() {
    let mut editor = chat_editor();
    let tool_call = call("approve", "bash");
    let (turn, tool, response, _receiver) = dynamic_call(&mut editor, &tool_call);
    park(
        &mut editor,
        ParkedTurn::Approval(approval(
            tool_call,
            Vec::new(),
            Some((turn, tool, response)),
        )),
    );
    assert_eq!(
        editor.ai_chat_activity(),
        AiChatActivity::WaitingToolApproval
    );
    assert_cancelled_to_idle(&mut editor);
}

#[tokio::test(flavor = "current_thread")]
async fn activity_reports_auto_mode_classification() {
    let mut editor = chat_editor();
    let (parked, _receiver) = classification(&mut editor);
    park(&mut editor, parked);
    assert_eq!(editor.ai_chat_activity(), AiChatActivity::ClassifyingTool);
    assert_cancelled_to_idle(&mut editor);
}

#[tokio::test(flavor = "current_thread")]
async fn activity_reports_running_shell() {
    let mut editor = chat_editor();
    let (pending, _kill) = shell(&mut editor, call("shell", "bash"), batch(Vec::new()));
    park(&mut editor, ParkedTurn::Shell(pending));
    assert_eq!(editor.ai_chat_activity(), AiChatActivity::RunningShell);
    assert_cancelled_to_idle(&mut editor);
}

#[tokio::test(flavor = "current_thread")]
async fn activity_reports_background_tool_as_external_work() {
    let mut editor = chat_editor();
    park(
        &mut editor,
        background(call("web", "web_fetch"), batch(Vec::new())),
    );
    assert_eq!(
        editor.ai_chat_activity(),
        AiChatActivity::RunningExternalTool
    );
    assert_cancelled_to_idle(&mut editor);
}

#[tokio::test(flavor = "current_thread")]
async fn activity_reports_a_blocking_walkthrough_over_the_live_provider_turn() {
    let mut editor = chat_editor();
    let tool_call = call("walk", "explain_with_codebase");
    let (continuation, _receiver) = walkthrough_dynamic(&mut editor, &tool_call);
    open_walkthrough(&mut editor, tool_call, Some(continuation));
    assert_eq!(
        editor.ai_chat_activity(),
        AiChatActivity::WaitingCodeExplanation
    );
    assert_cancelled_to_idle(&mut editor);
}

/// A question consumes the walkthrough's continuation; the walkthrough stays
/// open while the provider answers, so the turn is inference again.
#[tokio::test(flavor = "current_thread")]
async fn activity_reports_inference_while_an_open_walkthrough_is_answered() {
    let mut editor = chat_editor();
    let turn = editor.begin_ai_runtime_turn("answer").unwrap();
    editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn));
    open_walkthrough(&mut editor, call("walk", "explain_with_codebase"), None);
    assert_eq!(editor.ai_chat_activity(), AiChatActivity::Inference);

    editor.ai_runtime_interrupt_turn("answered");
    editor.ai_state.chat.as_mut().unwrap().runtime_turn = None;
    assert_eq!(
        editor.ai_chat_activity(),
        AiChatActivity::WaitingCodeExplanation,
        "a walkthrough that outlived its turn still needs the user"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn activity_reports_folder_approval_over_a_running_tool() {
    let mut editor = chat_editor();
    let (pending, _kill) = shell(&mut editor, call("shell", "bash"), batch(Vec::new()));
    park(&mut editor, ParkedTurn::Shell(pending));
    editor
        .ai_state
        .chat
        .as_mut()
        .unwrap()
        .pending_no_repo_folder_approval = Some(PathBuf::from("/tmp"));
    assert_eq!(
        editor.ai_chat_activity(),
        AiChatActivity::WaitingFolderApproval
    );
    assert_cancelled_to_idle(&mut editor);
}

// ---------------------------------------------------------------------------
// cancel while parked, per kind
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn cancelling_dynamic_approval_answers_the_provider() {
    let mut editor = chat_editor();
    let tool_call = call("approve", "bash");
    let (turn, tool, response, receiver) = dynamic_call(&mut editor, &tool_call);
    park(
        &mut editor,
        ParkedTurn::Approval(approval(
            tool_call,
            Vec::new(),
            Some((turn, tool, response)),
        )),
    );
    assert_cancelled_to_idle(&mut editor);
    assert_dynamic_cancelled(receiver);
    assert!(tool_results(&editor).is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_batch_approval_closes_the_committed_batch() {
    let mut editor = chat_editor();
    let (first, follow_up) = committed_batch(&mut editor, "bash");
    park(
        &mut editor,
        ParkedTurn::Approval(approval(first, vec![follow_up], None)),
    );
    assert_cancelled_to_idle(&mut editor);
    assert_batch_closed(&editor);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_classification_answers_the_provider() {
    let mut editor = chat_editor();
    let (parked, receiver) = classification(&mut editor);
    park(&mut editor, parked);
    assert_cancelled_to_idle(&mut editor);
    assert_dynamic_cancelled(receiver);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_batch_classification_closes_the_committed_batch() {
    let mut editor = chat_editor();
    let (first, follow_up) = committed_batch(&mut editor, "bash");
    let (_tx, verdict) = oneshot::channel();
    park(
        &mut editor,
        ParkedTurn::Classifying(PendingAutoModeClassification {
            tool_call: first,
            continuation: batch(vec![follow_up]),
            receiver: verdict,
        }),
    );
    assert_eq!(editor.ai_chat_activity(), AiChatActivity::ClassifyingTool);
    assert_cancelled_to_idle(&mut editor);
    assert_batch_closed(&editor);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_batch_shell_kills_retires_and_closes_the_batch() {
    let mut editor = chat_editor();
    let (first, follow_up) = committed_batch(&mut editor, "bash");
    let (pending, kill) = shell(&mut editor, first, batch(vec![follow_up]));
    park(&mut editor, ParkedTurn::Shell(pending));
    assert_cancelled_to_idle(&mut editor);
    assert!(kill.is_cancelled(), "the command itself must be killed");
    assert_eq!(
        editor.ai_state.chat.as_ref().unwrap().shell_transcripts["tool-1"].phase,
        ShellTranscriptPhase::Interrupted
    );
    assert_batch_closed(&editor);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_dynamic_shell_answers_the_provider() {
    let mut editor = chat_editor();
    let tool_call = call("shell", "bash");
    let (continuation, receiver) = dynamic(&mut editor, &tool_call);
    let (pending, kill) = shell(&mut editor, tool_call, continuation);
    park(&mut editor, ParkedTurn::Shell(pending));
    assert_cancelled_to_idle(&mut editor);
    assert!(kill.is_cancelled());
    assert_dynamic_cancelled(receiver);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_dynamic_background_tool_answers_the_provider() {
    let mut editor = chat_editor();
    let tool_call = call("web", "web_fetch");
    let (continuation, receiver) = dynamic(&mut editor, &tool_call);
    park(&mut editor, background(tool_call, continuation));
    assert_cancelled_to_idle(&mut editor);
    assert_dynamic_cancelled(receiver);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_batch_subagent_control_aborts_and_closes_the_batch() {
    let mut editor = chat_editor();
    let (first, follow_up) = committed_batch(&mut editor, "wait_agent");
    let (parked, abort) = subagent(first, batch(vec![follow_up]));
    park(&mut editor, parked);
    assert!(editor.cancel_ai_chat_generation());
    tokio::task::yield_now().await;
    assert!(abort.is_finished(), "the mailbox wait must be aborted");
    assert_batch_closed(&editor);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_dynamic_subagent_control_answers_the_provider() {
    let mut editor = chat_editor();
    let tool_call = call("wait", "wait_agent");
    let (continuation, receiver) = dynamic(&mut editor, &tool_call);
    let (parked, abort) = subagent(tool_call, continuation);
    park(&mut editor, parked);
    assert!(editor.cancel_ai_chat_generation());
    tokio::task::yield_now().await;
    assert!(abort.is_finished());
    assert_dynamic_cancelled(receiver);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_dynamic_walkthrough_closes_it_and_answers_the_provider() {
    let mut editor = chat_editor();
    let tool_call = call("walk", "explain_with_codebase");
    let (continuation, receiver) = walkthrough_dynamic(&mut editor, &tool_call);
    open_walkthrough(&mut editor, tool_call, Some(continuation));
    assert_cancelled_to_idle(&mut editor);
    assert!(!editor.ai_chat_has_pending_code_explanation());
    assert_dynamic_cancelled(receiver);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_batch_walkthrough_closes_the_committed_batch() {
    let mut editor = chat_editor();
    let (first, follow_up) = committed_batch(&mut editor, "explain_with_codebase");
    open_walkthrough(&mut editor, first, Some(walkthrough_batch(vec![follow_up])));
    assert_cancelled_to_idle(&mut editor);
    assert_batch_closed(&editor);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_editor_mcp_walkthrough_replies_to_the_external_agent() {
    let mut editor = chat_editor();
    editor.ai_state.chat.as_mut().unwrap().waiting = true;
    let (response, mut receiver) = oneshot::channel();
    open_walkthrough(
        &mut editor,
        call("walk", "explain_with_codebase"),
        Some(CodeExplanationContinuation::EditorMcp {
            request_id: "req".into(),
            rpc_id: serde_json::json!(7),
            response,
        }),
    );
    assert_cancelled_to_idle(&mut editor);
    assert!(!editor.ai_chat_has_pending_code_explanation());
    let reply = receiver.try_recv().expect("external agent was answered");
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(
        reply["result"]["content"][0]["text"],
        "Walkthrough cancelled"
    );
}

/// Regression (OV-00485): a turn parked on a delegated-agent wait/interrupt
/// was missing from the blocker list, so it was reported as inference.
#[tokio::test(flavor = "current_thread")]
async fn activity_reports_subagent_control_as_external_work() {
    let mut editor = chat_editor();
    let tool_call = call("wait", "wait_agent");
    let (continuation, _receiver) = dynamic(&mut editor, &tool_call);
    let (parked, _abort) = subagent(tool_call, continuation);
    park(&mut editor, parked);
    assert_eq!(
        editor.ai_chat_activity(),
        AiChatActivity::RunningExternalTool
    );
    assert_cancelled_to_idle(&mut editor);
}

// ---------------------------------------------------------------------------
// the one-park invariant
// ---------------------------------------------------------------------------

/// Parking an occupied turn never overwrites the occupant (that would drop
/// its response channel or orphan its task); the newcomer is cancelled.
#[tokio::test(flavor = "current_thread")]
async fn parking_an_occupied_turn_keeps_the_occupant_and_cancels_the_newcomer() {
    let mut editor = chat_editor();
    let (pending, kill) = shell(&mut editor, call("shell", "bash"), batch(Vec::new()));
    park(&mut editor, ParkedTurn::Shell(pending));

    let tool_call = call("web", "web_fetch");
    let (continuation, receiver) = dynamic(&mut editor, &tool_call);
    let ParkedTurn::Background(newcomer) = background(tool_call, continuation) else {
        unreachable!()
    };
    assert!(!editor.park_ai_turn(newcomer));

    assert_dynamic_cancelled(receiver);
    assert!(!kill.is_cancelled());
    assert_eq!(editor.ai_chat_activity(), AiChatActivity::RunningShell);
}

/// Tearing a turn down (provider error, stale branch) stops every kind of
/// parked tool work, delegated-agent control included.
#[tokio::test(flavor = "current_thread")]
async fn clearing_streaming_state_aborts_parked_subagent_control() {
    let mut editor = chat_editor();
    let (parked, abort) = subagent(call("wait", "wait_agent"), batch(Vec::new()));
    park(&mut editor, parked);

    editor.clear_streaming_state();
    tokio::task::yield_now().await;

    assert!(
        abort.is_finished(),
        "the mailbox wait must not outlive its turn"
    );
    assert!(editor.ai_state.chat.as_ref().unwrap().parked().is_none());
    assert_eq!(editor.ai_chat_activity(), AiChatActivity::Idle);
}

/// A walkthrough that outlived its turn is not replaced by a second one: the
/// new call is refused and handed back so the caller can answer it.
#[tokio::test(flavor = "current_thread")]
async fn a_second_walkthrough_is_refused_while_one_is_open() {
    let mut editor = chat_editor();
    open_walkthrough(&mut editor, call("first", "explain_with_codebase"), None);
    let second = ToolCallInfo {
        id: "second".into(),
        name: "explain_with_codebase".into(),
        arguments: serde_json::json!({
            "steps": [{ "type": "concept", "title": "Second", "body": "Body" }]
        }),
    };

    let (_, continuation) = editor
        .begin_code_explanation(second, Some(walkthrough_batch(Vec::new())))
        .expect_err("the open walkthrough keeps the screen");

    assert!(continuation.is_some(), "the refused call is handed back");
    let chat = editor.ai_state.chat.as_ref().unwrap();
    assert_eq!(
        chat.pending_code_explanation.as_ref().unwrap().tool_call.id,
        "first"
    );
    assert!(chat.parked().is_none());
}
