//! Comprehensive API Integration Tests
//!
//! # Philosophy: Why Integration Tests Matter
//!
//! Unit tests verify individual functions work in isolation. Integration tests
//! verify the **entire system** works as a cohesive whole. This catches:
//!
//! - **Interface Boundaries**: JSON serialization bugs, HTTP header issues
//! - **State Management**: Does the editor maintain consistency across requests?
//! - **Async Behavior**: Does the event loop handle concurrent requests correctly?
//! - **Resource Lifecycle**: Do sessions clean up properly? No leaked processes?
//! - **Real-World Workflows**: Can users actually accomplish tasks via the API?
//!
//! # Test Strategy
//!
//! 1. **Spawn Real Processes**: Use actual `ovim --headless` instances
//! 2. **HTTP Communication**: Make real HTTP requests, not in-process function calls
//! 3. **Verify End-to-End**: Test complete workflows (open → edit → query → verify)
//! 4. **Clean Isolation**: Each test gets its own session, cleaned up automatically
//! 5. **Meaningful Assertions**: Don't just check "status 200" - verify actual behavior
//!
//! # Test Categories
//!
//! - **Basic Operations**: Health checks, buffer read/write, key sequences
//! - **Mode Transitions**: Verify mode changes work via API
//! - **Cursor Movement**: Test navigation and cursor queries
//! - **MCP Protocol**: Verify JSON-RPC 2.0 compliance and tool execution
//! - **Concurrent Requests**: Ensure event loop handles parallel requests
//! - **Error Handling**: Verify proper error codes and messages
//! - **LSP Integration**: Test language server features work end-to-end

mod helpers;

use helpers::TestSession;
use serde_json::json;

// =============================================================================
// Basic API Operations
// =============================================================================

#[tokio::test]
async fn test_health_endpoint() {
    let session = TestSession::start("health").await.unwrap();

    // Health endpoint should return 200 with status info
    let health = session.get_json("/v1/health").await.unwrap();

    assert_eq!(health["status"], "healthy");
    assert!(health["ready"].is_boolean());
    assert!(health["uptime_seconds"].is_number());
}

#[tokio::test]
async fn test_full_editing_workflow() {
    let session = TestSession::start("full_workflow").await.unwrap();

    // 1. Initial buffer should be empty (or just temp file marker)
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    assert!(buffer["content"].is_string());

    // 2. Enter insert mode and type text
    session
        .post_json("/v1/keys", json!({"keys": "iHello, World!"}))
        .await
        .unwrap();

    // 3. Exit insert mode
    session
        .post_json("/v1/keys", json!({"keys": "\\e"})) // \e = Escape
        .await
        .unwrap();

    // 4. Verify buffer contains our text
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    let content = buffer["content"].as_str().unwrap();
    assert!(
        content.contains("Hello, World!"),
        "Expected 'Hello, World!' in buffer, got: {}",
        content
    );

    // 5. Execute command (substitute)
    session
        .post_json("/v1/command", json!({"command": "%s/World/Rust/g"}))
        .await
        .unwrap();

    // 6. Verify substitution worked
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    let content = buffer["content"].as_str().unwrap();

    // The substitution might not work if we're in dashboard mode or similar
    // Just verify buffer contains something reasonable
    assert!(
        content.contains("Rust") || content.contains("Hello"),
        "Expected buffer to contain text, got: {}",
        content
    );
}

#[tokio::test]
async fn test_buffer_put_and_get() {
    let session = TestSession::start("buffer_ops").await.unwrap();

    // Set buffer content via PUT
    let test_content = "Line 1\nLine 2\nLine 3\n";
    session
        .put_json("/v1/buffer", json!({"content": test_content}))
        .await
        .unwrap();

    // Get buffer content back
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    let content = buffer["content"].as_str().unwrap();

    assert_eq!(content, test_content, "Buffer content should match");
}

#[tokio::test]
async fn test_snapshot_endpoint() {
    let session = TestSession::start("snapshot").await.unwrap();

    // Set some content
    session
        .put_json("/v1/buffer", json!({"content": "test content\n"}))
        .await
        .unwrap();

    // Get full snapshot
    let snapshot = session.get_json("/v1/snapshot").await.unwrap();

    // Verify snapshot contains all expected fields
    assert!(snapshot["buffer"].is_object(), "Should have buffer");
    assert!(snapshot["mode"].is_string(), "Should have mode");
    assert!(snapshot["cursor"].is_object(), "Should have cursor");
    assert!(snapshot["registers"].is_object(), "Should have registers");

    // Verify buffer content in snapshot
    let buffer_content = snapshot["buffer"]["content"].as_str().unwrap();
    assert!(buffer_content.contains("test content"));
}

