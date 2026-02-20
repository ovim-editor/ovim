use bytes::Bytes;
use futures_core::Stream;
use std::fmt::Display;
use std::pin::Pin;
use tokio::sync::mpsc::UnboundedSender;

use super::chat_types::StreamChunk;

// ---------------------------------------------------------------------------
// Shared SSE line buffer
// ---------------------------------------------------------------------------

struct SseLineBuffer {
    buffer: String,
}

impl SseLineBuffer {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Feed raw bytes. Returns complete lines (without trailing \n or \r\n).
    fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        let text = String::from_utf8_lossy(chunk);
        self.buffer.push_str(&text);

        let mut lines = Vec::new();
        while let Some(pos) = self.buffer.find('\n') {
            let line = self.buffer[..pos].trim_end_matches('\r').to_string();
            self.buffer.drain(..=pos);
            lines.push(line);
        }
        lines
    }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible SSE parser
// ---------------------------------------------------------------------------

/// Accumulates incremental OpenAI tool call chunks.
struct OpenAiToolAccumulator {
    /// index -> (id, name, arguments_so_far)
    calls: std::collections::HashMap<usize, (String, String, String)>,
}

impl OpenAiToolAccumulator {
    fn new() -> Self {
        Self {
            calls: std::collections::HashMap::new(),
        }
    }

