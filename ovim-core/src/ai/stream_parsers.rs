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

pub async fn parse_openai_stream<E: Display>(
    mut stream: Pin<Box<dyn Stream<Item = Result<Bytes, E>> + Send>>,
    tx: UnboundedSender<StreamChunk>,
) {
    use std::future::poll_fn;
    let mut buf = SseLineBuffer::new();

    loop {
        let item = poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;

        match item {
            Some(Ok(bytes)) => {
                let lines = buf.feed(&bytes);
                for line in lines {
                    if line.starts_with("data: [DONE]") {
                        let _ = tx.send(StreamChunk::Done);
                        return;
                    }
                    if let Some(json_str) = line.strip_prefix("data: ") {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                            // Check finish_reason
                            if let Some(finish) = value
                                .get("choices")
                                .and_then(|c| c.get(0))
                                .and_then(|c| c.get("finish_reason"))
                                .and_then(|f| f.as_str())
                            {
                                if finish == "stop" || finish == "length" {
                                    // Extract any final delta content before Done
                                    if let Some(content) = value
                                        .get("choices")
                                        .and_then(|c| c.get(0))
                                        .and_then(|c| c.get("delta"))
                                        .and_then(|d| d.get("content"))
                                        .and_then(|c| c.as_str())
                                    {
                                        if !content.is_empty() {
                                            let _ =
                                                tx.send(StreamChunk::Content(content.to_string()));
                                        }
                                    }
                                    let _ = tx.send(StreamChunk::Done);
                                    return;
                                }
                            }

                            // Extract delta content
                            if let Some(content) = value
                                .get("choices")
                                .and_then(|c| c.get(0))
                                .and_then(|c| c.get("delta"))
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
    let mut current_block_type = String::new(); // "thinking" or "text"

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
                                    // Track block type
                                    if let Some(block_type) = value
                                        .get("content_block")
                                        .and_then(|b| b.get("type"))
                                        .and_then(|t| t.as_str())
                                    {
                                        current_block_type = block_type.to_string();
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
                                    // Block ended, reset type
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
                        // Check if done
                        if value.get("done").and_then(|d| d.as_bool()) == Some(true) {
                            // May contain final content
                            if let Some(content) = value
                                .get("message")
                                .and_then(|m| m.get("content"))
                                .and_then(|c| c.as_str())
                            {
                                if !content.is_empty() {
                                    let _ = tx.send(StreamChunk::Content(content.to_string()));
                                }
                            }
                            let _ = tx.send(StreamChunk::Done);
                            return;
                        }

                        // Extract content
                        if let Some(content) = value
                            .get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            if !content.is_empty() {
                                let _ = tx.send(StreamChunk::Content(content.to_string()));
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
                let _ = tx.send(StreamChunk::Done);
                return;
            }
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
}
