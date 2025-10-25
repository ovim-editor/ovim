use ovim::editor::Editor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

// Global channel for Java LSP status updates
static JAVA_STATUS_SENDER: OnceLock<mpsc::UnboundedSender<String>> = OnceLock::new();

/// Helper to send Java status updates
pub fn send_java_status(msg: String) {
    if let Some(tx) = JAVA_STATUS_SENDER.get() {
        let _ = tx.send(format!("Java: {}", msg));
    }
}

/// Initialize the Java status sender (called from main)
pub fn init_java_status_sender(sender: mpsc::UnboundedSender<String>) {
    JAVA_STATUS_SENDER.set(sender).ok();
}

/// Find the root of a JVM project (Maven or Gradle)
/// Searches parent directories for pom.xml, build.gradle, build.gradle.kts, or settings.gradle
pub fn find_jvm_project_root(file_path: &Path) -> &Path {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        // Check for Maven project (pom.xml)
        if dir.join("pom.xml").exists() {
            return dir;
        }
        // Check for Gradle project (build.gradle, build.gradle.kts, or settings.gradle)
        if dir.join("build.gradle").exists()
            || dir.join("build.gradle.kts").exists()
            || dir.join("settings.gradle").exists()
            || dir.join("settings.gradle.kts").exists()
        {
            return dir;
        }
        current = dir.parent();
    }
    // Fall back to file's parent directory if no project root found
    file_path.parent().unwrap_or_else(|| Path::new("/"))
}

/// Handle Java LSP initialization (for TUI mode - spawns background task)
pub async fn handle_java_lsp(editor: &mut Editor, abs_path: PathBuf) {
    // We need to move values into the spawned task, so clone what we need
    let abs_path_clone = abs_path.clone();
    let lsp_manager = editor.lsp_manager();

    // Spawn Java LSP initialization in background
    tokio::spawn(async move {
        initialize_java_lsp_background(lsp_manager, abs_path_clone).await;
    });

    // Initial status will be updated immediately by the background task
}

