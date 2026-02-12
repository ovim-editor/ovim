# Provider Layer

The provider layer handles communication with AI models: building requests, sending them over HTTP, streaming responses, and extracting edits. It supports both single-shot (selection context) and multi-turn (chat/query contexts) interactions.

## Provider Abstraction

Three wire formats, shared by all models:

| Provider | API Format | Auth | Streaming |
|----------|-----------|------|-----------|
| `openai` | OpenAI Chat Completions | Bearer token | SSE (`stream: true`) |
| `anthropic` | Anthropic Messages | `x-api-key` header | SSE (`stream: true`) |
| `ollama` | Ollama Chat | None (local) | NDJSON (default streaming) |

Any OpenAI-compatible endpoint (DeepSeek, Together, Groq, vLLM, etc.) uses the `openai` provider with a custom `base_url`.

## Request Types

### Single-Shot (Selection Context)

The existing `request_ai_edit()` function. One user message, one response. No conversation history.

```rust
pub async fn request_ai_edit(
    profile: &AiProfileConfig,
    request: &AiRequest,
) -> Result<AiJobResult>
```

Used by the selection context. Fast, minimal overhead.

### Multi-Turn (Chat/Query Contexts)

New function for conversational interactions:

```rust
pub async fn request_ai_chat(
    profile: &ResolvedProfile,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSchema>,
    stream_tx: mpsc::Sender<StreamChunk>,
) -> Result<()>
```

Sends the full conversation history. Supports tool definitions (for models with function calling). Streams response chunks through the channel.

### ChatMessage

Maps cleanly to all three provider APIs:

```rust
pub struct ChatMessage {
    pub role: ChatMessageRole,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,      // Assistant messages with tool use
    pub tool_call_id: Option<String>,           // Tool result messages
}

pub enum ChatMessageRole {
    System,
    User,
    Assistant,
    Tool,
}
```

### ToolSchema

Generated from `ToolDefinition` at request time, format-specific:

```rust
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,  // JSON Schema object
}
```

Conversion to provider-specific format happens in the provider adapter:
- **OpenAI**: `tools: [{ type: "function", function: { name, description, parameters } }]`
- **Anthropic**: `tools: [{ name, description, input_schema }]`
- **Ollama**: Not all models support tools; falls back to format-based editing

## Streaming

### StreamChunk

The streaming channel carries typed chunks:

```rust
pub enum StreamChunk {
    /// Chain-of-thought / thinking tokens (Anthropic extended thinking, etc.)
    Thinking(String),

    /// Content tokens (the actual response text)
    Content(String),

    /// Model wants to call a tool
    ToolCall {
        id: String,
        name: String,
        arguments: String,  // JSON string, may arrive incrementally
    },

    /// Tool call arguments complete, ready to execute
    ToolCallComplete {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    /// Response complete
    Done,

    /// Error during streaming
    Error(String),
}
```

### Provider-Specific Streaming

**OpenAI SSE**:
```
data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}
data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"..."}}]},"finish_reason":null}]}
data: {"choices":[{"delta":{},"finish_reason":"stop"}]}
data: [DONE]
```

**Anthropic SSE**:
```
event: content_block_start
data: {"type":"content_block_start","content_block":{"type":"thinking","thinking":"..."}}

event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"..."}}

event: content_block_start
data: {"type":"content_block_start","content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}

event: content_block_start
data: {"type":"content_block_start","content_block":{"type":"tool_use","name":"edit_diff"}}

event: message_stop
```

**Ollama NDJSON**:
```json
{"message":{"content":"Hello"},"done":false}
{"message":{"content":" world"},"done":false}
{"message":{"content":""},"done":true}
```

Each provider adapter parses its stream format and emits uniform `StreamChunk` values through the channel.

## Edit Modes

### Mode 1: Tool Calling (`edit_mode = "tools"`)

The model calls edit tools explicitly. The response contains structured tool calls:

```json
{
  "tool_calls": [{
    "name": "edit_diff",
    "arguments": {
      "diff": "--- src/main.rs\n+++ src/main.rs\n@@ -10,3 +10,3 @@\n-    old line\n+    new line"
    }
  }]
}
```

Flow:
1. Model emits `ToolCall` chunks during streaming
2. When `ToolCallComplete` arrives, execute the tool through the permission/scope pipeline
3. Send tool result back to the model as a `Tool` message
4. Model may make more tool calls or finish with a text response

This is the most reliable mode for capable models. The tool schema constrains the output format.

### Mode 2: Format Parsing (`edit_mode = "format"`)