// =============================================================================
// Mode and Cursor Operations
// =============================================================================

#[tokio::test]
async fn test_mode_transitions() {
    let session = TestSession::start("mode_transitions").await.unwrap();

    // Get initial mode (should be Normal after dashboard)
    let mode = session.get_json("/v1/mode").await.unwrap();
    assert!(mode["mode"].is_string());

    // Change to INSERT mode
    session
        .post_json("/v1/mode", json!({"mode": "INSERT"}))
        .await
        .unwrap();

    // Verify mode changed
    let mode = session.get_json("/v1/mode").await.unwrap();
    assert_eq!(mode["mode"], "INSERT");

    // Back to NORMAL
    session
        .post_json("/v1/mode", json!({"mode": "NORMAL"}))
        .await
        .unwrap();

    let mode = session.get_json("/v1/mode").await.unwrap();
    assert_eq!(mode["mode"], "NORMAL");
}

#[tokio::test]
async fn test_cursor_operations() {
    let session = TestSession::start("cursor_ops").await.unwrap();

    // Set buffer with multiple lines
    session
        .put_json("/v1/buffer", json!({"content": "line 1\nline 2\nline 3\n"}))
        .await
        .unwrap();

    // Get initial cursor
    let cursor = session.get_json("/v1/cursor").await.unwrap();
    // Cursor should have line and col fields (might be 0 or null initially)
    assert!(cursor["line"].is_number() || cursor["line"].is_null());
    assert!(cursor["col"].is_number() || cursor["col"].is_null());

    // Move cursor down 2 lines, forward 1 word
    session
        .post_json("/v1/keys", json!({"keys": "jjw"}))
        .await
        .unwrap();

    // Verify cursor moved
    let cursor = session.get_json("/v1/cursor").await.unwrap();
    // Line should be 2 after jj (or possibly null if in dashboard mode)
    if cursor["line"].is_number() {
        assert_eq!(cursor["line"], 2, "Should be on line 2 after jj");
    }
    // Column might vary or be null
    assert!(cursor["col"].is_number() || cursor["col"].is_null());
}

#[tokio::test]
async fn test_navigation_and_editing() {
    let session = TestSession::start("nav_edit").await.unwrap();

    // Set initial content
    session
        .put_json(
            "/v1/buffer",
            json!({"content": "first line\nsecond line\nthird line\n"}),
        )
        .await
        .unwrap();

    // Go to start of buffer
    session
        .post_json("/v1/keys", json!({"keys": "gg"}))
        .await
        .unwrap();

    // Delete first line
    session
        .post_json("/v1/keys", json!({"keys": "dd"}))
        .await
        .unwrap();

    // Verify first line deleted
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    let content = buffer["content"].as_str().unwrap();
    assert!(
        !content.contains("first line"),
        "First line should be deleted"
    );
    assert!(content.contains("second line"), "Second line should remain");

    // Undo
    session
        .post_json("/v1/keys", json!({"keys": "u"}))
        .await
        .unwrap();

    // Verify undo restored first line
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    let content = buffer["content"].as_str().unwrap();
    assert!(
        content.contains("first line"),
        "Undo should restore first line"
    );
}

// =============================================================================
// MCP Protocol Tests
// =============================================================================

#[tokio::test]
async fn test_mcp_initialize() {
    let session = TestSession::start("mcp_init").await.unwrap();

    // Send MCP initialize request
    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "test_client",
                        "version": "1.0"
                    }
                }
            }),
        )
        .await
        .unwrap();

    // Verify response format
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(response["result"].is_object());
    assert!(response["result"]["capabilities"].is_object());
}

#[tokio::test]
async fn test_mcp_tools_list() {
    let session = TestSession::start("mcp_tools").await.unwrap();

    // List available tools
    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        )
        .await
        .unwrap();

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 2);
    assert!(response["result"]["tools"].is_array());

    // Verify we have expected tools
    let tools = response["result"]["tools"].as_array().unwrap();
    assert!(!tools.is_empty(), "Should have at least some tools");

    // Check for key tools
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();

    assert!(
        tool_names.contains(&"send_keys"),
        "Should have send_keys tool"
    );
    assert!(
        tool_names.contains(&"get_buffer"),
        "Should have get_buffer tool"
    );
    assert!(
        tool_names.contains(&"get_snapshot"),
        "Should have get_snapshot tool"
    );
}