/// Background Java LSP initialization that doesn't block the UI
pub async fn initialize_java_lsp_background(
    lsp_manager: Option<Arc<ovim::lsp::LspManager>>,
    file_path: PathBuf,
) {
    use ovim::java::{parser, JdtlsDownloader, JdtlsLauncher};

    ovim::lsp_debug!("Java", "Background task started for {:?}", file_path);

    // Early exit if no LSP manager
    let Some(lsp_manager) = lsp_manager else {
        send_java_status("No LSP manager available".to_string());
        return;
    };

    // Find project root
    let project_root = find_jvm_project_root(&file_path);
    ovim::lsp_debug!("Java", "Project root: {:?}", project_root);

    send_java_status("Detecting project configuration...".to_string());
    ovim::lsp_debug!("Java", "Sent status: Detecting project configuration...");

    // Detect Java version from build files
    let project_config = match parser::detect_java_version(project_root).await {
        Ok(config) => config,
        Err(e) => {
            send_java_status(format!("Failed to detect version: {}", e));
            return;
        }
    };

    send_java_status(format!(
        "Detected Java {} project",
        project_config.java_version.as_str()
    ));

    // Get jdtls installation directory
    let jdtls_dir = match ovim::java::jdtls_dir().await {
        Ok(dir) => dir,
        Err(e) => {
            send_java_status(format!("Failed to get cache dir: {}", e));
            return;
        }
    };

    // Ensure jdtls is installed
    let downloader = JdtlsDownloader::new(jdtls_dir.clone());

    if !downloader.is_installed().await {
        send_java_status("Downloading jdtls... (first time setup)".to_string());

        match downloader
            .ensure_installed(|msg| {
                send_java_status(msg);
            })
            .await
        {
            Ok(()) => send_java_status("Download complete!".to_string()),
            Err(e) => {
                send_java_status(format!("Download failed: {}", e));
                return;
            }
        }
    } else {
        send_java_status("Using cached jdtls".to_string());
    }

    // Ensure Lombok is installed
    if !downloader.is_lombok_installed().await {
        send_java_status("Downloading Lombok... (first time setup)".to_string());

        match downloader
            .ensure_lombok_installed(|msg| {
                send_java_status(msg);
            })
            .await
        {
            Ok(()) => send_java_status("Lombok download complete!".to_string()),
            Err(e) => {
                send_java_status(format!("Lombok download failed: {}", e));
                // Non-fatal: continue without Lombok
            }
        }
    } else {
        send_java_status("Using cached Lombok".to_string());
    }

    // Get Lombok JAR path (if installed)
    let lombok_jar = if downloader.is_lombok_installed().await {
        Some(downloader.lombok_jar_path())
    } else {
        None
    };

    // Get workspace directory
    let workspace_dir = match ovim::java::workspace_dir(project_root).await {
        Ok(dir) => dir,
        Err(e) => {
            send_java_status(format!("Failed to create workspace: {}", e));
            return;
        }
    };

    send_java_status("Configuring launcher...".to_string());

    // Create launcher
    let launcher =
        JdtlsLauncher::from_project_config(project_config, jdtls_dir, workspace_dir, lombok_jar);

    send_java_status("Finding JVM...".to_string());

    // Get launch command (async JVM detection)
    let launch_args = match launcher.launch_command().await {
        Ok(args) => {
            send_java_status("JVM found, launching jdtls...".to_string());
            args
        }
        Err(e) => {
            send_java_status(format!("Failed to find JVM: {}", e));
            return;
        }
    };

    // Extract java command and args
    if launch_args.is_empty() {
        send_java_status("Invalid launch configuration".to_string());
        return;
    }

    let server_command = &launch_args[0];
    let server_args: Vec<String> = launch_args[1..].to_vec();

    send_java_status("Starting LSP server...".to_string());
    ovim::lsp_debug!("Java", "About to spawn start_server task");
    ovim::lsp_debug!("Java", "Server command: {:?}", server_command);
    ovim::lsp_debug!("Java", "Server args: {:?}", server_args);

    // Start the LSP server with progress updates during initialization
    // jdtls can take 60-120 seconds to initialize, so we send periodic updates
    let lsp_clone = lsp_manager.clone();
    let server_command_clone = server_command.to_string();
    let server_args_clone = server_args.clone();
    let project_root_clone = project_root.to_path_buf();

    let mut start_task = tokio::spawn(async move {
        ovim::lsp_debug!("Java", "Inside start_server task...");
        let result = lsp_clone
            .start_server(
                "java",
                &server_command_clone,
                server_args_clone,
                &project_root_clone,
            )
            .await;
        ovim::lsp_debug!("Java", "start_server returned: {:?}", result);
        result
    });

    // Poll for completion with progress updates
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3));
    let mut dots = 1;
    let start_result = loop {
        tokio::select! {
            result = &mut start_task => {
                break result;
            }
            _ = interval.tick() => {
                let dot_str = ".".repeat(dots);
                send_java_status(format!("Starting LSP server{}", dot_str));
                dots = (dots % 3) + 1;
            }
        }
    };

    match start_result {
        Ok(Ok(())) => {
            send_java_status("Server started successfully".to_string());
        }
        Ok(Err(e)) => {
            send_java_status(format!("Failed to start server: {}", e));
            return;
        }
        Err(e) => {
            send_java_status(format!("Server task failed: {}", e));
            return;
        }
    }

    send_java_status("Initializing LSP connection...".to_string());

    // Start notification listener
    lsp_manager
        .start_notification_listener("java".to_string())
        .await;

    // IMPORTANT: Don't send didOpen here - it will be handled by ensure_document_opened
    // when the editor actually needs to use LSP features. This avoids race conditions
    // and duplicate didOpen notifications.
    send_java_status("Ready ✓".to_string());
}