The model outputs edits embedded in prose. The `edit_format` determines how to parse them.

Flow:
1. Model streams text content
2. When `Done` arrives, run the format parser on the full response
3. Parser extracts `Vec<EditTuple>` from the text
4. Apply edits through the normal pipeline

This mode works with any model (no tool calling required) but is less reliable — the model might deviate from the expected format.

### Edit Format Parsers

Each format provides:
- **Instruction text**: Injected into the system prompt
- **Parser function**: Extracts edits from response text

```rust
pub struct EditFormat {
    pub name: String,
    pub instruction: String,
    pub parser: EditParser,
}

pub enum EditParser {
    Builtin(fn(&str) -> Result<Vec<EditTuple>>),
    Lua(String),  // Lua function name
}

pub struct EditTuple {
    pub file: String,       // File path (validated against scope)
    pub start_line: usize,
    pub end_line: usize,
    pub new_content: String,
}
```

### Built-in Format Parsers

**`diff`**: Parses unified diff blocks from fenced code blocks:
```
```diff
--- src/main.rs
+++ src/main.rs
@@ -10,3 +10,3 @@
-    old line
+    new line
```​
```

**`codeblock`**: Extracts the first fenced code block as replacement for the entire selection:
```
Here's the fix:
```rust
fn corrected() { ... }
```​
```

**`range`**: Parses JSON edit objects:
```json
{"file": "src/main.rs", "start": 10, "end": 15, "content": "new code\nhere\n"}
```

**`full_file`**: The entire response (after stripping markdown fencing) replaces the file content.

**`search_replace`**: Parses search/replace pairs:
```json
{"search": "old_function_name", "replace": "new_function_name"}
```

## System Prompt Assembly

The system prompt is assembled from:

1. **Context template**: Selection, chat, or query specific preamble
2. **Buffer context**: File path, language, relevant code content
3. **Tool definitions** (tools mode): JSON schemas for available tools
4. **Edit format instruction** (format mode): How to express edits
5. **User's `system_prompt_extra`**: Profile-level additions

### Context Templates

**Selection**:
```
You are a code editor. Edit the selected text based on the instruction.
Return only the replacement code.
```

**Chat**:
```
You are a code editor assistant integrated into a terminal text editor.
You can read and modify code using the tools provided.
The user is editing {file_path} ({language}).
Make changes by calling the appropriate edit tools.
Explain your reasoning briefly.
```

**Query**:
```
You are a code analysis assistant.
Answer questions about the code clearly and precisely.
You can read files and search the project to find relevant code.
Cite specific locations (file:line) when referencing code.
You cannot modify any files.
```

## Conversation History Management

For multi-turn contexts, the full branch history is sent:

```rust
fn build_messages(tree: &ConversationTree) -> Vec<ChatMessage> {
    let branch = tree.active_branch();
    let mut messages = Vec::new();

    for node in branch {
        match node.role {
            ChatRole::User => {
                messages.push(ChatMessage {
                    role: ChatMessageRole::User,
                    content: node.content.clone(),
                    ..Default::default()
                });
            }
            ChatRole::Assistant => {
                messages.push(ChatMessage {
                    role: ChatMessageRole::Assistant,
                    content: node.content.clone(),
                    tool_calls: /* reconstruct from node.edits if any */,
                    ..Default::default()
                });
            }
            ChatRole::Thinking => {
                // Thinking blocks are NOT sent back to the model
                // (they're CoT artifacts, not conversation turns)
            }
        }
    }

    messages
}
```

### Context Budget

The conversation history grows with each turn. When approaching the model's context limit:

1. Display token count in the model selector bar (yellow/red warnings)
2. Optionally summarize older messages (compress early turns into a summary)
3. Never silently drop messages — the user should know when context is tight

## Event Loop Integration

The streaming receiver is polled in the event loop alongside terminal input and tick events:

```rust
// In the select! macro
stream_chunk = chat_rx.recv() => {
    match stream_chunk {
        Some(StreamChunk::Content(text)) => {
            editor.ai_chat.append_streaming_content(&text);
            editor.mark_dirty();
        }
        Some(StreamChunk::ToolCallComplete { name, arguments, .. }) => {
            let result = editor.execute_tool_call(&name, &arguments)?;
            // Send result back to model (via a response channel)
            tool_result_tx.send(result)?;
        }
        Some(StreamChunk::Done) => {
            editor.ai_chat.finalize_streaming();
            editor.mark_dirty();
        }
        // ...
    }
}
```

This keeps the UI responsive during streaming — each chunk triggers a re-render that shows the new tokens.