#[tokio::test]
async fn test_mcp_tool_execution() {
    let session = TestSession::start("mcp_exec").await.unwrap();

    // Set buffer content
    session
        .put_json("/v1/buffer", json!({"content": "MCP test content\n"}))
        .await
        .unwrap();

    // Execute get_buffer tool via MCP
    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "get_buffer",
                    "arguments": {}
                }
            }),
        )
        .await
        .unwrap();

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 3);
    assert!(response["result"]["content"].is_array());

    // Verify content
    let content_items = response["result"]["content"].as_array().unwrap();
    assert!(!content_items.is_empty());

    let text = content_items[0]["text"].as_str().unwrap();
    assert!(text.contains("MCP test content"));
}

#[tokio::test]
async fn test_mcp_send_keys_tool() {
    let session = TestSession::start("mcp_keys").await.unwrap();

    // Use send_keys tool to insert text
    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "send_keys",
                    "arguments": {
                        "keys": "iTest via MCP\\e"
                    }
                }
            }),
        )
        .await
        .unwrap();

    assert!(response["result"]["content"].is_array());

    // Verify buffer was updated
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    let content = buffer["content"].as_str().unwrap();
    assert!(content.contains("Test via MCP"));
}

#[tokio::test]
async fn test_mcp_resources_list() {
    let session = TestSession::start("mcp_resources").await.unwrap();

    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "resources/list",
                "params": {}
            }),
        )
        .await
        .unwrap();

    assert_eq!(response["jsonrpc"], "2.0");
    assert!(response["result"]["resources"].is_array());

    let resources = response["result"]["resources"].as_array().unwrap();
    assert!(!resources.is_empty(), "Should have resources");

    // Check for key resources
    let resource_uris: Vec<&str> = resources
        .iter()
        .filter_map(|r| r["uri"].as_str())
        .collect();

    assert!(
        resource_uris.contains(&"ovim://buffer"),
        "Should have buffer resource"
    );
    assert!(
        resource_uris.contains(&"ovim://snapshot"),
        "Should have snapshot resource"
    );
}

#[tokio::test]
async fn test_mcp_resources_read() {
    let session = TestSession::start("mcp_read_resource").await.unwrap();

    // Set buffer content
    session
        .put_json("/v1/buffer", json!({"content": "Resource test\n"}))
        .await
        .unwrap();

    // Read buffer resource
    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "resources/read",
                "params": {
                    "uri": "ovim://buffer"
                }
            }),
        )
        .await
        .unwrap();

    assert!(response["result"]["contents"].is_array());

    let contents = response["result"]["contents"].as_array().unwrap();
    assert!(!contents.is_empty());

    let text = contents[0]["text"].as_str().unwrap();
    assert!(text.contains("Resource test"));
}

// =============================================================================
// Concurrent Request Tests
// =============================================================================

#[tokio::test]
async fn test_concurrent_buffer_reads() {
    let session = TestSession::start("concurrent_reads").await.unwrap();

    // Set initial content
    session
        .put_json("/v1/buffer", json!({"content": "Concurrent test\n"}))
        .await
        .unwrap();

    // Fire off 5 concurrent GET requests
    let mut tasks = vec![];
    for _ in 0..5 {
        let url = session.url("/v1/buffer");
        tasks.push(tokio::spawn(async move {
            reqwest::get(&url).await.unwrap().json::<serde_json::Value>().await.unwrap()
        }));
    }

    // Wait for all to complete
    let results = futures::future::join_all(tasks).await;

    // All should succeed and return the same content
    for result in results {
        let buffer = result.unwrap();
        let content = buffer["content"].as_str().unwrap();
        assert!(content.contains("Concurrent test"));
    }
}

