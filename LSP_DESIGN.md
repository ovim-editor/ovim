# LSP (Language Server Protocol) Integration Design

## Overview

Adding LSP support to ovim will provide IDE-like features:
- **Diagnostics** - Real-time error/warning detection
- **Go to Definition** - Jump to symbol definitions
- **Hover** - View documentation on hover
- **Completion** - Intelligent autocompletion
- **Code Actions** - Quick fixes and refactorings
- **Formatting** - Automatic code formatting
- **Rename** - Smart symbol renaming
- **Find References** - Find all uses of a symbol

## Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────┐
│                   Editor                        │
│  ┌──────────────┐        ┌─────────────────┐   │
│  │   Buffer     │◄──────►│  LSP Manager    │   │
│  │              │        │                 │   │
│  │ - Content    │        │ - Diagnostics   │   │
│  │ - Cursor     │        │ - Capabilities  │   │
│  │ - Version    │        │ - Requests      │   │
│  └──────────────┘        └────────┬────────┘   │
│                                   │            │
│                          ┌────────▼────────┐   │
│                          │ Language Server │   │
│                          │   (per lang)    │   │
│                          └────────┬────────┘   │
└───────────────────────────────────┼────────────┘
                                    │ JSON-RPC
                                    │ stdio
                         ┌──────────▼──────────┐
                         │  rust-analyzer      │
                         │  pyright            │
                         │  tsserver           │
                         │  etc.               │
                         └─────────────────────┘
```

### Module Structure

```
src/lsp/
├── mod.rs              # Public API and LSP manager
├── client.rs           # LSP client managing multiple servers
├── server.rs           # Individual language server process
├── protocol.rs         # JSON-RPC message handling
├── types.rs            # Helper types and conversions
├── diagnostics.rs      # Diagnostics collection and display
├── completion.rs       # Completion handling
├── definition.rs       # Go-to-definition
├── hover.rs            # Hover information
└── config.rs           # LSP configuration
```

## Core Components

### 1. LSP Manager

Central coordinator for all LSP interactions.

```rust
pub struct LspManager {
    /// Active language servers (one per language)
    servers: HashMap<Language, LanguageServer>,

    /// Configuration for each language
    config: LspConfig,

    /// Current diagnostics per file
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,

    /// Pending requests
    pending_requests: HashMap<RequestId, oneshot::Sender<Response>>,

    /// Next request ID
    next_request_id: AtomicU64,
}

impl LspManager {
    pub fn new(config: LspConfig) -> Self;
    pub async fn start_server(&mut self, language: Language) -> Result<()>;
    pub async fn stop_server(&mut self, language: Language) -> Result<()>;
    pub async fn did_open(&mut self, path: &Path, language: Language, text: &str) -> Result<()>;
    pub async fn did_change(&mut self, path: &Path, changes: Vec<TextDocumentContentChangeEvent>) -> Result<()>;
    pub async fn did_save(&mut self, path: &Path) -> Result<()>;
    pub async fn did_close(&mut self, path: &Path) -> Result<()>;
    pub async fn goto_definition(&mut self, path: &Path, position: Position) -> Result<Location>;
    pub async fn hover(&mut self, path: &Path, position: Position) -> Result<Option<Hover>>;
    pub async fn completion(&mut self, path: &Path, position: Position) -> Result<Vec<CompletionItem>>;
    pub fn get_diagnostics(&self, path: &Path) -> &[Diagnostic];
}
```

### 2. Language Server

Manages a single language server process.

```rust
pub struct LanguageServer {
    /// Language this server handles
    language: Language,

    /// Server process
    process: Child,

    /// Stdin handle
    stdin: ChildStdin,

    /// Stdout reader task
    stdout_task: JoinHandle<()>,

    /// Server capabilities
    capabilities: ServerCapabilities,

    /// Channel to send messages to server
    message_tx: mpsc::Sender<Message>,

    /// Channel to receive responses/notifications
    response_rx: mpsc::Receiver<Message>,

    /// Document versions (for didChange)
    document_versions: HashMap<PathBuf, i32>,
}

impl LanguageServer {
    pub async fn start(language: Language, command: &str, args: &[String]) -> Result<Self>;
    pub async fn initialize(&mut self, root_uri: Url) -> Result<()>;
    pub async fn send_notification(&mut self, method: &str, params: impl Serialize) -> Result<()>;
    pub async fn send_request(&mut self, method: &str, params: impl Serialize) -> Result<Value>;
    pub async fn shutdown(&mut self) -> Result<()>;
}
```

### 3. Protocol Handler

Handles JSON-RPC message serialization/deserialization.

```rust
pub struct JsonRpcMessage {
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    pub method: Option<String>,
    pub params: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<ResponseError>,
}