    /// Process a tool_calls array from a delta. Returns true if any tool call data was found.
    fn process_delta(&mut self, tool_calls: &[serde_json::Value]) -> bool {
        let mut found = false;
        for tc in tool_calls {
            let Some(index) = tc.get("index").and_then(|i| i.as_u64()) else {
                continue;
            };
            let index = index as usize;
            found = true;

            let entry = self
                .calls
                .entry(index)
                .or_insert_with(|| (String::new(), String::new(), String::new()));

            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                entry.0 = id.to_string();
            }
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    entry.1 = name.to_string();
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    entry.2.push_str(args);
                }
            }
        }
        found
    }

    /// Emit ToolCallComplete for all accumulated calls.
    fn emit_all(&mut self, tx: &UnboundedSender<StreamChunk>) {
        let mut indices: Vec<usize> = self.calls.keys().copied().collect();
        indices.sort();
        for idx in indices {
            if let Some((id, name, args_str)) = self.calls.remove(&idx) {
                let arguments =
                    serde_json::from_str(&args_str).unwrap_or(serde_json::Value::String(args_str));
                let _ = tx.send(StreamChunk::ToolCallComplete {
                    id,
                    name,
                    arguments,
                });
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}

pub async fn parse_openai_stream<E: Display>(
    mut stream: Pin<Box<dyn Stream<Item = Result<Bytes, E>> + Send>>,
    tx: UnboundedSender<StreamChunk>,
) {
    use std::future::poll_fn;
    let mut buf = SseLineBuffer::new();
    let mut tool_acc = OpenAiToolAccumulator::new();

    loop {
        let item = poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;

        match item {
            Some(Ok(bytes)) => {
                let lines = buf.feed(&bytes);
                for line in lines {
                    if line.starts_with("data: [DONE]") {
                        // Emit any remaining tool calls before Done
                        if !tool_acc.is_empty() {
                            tool_acc.emit_all(&tx);
                        }
                        let _ = tx.send(StreamChunk::Done);
                        return;
                    }
                    if let Some(json_str) = line.strip_prefix("data: ") {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                            let delta = value
                                .get("choices")
                                .and_then(|c| c.get(0))
                                .and_then(|c| c.get("delta"));

                            // Accumulate tool calls if present
                            if let Some(tool_calls) = delta
                                .and_then(|d| d.get("tool_calls"))
                                .and_then(|t| t.as_array())
                            {
                                tool_acc.process_delta(tool_calls);
                            }

                            // Check finish_reason
                            if let Some(finish) = value
                                .get("choices")
                                .and_then(|c| c.get(0))
                                .and_then(|c| c.get("finish_reason"))
                                .and_then(|f| f.as_str())
                            {
                                match finish {
                                    "tool_calls" => {
                                        // Emit all accumulated tool calls, then Done
                                        tool_acc.emit_all(&tx);
                                        let _ = tx.send(StreamChunk::Done);
                                        return;
                                    }
                                    "stop" | "length" => {
                                        // Extract any final delta content before Done
                                        if let Some(content) = delta
                                            .and_then(|d| d.get("content"))
                                            .and_then(|c| c.as_str())
                                        {
                                            if !content.is_empty() {
                                                let _ = tx.send(StreamChunk::Content(
                                                    content.to_string(),
                                                ));
                                            }
                                        }
                                        let _ = tx.send(StreamChunk::Done);
                                        return;
                                    }
                                    _ => {}
                                }
                            }

                            // Extract delta content
                            if let Some(content) = delta
                                .and_then(|d| d.get("content"))
                                .and_then(|c| c.as_str())
                            {
                                if !content.is_empty() {
                                    let _ = tx.send(StreamChunk::Content(content.to_string()));
                                }
                            }
                        }
                    }
                }
            }
            Some(Err(e)) => {
                let _ = tx.send(StreamChunk::Error(e.to_string()));
                return;
            }
            None => {
                // Stream ended without [DONE] — graceful close
                if !tool_acc.is_empty() {
                    tool_acc.emit_all(&tx);
                }
                let _ = tx.send(StreamChunk::Done);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Anthropic SSE parser
// ---------------------------------------------------------------------------

pub async fn parse_anthropic_stream<E: Display>(
    mut stream: Pin<Box<dyn Stream<Item = Result<Bytes, E>> + Send>>,
    tx: UnboundedSender<StreamChunk>,
) {
    use std::future::poll_fn;

    let mut buf = SseLineBuffer::new();
    let mut current_event_type = String::new();
    let mut current_block_type = String::new(); // "thinking", "text", or "tool_use"

    // Accumulator for current tool_use block
    let mut tool_id = String::new();
    let mut tool_name = String::new();
    let mut tool_input_json = String::new();

    loop {
        let item = poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;

        match item {
            Some(Ok(bytes)) => {
                let lines = buf.feed(&bytes);
                for line in lines {
                    if let Some(event) = line.strip_prefix("event: ") {
                        current_event_type = event.trim().to_string();
                        continue;
                    }

                    if let Some(json_str) = line.strip_prefix("data: ") {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                            match current_event_type.as_str() {
                                "content_block_start" => {
                                    if let Some(block) = value.get("content_block") {
                                        if let Some(block_type) =
                                            block.get("type").and_then(|t| t.as_str())
                                        {
                                            current_block_type = block_type.to_string();
                                            if block_type == "tool_use" {
                                                tool_id = block
                                                    .get("id")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                                tool_name = block
                                                    .get("name")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                                tool_input_json.clear();
                                            }
                                        }
                                    }
                                }
                                "content_block_delta" => {
                                    if let Some(delta) = value.get("delta") {
                                        let delta_type = delta.get("type").and_then(|t| t.as_str());
                                        match delta_type {
                                            Some("thinking_delta") => {
                                                if let Some(text) =
                                                    delta.get("thinking").and_then(|t| t.as_str())
                                                {
                                                    if !text.is_empty() {
                                                        let _ = tx.send(StreamChunk::Thinking(
                                                            text.to_string(),
                                                        ));
                                                    }
                                                }
                                            }
                                            Some("text_delta") => {
                                                if let Some(text) =
                                                    delta.get("text").and_then(|t| t.as_str())
                                                {
                                                    if !text.is_empty() {
                                                        let _ = tx.send(StreamChunk::Content(
                                                            text.to_string(),
                                                        ));
                                                    }
                                                }
                                            }
                                            Some("input_json_delta") => {
                                                // Accumulate partial JSON for tool_use
                                                if let Some(partial) = delta
                                                    .get("partial_json")
                                                    .and_then(|v| v.as_str())
                                                {
                                                    tool_input_json.push_str(partial);
                                                }
                                            }
                                            _ => {
                                                // Fallback: use block type to determine
                                                if current_block_type == "thinking" {
                                                    if let Some(text) = delta
                                                        .get("thinking")
                                                        .and_then(|t| t.as_str())
                                                    {
                                                        if !text.is_empty() {
                                                            let _ = tx.send(StreamChunk::Thinking(
                                                                text.to_string(),
                                                            ));
                                                        }
                                                    }
                                                } else if current_block_type == "tool_use" {
                                                    if let Some(partial) = delta
                                                        .get("partial_json")
                                                        .and_then(|v| v.as_str())
                                                    {
                                                        tool_input_json.push_str(partial);
                                                    }
                                                } else if let Some(text) =
                                                    delta.get("text").and_then(|t| t.as_str())
                                                {
                                                    if !text.is_empty() {
                                                        let _ = tx.send(StreamChunk::Content(
                                                            text.to_string(),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                "content_block_stop" => {
                                    if current_block_type == "tool_use" {
                                        // Parse accumulated JSON and emit ToolCallComplete
                                        let arguments = serde_json::from_str(&tool_input_json)
                                            .unwrap_or(serde_json::Value::Object(
                                                serde_json::Map::new(),
                                            ));
                                        let _ = tx.send(StreamChunk::ToolCallComplete {
                                            id: std::mem::take(&mut tool_id),
                                            name: std::mem::take(&mut tool_name),
                                            arguments,
                                        });
                                        tool_input_json.clear();
                                    }
                                    current_block_type.clear();
                                }
                                "message_stop" => {
                                    let _ = tx.send(StreamChunk::Done);
                                    return;
                                }
                                "error" => {
                                    let msg = value
                                        .get("error")
                                        .and_then(|e| e.get("message"))
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("unknown Anthropic error");
                                    let _ = tx.send(StreamChunk::Error(msg.to_string()));
                                    return;
                                }
                                _ => {
                                    // message_start, ping, etc. — ignore
                                }
                            }
                        }
                    }
                }
            }
            Some(Err(e)) => {
                let _ = tx.send(StreamChunk::Error(e.to_string()));
                return;
            }
            None => {
                // Stream ended without message_stop — graceful
                let _ = tx.send(StreamChunk::Done);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ollama NDJSON parser
// ---------------------------------------------------------------------------

pub async fn parse_ollama_stream<E: Display>(
    mut stream: Pin<Box<dyn Stream<Item = Result<Bytes, E>> + Send>>,
    tx: UnboundedSender<StreamChunk>,
) {
    use std::future::poll_fn;

    let mut buf = SseLineBuffer::new();
    let mut accumulated_content = String::new();
    let mut got_structured_tool_calls = false;

    loop {
        let item = poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;

        match item {
            Some(Ok(bytes)) => {
                let lines = buf.feed(&bytes);
                for line in lines {
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                        // Extract content
                        if let Some(content) = value
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            if !content.is_empty() {
                                accumulated_content.push_str(content);
                                let _ = tx.send(StreamChunk::Content(content.to_string()));
                            }
                        }

                        // Extract tool calls (Ollama sends these on the done message
                        // or as non-streaming responses)
                        if let Some(tool_calls) = value
                            .get("message")
                            .and_then(|m| m.get("tool_calls"))
                            .and_then(|tc| tc.as_array())
                        {
                            for (i, tc) in tool_calls.iter().enumerate() {
                                let name = tc
                                    .get("function")
                                    .and_then(|f| f.get("name"))
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("");
                                let arguments = tc
                                    .get("function")
                                    .and_then(|f| f.get("arguments"))
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Object(Default::default()));
                                if !name.is_empty() {
                                    got_structured_tool_calls = true;
                                    let _ = tx.send(StreamChunk::ToolCallComplete {
                                        id: format!("ollama-tc-{}", i),
                                        name: name.to_string(),
                                        arguments,
                                    });
                                }
                            }
                        }

                        // Check if done
                        if value.get("done").and_then(|d| d.as_bool()) == Some(true) {
                            // If no structured tool_calls were received, try to
                            // parse the accumulated content as tool-call JSON.
                            // Some models emit raw JSON in content instead of
                            // using the structured tool_calls field.
                            if !got_structured_tool_calls {
                                try_extract_tool_calls_from_content(
                                    &accumulated_content,
                                    &tx,
                                );
                            }
                            let _ = tx.send(StreamChunk::Done);
                            return;
                        }
                    }
                }
            }
            Some(Err(e)) => {
                let _ = tx.send(StreamChunk::Error(e.to_string()));
                return;
            }
            None => {
                let _ = tx.send(StreamChunk::Done);
                return;
            }
        }
    }
}

/// Attempt to parse tool call JSON that a model emitted in `message.content`
/// instead of using structured `tool_calls`. Supports two patterns:
///
/// 1. A single object: `{"name": "tool", "arguments": {...}}`
/// 2. An array of objects: `[{"name": "tool", "arguments": {...}}, ...]`
///
/// If parsing succeeds, sends `ToolCallComplete` chunks. The content chunks
/// were already sent to the consumer, so the consumer will see both — the
/// `ToolCallComplete` presence will cause the tool-call path to be taken,
/// and the raw text content is ignored for that turn.
fn try_extract_tool_calls_from_content(
    content: &str,
    tx: &UnboundedSender<StreamChunk>,
) {
    let trimmed = content.trim();
    if trimmed.is_empty() || !trimmed.starts_with(['{', '[']) {
        return;
    }

    // Try to parse as JSON
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return;
    };

    let calls: Vec<&serde_json::Value> = if let Some(arr) = value.as_array() {
        arr.iter().collect()
    } else if value.is_object() {
        vec![&value]
    } else {
        return;
    };

    for (i, tc) in calls.iter().enumerate() {
        // Accept both {"name": ..., "arguments": ...} and
        // {"function": {"name": ..., "arguments": ...}} formats
        let (name, arguments) = if let Some(func) = tc.get("function") {
            let n = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let a = func
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            (n, a)
        } else {
            let n = tc.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let a = tc
                .get("arguments")
                .or_else(|| tc.get("parameters"))
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            (n, a)
        };

        if !name.is_empty() {
            let _ = tx.send(StreamChunk::ToolCallComplete {
                id: format!("ollama-content-tc-{}", i),
                name: name.to_string(),
                arguments,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::sync::mpsc;

    /// Helper: turn a Vec<&str> into a pinned stream of Result<Bytes, String>.
    fn make_stream(chunks: Vec<&str>) -> Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>> {
        let items: Vec<Result<Bytes, String>> = chunks
            .into_iter()
            .map(|s| Ok(Bytes::from(s.to_string())))
            .collect();
        Box::pin(TestStream {
            items: items.into_iter().collect(),
        })
    }

    fn make_error_stream(
        chunks: Vec<Result<&str, &str>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>> {
        let items: Vec<Result<Bytes, String>> = chunks
            .into_iter()
            .map(|r| match r {
                Ok(s) => Ok(Bytes::from(s.to_string())),
                Err(e) => Err(e.to_string()),
            })
            .collect();
        Box::pin(TestStream {
            items: items.into_iter().collect(),
        })
    }

    struct TestStream {
        items: std::collections::VecDeque<Result<Bytes, String>>,
    }

    impl Stream for TestStream {
        type Item = Result<Bytes, String>;
        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match self.items.pop_front() {
                Some(item) => Poll::Ready(Some(item)),
                None => Poll::Ready(None),
            }
        }
    }

    fn collect_chunks(rx: &mut mpsc::UnboundedReceiver<StreamChunk>) -> Vec<StreamChunk> {
        let mut chunks = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            chunks.push(chunk);
        }
        chunks
    }

    // ---- OpenAI tests ----

    #[tokio::test]
    async fn openai_happy_path() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
            "data: [DONE]\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "Hello"));
        assert!(matches!(&chunks[1], StreamChunk::Content(s) if s == " world"));
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    #[tokio::test]
    async fn openai_partial_line_buffering() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Split a single SSE event across two byte chunks
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"del",
            "ta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "Hi"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    #[tokio::test]
    async fn openai_finish_reason_without_done() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "Hi"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    #[tokio::test]
    async fn openai_empty_content_skipped() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"X\"}}]}\n\n",
            "data: [DONE]\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2); // Empty skipped
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "X"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    // ---- Anthropic tests ----

    #[tokio::test]
    async fn anthropic_thinking_and_content() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"thinking\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me think\"}}\n\n",
            "event: content_block_stop\ndata: {}\n\n",
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello!\"}}\n\n",
            "event: content_block_stop\ndata: {}\n\n",
            "event: message_stop\ndata: {}\n\n",
        ]);
        parse_anthropic_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3);
        assert!(matches!(&chunks[0], StreamChunk::Thinking(s) if s == "Let me think"));
        assert!(matches!(&chunks[1], StreamChunk::Content(s) if s == "Hello!"));
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    #[tokio::test]
    async fn anthropic_content_only() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"No thinking here\"}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        ]);
        parse_anthropic_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "No thinking here"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    #[tokio::test]
    async fn anthropic_error_event() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "event: error\ndata: {\"error\":{\"message\":\"rate limited\"}}\n\n",
        ]);
        parse_anthropic_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Error(s) if s == "rate limited"));
    }

    // ---- Ollama tests ----

    #[tokio::test]
    async fn ollama_happy_path() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"Hi\"},\"done\":false}\n",
            "{\"message\":{\"content\":\" there\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"\"},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "Hi"));
        assert!(matches!(&chunks[1], StreamChunk::Content(s) if s == " there"));
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    #[tokio::test]
    async fn ollama_partial_json_line() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // JSON split across byte chunks
        let stream = make_stream(vec![
            "{\"message\":{\"con",
            "tent\":\"OK\"},\"done\":false}\n{\"message\":{\"content\":\"\"},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "OK"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    #[tokio::test]
    async fn ollama_tool_call_on_done() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"read_file\",\"arguments\":{\"start_line\":1}}}]},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2); // ToolCallComplete + Done
        match &chunks[0] {
            StreamChunk::ToolCallComplete {
                id,
                name,
                arguments,
            } => {
                assert!(id.starts_with("ollama-tc-"));
                assert_eq!(name, "read_file");
                assert_eq!(arguments["start_line"], 1);
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    #[tokio::test]
    async fn ollama_tool_call_with_content() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"Let me check.\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"read_diagnostics\",\"arguments\":{}}}]},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3); // Content + ToolCallComplete + Done
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "Let me check."));
        match &chunks[1] {
            StreamChunk::ToolCallComplete { name, .. } => {
                assert_eq!(name, "read_diagnostics");
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    #[tokio::test]
    async fn ollama_multiple_tool_calls() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"read_file\",\"arguments\":{}}},{\"function\":{\"name\":\"read_diagnostics\",\"arguments\":{}}}]},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3); // 2 ToolCallComplete + Done
        match &chunks[0] {
            StreamChunk::ToolCallComplete { id, name, .. } => {
                assert_eq!(id, "ollama-tc-0");
                assert_eq!(name, "read_file");
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        match &chunks[1] {
            StreamChunk::ToolCallComplete { id, name, .. } => {
                assert_eq!(id, "ollama-tc-1");
                assert_eq!(name, "read_diagnostics");
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    // ---- Stream disconnect ----

    #[tokio::test]
    async fn stream_disconnect_sends_done() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Stream ends without [DONE]
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "partial"));
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    #[tokio::test]
    async fn stream_byte_error() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_error_stream(vec![
            Ok("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n"),
            Err("connection reset"),
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "ok"));
        assert!(matches!(&chunks[1], StreamChunk::Error(s) if s == "connection reset"));
    }

    // ---- OpenAI tool call tests ----

    #[tokio::test]
    async fn openai_tool_call_single() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"start_line\\\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": 1}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2); // ToolCallComplete + Done
        match &chunks[0] {
            StreamChunk::ToolCallComplete {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read_file");
                assert_eq!(arguments["start_line"], 1);
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    #[tokio::test]
    async fn openai_tool_call_with_content() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"Let me check.\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_2\",\"type\":\"function\",\"function\":{\"name\":\"read_diagnostics\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3); // Content + ToolCallComplete + Done
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "Let me check."));
        match &chunks[1] {
            StreamChunk::ToolCallComplete { id, name, .. } => {
                assert_eq!(id, "call_2");
                assert_eq!(name, "read_diagnostics");
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    #[tokio::test]
    async fn openai_multiple_tool_calls() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"type\":\"function\",\"function\":{\"name\":\"read_diagnostics\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
        ]);
        parse_openai_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3); // 2 ToolCallComplete + Done
        match &chunks[0] {
            StreamChunk::ToolCallComplete { name, .. } => assert_eq!(name, "read_file"),
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        match &chunks[1] {
            StreamChunk::ToolCallComplete { name, .. } => assert_eq!(name, "read_diagnostics"),
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    // ---- Anthropic tool call tests ----

    #[tokio::test]
    async fn anthropic_tool_use_block() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Checking.\"}}\n\n",
            "event: content_block_stop\ndata: {}\n\n",
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"read_file\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"start\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"_line\\\": 1}\"}}\n\n",
            "event: content_block_stop\ndata: {}\n\n",
            "event: message_stop\ndata: {}\n\n",
        ]);
        parse_anthropic_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 3); // Content + ToolCallComplete + Done
        assert!(matches!(&chunks[0], StreamChunk::Content(s) if s == "Checking."));
        match &chunks[1] {
            StreamChunk::ToolCallComplete {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "read_file");
                assert_eq!(arguments["start_line"], 1);
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[2], StreamChunk::Done));
    }

    #[tokio::test]
    async fn anthropic_tool_use_only() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"read_diagnostics\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
            "event: content_block_stop\ndata: {}\n\n",
            "event: message_stop\ndata: {}\n\n",
        ]);
        parse_anthropic_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        assert_eq!(chunks.len(), 2); // ToolCallComplete + Done
        match &chunks[0] {
            StreamChunk::ToolCallComplete { id, name, .. } => {
                assert_eq!(id, "toolu_2");
                assert_eq!(name, "read_diagnostics");
            }
            other => panic!("expected ToolCallComplete, got: {:?}", other),
        }
        assert!(matches!(&chunks[1], StreamChunk::Done));
    }

    // ---- Ollama content-based tool call extraction ----

    #[tokio::test]
    async fn ollama_content_tool_call_single_object() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Model emits tool call JSON as content text, no structured tool_calls
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"{\\\"name\\\": \\\"read_file\\\", \\\"arguments\\\": {\\\"path\\\": \\\"src/main.rs\\\"}}\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"\"},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        // Content + ToolCallComplete + Done
        assert!(chunks.iter().any(|c| matches!(c, StreamChunk::ToolCallComplete { name, .. } if name == "read_file")));
        assert!(matches!(chunks.last().unwrap(), StreamChunk::Done));
    }

    #[tokio::test]
    async fn ollama_content_tool_call_array() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"[{\\\"name\\\": \\\"read_file\\\", \\\"arguments\\\": {}}, {\\\"name\\\": \\\"list_files\\\", \\\"arguments\\\": {}}]\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"\"},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        let tc_count = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::ToolCallComplete { .. }))
            .count();
        assert_eq!(tc_count, 2);
    }

    #[tokio::test]
    async fn ollama_content_not_tool_call_json() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Normal text that happens to start with { but isn't a tool call
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"Here is some code: {}\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"\"},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        let tc_count = chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::ToolCallComplete { .. }))
            .count();
        // No tool calls extracted — `Here is some code: {}` has no "name" field
        assert_eq!(tc_count, 0);
    }

    #[tokio::test]
    async fn ollama_structured_tool_calls_skip_content_parsing() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Has BOTH structured tool_calls AND JSON-looking content — structured wins
        let stream = make_stream(vec![
            "{\"message\":{\"content\":\"{\\\"name\\\": \\\"wrong_tool\\\", \\\"arguments\\\": {}}\",\"tool_calls\":[{\"function\":{\"name\":\"real_tool\",\"arguments\":{}}}]},\"done\":true}\n",
        ]);
        parse_ollama_stream(stream, tx).await;
        let chunks = collect_chunks(&mut rx);
        let tc_names: Vec<&str> = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::ToolCallComplete { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        // Only the structured tool call, not the content-parsed one
        assert_eq!(tc_names, vec!["real_tool"]);
    }
}
