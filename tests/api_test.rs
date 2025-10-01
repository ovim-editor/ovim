#[cfg(test)]
mod api_tests {
    use std::time::Duration;
    use tokio::time::sleep;

    const API_BASE: &str = "http://127.0.0.1";

    async fn send_keys(port: u16, keys: &str) -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{API_BASE}:{port}/keys"))
            .header("Content-Type", "application/json")
            .body(format!(r#"{{"keys": "{}"}}"#, keys))
            .send()
            .await?;
        Ok(resp.text().await?)
    }

    async fn get_buffer(port: u16) -> Result<String, Box<dyn std::error::Error>> {
        let resp = reqwest::get(format!("{API_BASE}:{port}/buffer")).await?;
        Ok(resp.text().await?)
    }

    async fn set_buffer(port: u16, content: &str) -> Result<String, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let escaped_content = content.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        let resp = client
            .put(format!("{API_BASE}:{port}/buffer"))
            .header("Content-Type", "application/json")
            .body(format!(r#"{{"content": "{}"}}"#, escaped_content))
            .send()
            .await?;
        Ok(resp.text().await?)
    }

    async fn get_cursor(port: u16) -> Result<String, Box<dyn std::error::Error>> {
        let resp = reqwest::get(format!("{API_BASE}:{port}/cursor")).await?;
        Ok(resp.text().await?)
    }

    async fn get_mode(port: u16) -> Result<String, Box<dyn std::error::Error>> {
        let resp = reqwest::get(format!("{API_BASE}:{port}/mode")).await?;
        Ok(resp.text().await?)
    }

    // Note: These tests require a running ovim instance with --expose-rest-api
    // Run: cargo run -- test.txt --expose-rest-api
    // Then: cargo test --test api_test -- --test-threads=1

    #[tokio::test]
    #[ignore] // Ignore by default since it requires manual server start
    async fn test_basic_navigation() {
        let port = 59028; // Update this with actual port from server output

        // Set initial content
        set_buffer(port, "Line 1\nLine 2\nLine 3\nLine 4").await.unwrap();

        // Navigate to top
        send_keys(port, "gg").await.unwrap();
        sleep(Duration::from_millis(50)).await;

        let cursor = get_cursor(port).await.unwrap();
        assert!(cursor.contains(r#""line":0"#), "gg should move to line 0");

        // Move down 2
        send_keys(port, "jj").await.unwrap();
        sleep(Duration::from_millis(50)).await;

        let cursor = get_cursor(port).await.unwrap();
        assert!(cursor.contains(r#""line":2"#), "jj should move to line 2");
    }

    #[tokio::test]
    #[ignore]
    async fn test_insert_and_edit() {
        let port = 59028;

        set_buffer(port, "Hello World").await.unwrap();

        // Go to start and enter insert mode
        send_keys(port, "ggi").await.unwrap();
        sleep(Duration::from_millis(50)).await;

        let mode = get_mode(port).await.unwrap();
        assert!(mode.contains("Insert"), "Should be in Insert mode");

        // Type text
        send_keys(port, "PREFIX: ").await.unwrap();
        send_keys(port, "<Esc>").await.unwrap();
        sleep(Duration::from_millis(50)).await;

        let buffer = get_buffer(port).await.unwrap();
        assert!(buffer.contains("PREFIX:"), "Text should be inserted");
    }

    #[tokio::test]
    #[ignore]
    async fn test_delete_and_undo() {
        let port = 59028;

        set_buffer(port, "Line 1\nLine 2\nLine 3").await.unwrap();

        // Delete first line
        send_keys(port, "ggdd").await.unwrap();
        sleep(Duration::from_millis(50)).await;

        let buffer = get_buffer(port).await.unwrap();
        assert!(buffer.contains("Line 2"), "Line 2 should remain");
        assert!(!buffer.contains("Line 1"), "Line 1 should be deleted");

        // Undo
        send_keys(port, "u").await.unwrap();
        sleep(Duration::from_millis(50)).await;

        let buffer = get_buffer(port).await.unwrap();
        assert!(buffer.contains("Line 1"), "Line 1 should be back after undo");
    }
}