impl JsonRpcMessage {
    pub fn request(id: RequestId, method: String, params: Value) -> Self;
    pub fn notification(method: String, params: Value) -> Self;
    pub fn response(id: RequestId, result: Value) -> Self;
    pub fn error_response(id: RequestId, error: ResponseError) -> Self;
    pub fn parse(content: &str) -> Result<Self>;
    pub fn serialize(&self) -> Result<String>;
}

// Message framing: Content-Length header + JSON body
pub fn write_message(writer: &mut impl Write, message: &JsonRpcMessage) -> Result<()>;
pub async fn read_message(reader: &mut impl AsyncRead) -> Result<JsonRpcMessage>;
```

### 4. Configuration

Configuration for language servers.

```rust
pub struct LspConfig {
    pub servers: HashMap<Language, ServerConfig>,
}

pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub settings: Option<Value>,
}

// Example config file (LSP.toml):
// [servers.rust]
// command = "rust-analyzer"
// args = []
//
// [servers.python]
// command = "pyright-langserver"
// args = ["--stdio"]
//
// [servers.javascript]
// command = "typescript-language-server"
// args = ["--stdio"]
```

## Integration with Editor

### Buffer Changes

When buffer is modified, track changes and send to LSP:

```rust
// In Buffer::insert_text_at(), Buffer::delete_range(), etc.
impl Buffer {
    pub fn insert_text_at(&mut self, line: usize, col: usize, text: &str) {
        // ... existing code ...

        // Track change for LSP
        if let Some(ref mut lsp_tracker) = self.lsp_change_tracker {
            lsp_tracker.record_insert(line, col, text);
        }
    }

    pub fn flush_lsp_changes(&mut self) -> Vec<TextDocumentContentChangeEvent> {
        if let Some(ref mut tracker) = self.lsp_change_tracker {
            tracker.drain_changes()
        } else {
            Vec::new()
        }
    }
}
```

### Editor Integration

Add LSP manager to Editor:

```rust
pub struct Editor {
    // ... existing fields ...

    /// LSP manager (optional, only if LSP is enabled)
    lsp_manager: Option<Arc<Mutex<LspManager>>>,

    /// Task handle for LSP background tasks
    lsp_task: Option<JoinHandle<()>>,
}

impl Editor {
    pub fn enable_lsp(&mut self, config: LspConfig) -> Result<()> {
        let manager = LspManager::new(config);
        self.lsp_manager = Some(Arc::new(Mutex::new(manager)));

        // Start background task for LSP message handling
        self.lsp_task = Some(tokio::spawn(lsp_background_task(self.lsp_manager.clone())));

        Ok(())
    }

    pub async fn on_file_opened(&mut self) -> Result<()> {
        if let Some(ref lsp) = self.lsp_manager {
            let path = self.buffer.file_path().unwrap();
            let language = detect_language(path);
            let text = self.buffer.rope().to_string();

            lsp.lock().await.did_open(path, language, &text).await?;
        }
        Ok(())
    }

    pub async fn on_buffer_changed(&mut self) -> Result<()> {
        if let Some(ref lsp) = self.lsp_manager {
            let changes = self.buffer.flush_lsp_changes();
            if !changes.is_empty() {
                let path = self.buffer.file_path().unwrap();
                lsp.lock().await.did_change(path, changes).await?;
            }
        }
        Ok(())
    }
}
```

### UI Changes

Display diagnostics in the UI:

```rust
// In renderer.rs
impl Renderer {
    fn render_buffer(...) {
        // ... existing code ...

        // Render diagnostics
        if let Some(diagnostics) = editor.get_diagnostics_for_line(line_idx) {
            for diag in diagnostics {
                // Underline error ranges
                // Add error markers in margin
                // Use different colors for error/warning/info
            }
        }
    }