/// Old version that requires mutable editor (used in headless mode)
/// (Reserved for alternative Java LSP initialization path)
#[allow(dead_code)]
pub async fn initialize_java_lsp(editor: &mut Editor, file_path: &Path) {
    use ovim::java::{parser, JdtlsDownloader, JdtlsLauncher};

    // Find project root
    let project_root = find_jvm_project_root(file_path);

    editor.set_lsp_status("Java: Detecting project configuration...".to_string());

    // Detect Java version from build files
    let project_config = match parser::detect_java_version(project_root).await {
        Ok(config) => config,
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to detect version: {}", e));
            return;
        }
    };

    editor.set_lsp_status(format!(
        "Java: Detected Java {} project",
        project_config.java_version.as_str()
    ));

    // Get jdtls installation directory
    let jdtls_dir = match ovim::java::jdtls_dir().await {
        Ok(dir) => dir,
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to get cache dir: {}", e));
            return;
        }
    };

    // Ensure jdtls is installed
    let downloader = JdtlsDownloader::new(jdtls_dir.clone());

    if !downloader.is_installed().await {
        editor.set_lsp_status("Java: Downloading jdtls... (first time setup)".to_string());

        // Create a channel for async progress updates
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn download task
        let mut download_task = tokio::spawn(async move {
            downloader
                .ensure_installed(move |msg| {
                    let _ = progress_tx.send(msg);
                })
                .await
        });

        // Poll for progress updates without blocking
        loop {
            tokio::select! {
                Some(msg) = progress_rx.recv() => {
                    editor.set_lsp_status(format!("Java: {}", msg));
                }
                result = &mut download_task => {
                    match result {
                        Ok(Ok(())) => {
                            editor.set_lsp_status("Java: Download complete!".to_string());
                            break;
                        }
                        Ok(Err(e)) => {
                            editor.set_lsp_status(format!("Java: Download failed: {}", e));
                            return;
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("Java: Download task failed: {}", e));
                            return;
                        }
                    }
                }
            }
        }
    } else {
        editor.set_lsp_status("Java: Using cached jdtls".to_string());
    }

    // Ensure Lombok is installed
    let lombok_downloader = JdtlsDownloader::new(jdtls_dir.clone());
    if !lombok_downloader.is_lombok_installed().await {
        editor.set_lsp_status("Java: Downloading Lombok... (first time setup)".to_string());

        // Create a channel for async progress updates
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn download task
        let mut download_task = tokio::spawn(async move {
            lombok_downloader
                .ensure_lombok_installed(move |msg| {
                    let _ = progress_tx.send(msg);
                })
                .await
        });

        // Poll for progress updates without blocking
        loop {
            tokio::select! {
                Some(msg) = progress_rx.recv() => {
                    editor.set_lsp_status(format!("Java: {}", msg));
                }
                result = &mut download_task => {
                    match result {
                        Ok(Ok(())) => {
                            editor.set_lsp_status("Java: Lombok download complete!".to_string());
                            break;
                        }
                        Ok(Err(e)) => {
                            editor.set_lsp_status(format!("Java: Lombok download failed: {}", e));
                            // Non-fatal: continue without Lombok
                            break;
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("Java: Lombok download task failed: {}", e));
                            // Non-fatal: continue without Lombok
                            break;
                        }
                    }
                }
            }
        }
    } else {
        editor.set_lsp_status("Java: Using cached Lombok".to_string());
    }

    // Get Lombok JAR path (if installed)
    let lombok_downloader2 = JdtlsDownloader::new(jdtls_dir.clone());
    let lombok_jar = if lombok_downloader2.is_lombok_installed().await {
        Some(lombok_downloader2.lombok_jar_path())
    } else {
        None
    };

    // Get workspace directory
    let workspace_dir = match ovim::java::workspace_dir(project_root).await {
        Ok(dir) => dir,
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to create workspace: {}", e));
            return;
        }
    };

    editor.set_lsp_status("Java: Configuring launcher...".to_string());

    // Create launcher
    let launcher =
        JdtlsLauncher::from_project_config(project_config, jdtls_dir, workspace_dir, lombok_jar);

    editor.set_lsp_status("Java: Finding JVM...".to_string());

    // Get launch command (async JVM detection)
    let launch_args = match launcher.launch_command().await {
        Ok(args) => {
            editor.set_lsp_status("Java: JVM found, launching jdtls...".to_string());
            args
        }
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to find JVM: {}", e));
            return;
        }
    };

    // Start LSP server using the launch args
    if let Some(lsp_manager) = editor.lsp_manager() {
        // Extract java command and args
        if launch_args.is_empty() {
            editor.set_lsp_status("Java: Invalid launch configuration".to_string());
            return;
        }

        let server_command = &launch_args[0];
        let server_args: Vec<String> = launch_args[1..].to_vec();

        editor.set_lsp_status("Java: Starting LSP server...".to_string());

        match lsp_manager
            .start_server("java", server_command, server_args, project_root)
            .await
        {
            Ok(_) => {
                editor.register_lsp_server("java".to_string(), "jdtls".to_string());

                editor.set_lsp_status("Java: Initializing LSP connection...".to_string());

                lsp_manager
                    .start_notification_listener("java".to_string())
                    .await;

                // IMPORTANT: Don't send didOpen here - it will be handled by ensure_document_opened
                // when the editor actually needs to use LSP features. This avoids race conditions
                // and duplicate didOpen notifications.
                editor.set_lsp_status("Java: Ready ✓".to_string());
            }
            Err(e) => {
                editor.set_lsp_status(format!("Java: Failed to start server: {}", e));
            }
        }
    }
}