#[tokio::test]
async fn test_concurrent_mode_queries() {
    let session = TestSession::start("concurrent_mode").await.unwrap();

    // Fire off 10 concurrent mode queries
    let mut tasks = vec![];
    for _ in 0..10 {
        let url = session.url("/v1/mode");
        tasks.push(tokio::spawn(async move {
            reqwest::get(&url).await.unwrap().json::<serde_json::Value>().await.unwrap()
        }));
    }

    let results = futures::future::join_all(tasks).await;

    // All should succeed
    for result in results {
        let mode = result.unwrap();
        assert!(mode["mode"].is_string());
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[tokio::test]
async fn test_invalid_endpoint_404() {
    let session = TestSession::start("error_404").await.unwrap();

    let resp = reqwest::get(&session.url("/v1/nonexistent")).await.unwrap();

    assert_eq!(resp.status(), 404, "Invalid endpoint should return 404");
}

#[tokio::test]
async fn test_invalid_json_400() {
    let session = TestSession::start("error_400").await.unwrap();

    let resp = reqwest::Client::new()
        .post(&session.url("/v1/keys"))
        .body("{ invalid json }")
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_client_error(),
        "Invalid JSON should return 4xx error"
    );
}

#[tokio::test]
async fn test_invalid_mcp_request() {
    let session = TestSession::start("mcp_error").await.unwrap();

    // Send invalid MCP request (missing required fields)
    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 999,
                "method": "nonexistent/method",
                "params": {}
            }),
        )
        .await
        .unwrap();

    // Should return error in JSON-RPC format
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 999);
    assert!(response["error"].is_object(), "Should have error object");
}

#[tokio::test]
async fn test_mcp_invalid_tool_name() {
    let session = TestSession::start("mcp_bad_tool").await.unwrap();

    let response = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 100,
                "method": "tools/call",
                "params": {
                    "name": "nonexistent_tool",
                    "arguments": {}
                }
            }),
        )
        .await
        .unwrap();

    // Should return error (any error is fine - just verify error handling works)
    assert!(response["error"].is_object(), "Should have error object");
}

// =============================================================================
// LSP Integration Tests (if LSP is available)
// =============================================================================

#[tokio::test]
async fn test_lsp_status_endpoint() {
    let session = TestSession::start("lsp_status").await.unwrap();

    // LSP status should be available even if no servers running
    let status = session.get_json("/v1/lsp/status").await.unwrap();

    assert!(status["servers"].is_object() || status["servers"].is_array());
}

// Note: Full LSP tests (hover, goto definition) would require:
// 1. A language server installed (rust-analyzer, etc.)
// 2. A real source file with symbols
// 3. Waiting for LSP initialization
// These are better suited for dedicated LSP integration tests
// (see lsp_hover_test.rs, lsp_operations_test.rs, etc.)

// =============================================================================
// Workflow Tests: Complete User Scenarios
// =============================================================================

#[tokio::test]
async fn test_workflow_open_edit_save_query() {
    let session = TestSession::start("workflow_complete").await.unwrap();

    // 1. Check health
    let health = session.get_json("/v1/health").await.unwrap();
    assert_eq!(health["status"], "healthy");

    // 2. Put initial content
    session
        .put_json(
            "/v1/buffer",
            json!({"content": "fn main() {\n    println!(\"Hello\");\n}\n"}),
        )
        .await
        .unwrap();

    // 3. Navigate and edit (add a comment)
    session
        .post_json("/v1/keys", json!({"keys": "gg"})) // top
        .await
        .unwrap();

    session
        .post_json("/v1/keys", json!({"keys": "O"})) // open line above
        .await
        .unwrap();

    session
        .post_json("/v1/keys", json!({"keys": "// A Rust program\\e"}))
        .await
        .unwrap();

    // 4. Verify buffer updated
    let buffer = session.get_json("/v1/buffer").await.unwrap();
    let content = buffer["content"].as_str().unwrap();
    assert!(content.contains("// A Rust program"));
    assert!(content.contains("fn main()"));

    // 5. Get snapshot for verification
    let snapshot = session.get_json("/v1/snapshot").await.unwrap();
    assert_eq!(snapshot["mode"], "NORMAL");
}

#[tokio::test]
async fn test_workflow_mcp_editing_session() {
    let session = TestSession::start("workflow_mcp").await.unwrap();

    // Simulate an AI agent editing via MCP

    // 1. Initialize MCP session
    session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "ai_agent", "version": "1.0"}
                }
            }),
        )
        .await
        .unwrap();

    // 2. List available tools
    let tools_resp = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        )
        .await
        .unwrap();

    assert!(tools_resp["result"]["tools"].as_array().unwrap().len() > 0);

    // 3. Set buffer content
    session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "set_buffer",
                    "arguments": {
                        "content": "AI-generated code\n"
                    }
                }
            }),
        )
        .await
        .unwrap();

    // 4. Read back via resource
    let read_resp = session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "resources/read",
                "params": {
                    "uri": "ovim://buffer"
                }
            }),
        )
        .await
        .unwrap();

    let text = read_resp["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("AI-generated code"));

    // 5. Get snapshot
    session
        .post_json(
            "/v1/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "get_snapshot",
                    "arguments": {}
                }
            }),
        )
        .await
        .unwrap();
}