    fn render_status_line(...) {
        // ... existing code ...

        // Show diagnostic count
        let error_count = editor.diagnostic_count(DiagnosticSeverity::Error);
        let warning_count = editor.diagnostic_count(DiagnosticSeverity::Warning);
        // Display: "E:3 W:5"
    }
}
```

### Keybindings

Add LSP keybindings:

```rust
// In input handler (Normal mode)
Key::Char('g') => {
    if self.last_key == Some('g') {
        // gg - go to top
    } else if self.last_key == Some('d') {
        // gd - go to definition
        self.goto_definition().await?;
    }
}

Key::Char('K') => {
    // K - show hover info
    self.show_hover().await?;
}

// In Insert mode
Key::Char(c) => {
    self.insert_char(c);

    // Trigger completion on certain characters
    if is_trigger_character(c) {
        self.trigger_completion().await?;
    }
}
```

## Implementation Phases

### Phase 1: Foundation (Week 1)
- [ ] Add dependencies (lsp-types, tokio)
- [ ] Create module structure
- [ ] Implement JSON-RPC protocol handling
- [ ] Implement LanguageServer process management
- [ ] Test with mock server

### Phase 2: Document Sync (Week 2)
- [ ] Implement initialize/initialized handshake
- [ ] Implement didOpen notification
- [ ] Implement didChange with incremental sync
- [ ] Implement didSave and didClose
- [ ] Test with rust-analyzer

### Phase 3: Diagnostics (Week 3)
- [ ] Handle publishDiagnostics notification
- [ ] Store diagnostics in editor state
- [ ] Display diagnostics in UI
- [ ] Add diagnostic navigation (next/prev)

### Phase 4: Navigation (Week 4)
- [ ] Implement textDocument/definition
- [ ] Implement textDocument/hover
- [ ] Add keybindings (gd, K)
- [ ] Test with real code

### Phase 5: Completion (Week 5)
- [ ] Implement textDocument/completion
- [ ] Create completion menu UI
- [ ] Handle completion item selection
- [ ] Trigger completion on type

### Phase 6: Advanced Features (Week 6+)
- [ ] Code actions
- [ ] Formatting
- [ ] Rename
- [ ] Find references
- [ ] Signature help

### Phase 7: Polish
- [ ] Configuration file support
- [ ] Auto-detect and install servers
- [ ] Error handling and recovery
- [ ] Performance optimization
- [ ] Documentation

## Testing Strategy

### Unit Tests
- Protocol message parsing/serialization
- Change tracking logic
- Position/range conversions

### Integration Tests
```rust
#[tokio::test]
async fn test_lsp_basic_flow() {
    let config = LspConfig::default();
    let mut manager = LspManager::new(config);

    // Start server
    manager.start_server(Language::Rust).await.unwrap();

    // Open document
    manager.did_open(
        Path::new("test.rs"),
        Language::Rust,
        "fn main() { let x = 1; }"
    ).await.unwrap();

    // Wait for diagnostics
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Check diagnostics received
    let diags = manager.get_diagnostics(Path::new("test.rs"));
    assert!(!diags.is_empty());
}
```

### End-to-End Tests
- Test with real language servers (rust-analyzer, pyright, etc.)
- Test via REST API in headless mode
- Test all features (definition, hover, completion, etc.)

## Performance Considerations

1. **Async I/O** - Use tokio for non-blocking server communication
2. **Incremental Sync** - Send only changed text, not entire document
3. **Debouncing** - Don't send didChange on every keystroke, batch them
4. **Caching** - Cache hover/definition results for common queries
5. **Background Processing** - Handle LSP in separate task/thread

## Error Handling

- Server startup failures → Show warning, continue without LSP
- Server crashes → Restart automatically, show notification
- Request timeouts → Cancel request, show error
- Invalid responses → Log error, gracefully degrade
- Missing capabilities → Disable feature, inform user

## Future Enhancements

- **Multi-root workspaces** - Support multiple project roots
- **Multiple servers per language** - e.g., rust-analyzer + clippy
- **Semantic tokens** - Enhanced syntax highlighting from LSP
- **Inlay hints** - Type hints, parameter names
- **Call hierarchy** - Navigate call trees
- **Document symbols** - Outline view
- **Workspace symbols** - Project-wide symbol search
- **LSP extensions** - Server-specific features

## References

- [LSP Specification](https://microsoft.github.io/language-server-protocol/)
- [lsp-types crate](https://docs.rs/lsp-types/)
- [Helix LSP implementation](https://github.com/helix-editor/helix/tree/master/helix-lsp)
- [Neovim LSP implementation](https://github.com/neovim/neovim/tree/master/runtime/lua/vim/lsp)
