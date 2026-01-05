// Auto-Install System for Language Servers
//
// Educational Note: Package Manager Integration
//
// This module handles automatic installation of language servers via package managers
// like npm, cargo, etc. The design principles here are:
//
// 1. User Consent First - Always prompt before installing anything
// 2. Graceful Degradation - If auto-install fails, show manual instructions
// 3. Network/Permission Resilience - Handle common failure modes with helpful messages
// 4. Progress Feedback - Users should know what's happening during long installs
//
// Why this matters:
// - Installing software is potentially dangerous (security, disk space, permissions)
// - Users should be in control, not surprised by automatic actions
// - Error messages should be actionable, not just "it failed"
//
// The pattern: Try → Fail gracefully → Guide user to success

use ovim::language_config::{AutoInstallConfig, InstallMethod};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tokio::process::Command as TokioCommand;

/// Result of an auto-install attempt
#[derive(Debug)]
pub enum InstallResult {
    /// Installation succeeded, LSP server now available at this path
    Success(PathBuf),

    /// Installation failed with a user-facing error message
    Failed(String),

    /// Prerequisites not met (e.g., npm not installed)
    PrerequisitesMissing(String),

    /// User declined installation
    #[allow(dead_code)]
    Declined,
}

/// Attempt to auto-install a language server
///
/// Educational Note: Async for Network Operations
/// This function is async because package installations often involve:
/// - Network requests (downloading packages)
/// - Long-running processes (building from source)
/// - Multiple steps that could fail independently
///
/// Making it async allows the editor to remain responsive during installation.
pub async fn attempt_auto_install(
    language_name: &str,
    _package_name: &str,
    config: &AutoInstallConfig,
) -> InstallResult {
    match &config.method {
        InstallMethod::Npm { package, global } => {
            install_via_npm(language_name, package, *global).await
        }
        InstallMethod::Cargo { package } => {
            install_via_cargo(language_name, package).await
        }
        InstallMethod::Github {
            repo,
            asset_pattern,
            install_path,
        } => {
            install_via_github(language_name, repo, asset_pattern, install_path).await
        }
        InstallMethod::Shell { command } => {
            install_via_shell(language_name, command).await
        }
    }
}

/// Install language server via npm
///
/// Educational Note: Error Handling Strategy
/// npm can fail in many ways:
/// 1. npm not installed → PrerequisitesMissing
/// 2. Network failure → Failed with retry suggestion
/// 3. Permission denied → Failed with permission fix suggestion
/// 4. Package not found → Failed with package name check
///
/// Each failure mode gets a specific, actionable error message.
async fn install_via_npm(_language_name: &str, package: &str, global: bool) -> InstallResult {
    // Step 1: Check if npm is available
    let npm_check = Command::new("npm").arg("--version").output();

    if npm_check.is_err() || !npm_check.unwrap().status.success() {
        return InstallResult::PrerequisitesMissing(
            "npm not found. Install Node.js first:\n  \
             - macOS: brew install node\n  \
             - Linux: sudo apt install nodejs npm\n  \
             - Windows: Download from https://nodejs.org"
                .to_string(),
        );
    }

    // Step 2: Construct npm install command
    let mut args = vec!["install"];
    if global {
        args.push("-g");
    }
    args.push(package);

    ovim::lsp_info!(
        "AutoInstall",
        "Installing {} via npm: npm {}",
        package,
        args.join(" ")
    );

    // Step 3: Run npm install with output streaming
    let child = match TokioCommand::new("npm")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return InstallResult::Failed(format!("Failed to spawn npm process: {}", e));
        }
    };

    // Step 4: Wait for completion
    let output = match child.wait_with_output().await {
        Ok(output) => output,
        Err(e) => {
            return InstallResult::Failed(format!("npm install process failed: {}", e));
        }
    };

    // Step 5: Check exit status and parse errors
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Parse common npm error patterns
        if stderr.contains("EACCES") || stderr.contains("permission denied") {
            return InstallResult::Failed(format!(
                "Permission denied. Try one of these:\n  \
                 1. Run with sudo: sudo npm install -g {}\n  \
                 2. Configure npm to use local directory:\n     \
                 mkdir -p ~/.npm-global && npm config set prefix ~/.npm-global\n     \
                 Then add to PATH: export PATH=~/.npm-global/bin:$PATH\n  \
                 3. Use a version manager like nvm",
                package
            ));
        }

        if stderr.contains("ENOTFOUND") || stderr.contains("ETIMEDOUT") {
            return InstallResult::Failed(
                "Network error. Check internet connection and try again.".to_string(),
            );
        }

        if stderr.contains("404") || stderr.contains("not found") {
            return InstallResult::Failed(format!(
                "Package '{}' not found in npm registry. Check package name.",
                package
            ));
        }

        // Generic failure with stderr output
        return InstallResult::Failed(format!(
            "npm install failed:\n{}",
            stderr.lines().take(10).collect::<Vec<_>>().join("\n")
        ));
    }

    // Step 6: Verify installation succeeded
    let install_path = verify_npm_installation(package, global).await;

    match install_path {
        Some(path) => {
            ovim::lsp_info!(
                "AutoInstall",
                "Successfully installed {} at {}",
                package,
                path.display()
            );
            InstallResult::Success(path)
        }
        None => InstallResult::Failed(format!(
            "Installation appeared to succeed, but {} was not found in PATH. \
             You may need to restart your shell or update PATH manually.",
            package
        )),
    }
}

/// Verify npm package installation by finding the binary
///
/// Educational Note: PATH Resolution
/// After `npm install -g`, the binary should be in PATH. But there are edge cases:
/// - npm might install to a directory not in PATH
/// - Shell hasn't refreshed PATH yet
/// - User's npm prefix is misconfigured
///
/// We check common locations as fallback.
async fn verify_npm_installation(package: &str, global: bool) -> Option<PathBuf> {
    // Try `which <package>` first (checks PATH)
    if let Ok(output) = Command::new("which").arg(package).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    // Fallback: Check common npm global install locations
    if global {
        let candidates = vec![
            dirs::home_dir().map(|h| h.join(".npm-global/bin").join(package)),
            dirs::home_dir().map(|h| h.join(".nvm/current/bin").join(package)),
            Some(PathBuf::from(format!("/usr/local/bin/{}", package))),
            Some(PathBuf::from(format!("/opt/homebrew/bin/{}", package))),
        ];

        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Install via cargo (Rust's package manager)
async fn install_via_cargo(_language_name: &str, package: &str) -> InstallResult {
    // Check if cargo is available
    let cargo_check = Command::new("cargo").arg("--version").output();

    if cargo_check.is_err() || !cargo_check.unwrap().status.success() {
        return InstallResult::PrerequisitesMissing(
            "cargo not found. Install Rust from https://rustup.rs".to_string(),
        );
    }

    ovim::lsp_info!("AutoInstall", "Installing {} via cargo install", package);

    // Run cargo install
    let child = match TokioCommand::new("cargo")
        .args(&["install", package])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return InstallResult::Failed(format!("Failed to spawn cargo process: {}", e));
        }
    };

    let output = match child.wait_with_output().await {
        Ok(output) => output,
        Err(e) => {
            return InstallResult::Failed(format!("cargo install failed: {}", e));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return InstallResult::Failed(format!(
            "cargo install failed:\n{}",
            stderr.lines().take(10).collect::<Vec<_>>().join("\n")
        ));
    }

    // Verify installation
    if let Ok(output) = Command::new("which").arg(package).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return InstallResult::Success(PathBuf::from(path));
        }
    }

    InstallResult::Failed(format!(
        "Installation appeared to succeed, but {} was not found in PATH",
        package
    ))
}

/// Install via GitHub release (download binary)
async fn install_via_github(
    _language_name: &str,
    _repo: &str,
    _asset_pattern: &str,
    _install_path: &str,
) -> InstallResult {
    // TODO: Implement GitHub release download
    // This is more complex - needs:
    // 1. Detect OS/arch
    // 2. Fetch GitHub API for latest release
    // 3. Find matching asset (using pattern)
    // 4. Download and extract
    // 5. Make executable and move to install_path
    InstallResult::Failed(
        "GitHub release installation not yet implemented. Install manually.".to_string(),
    )
}

/// Install via custom shell command
async fn install_via_shell(_language_name: &str, command: &str) -> InstallResult {
    ovim::lsp_info!("AutoInstall", "Running custom install command: {}", command);

    // Parse command (simple split on spaces - doesn't handle quotes)
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return InstallResult::Failed("Empty install command".to_string());
    }

    let child = match TokioCommand::new(parts[0])
        .args(&parts[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return InstallResult::Failed(format!("Failed to run install command: {}", e));
        }
    };

    let output = match child.wait_with_output().await {
        Ok(output) => output,
        Err(e) => {
            return InstallResult::Failed(format!("Install command failed: {}", e));
        }
    };

    if output.status.success() {
        InstallResult::Success(PathBuf::from("(custom install succeeded)"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        InstallResult::Failed(format!(
            "Install command failed:\n{}",
            stderr.lines().take(10).collect::<Vec<_>>().join("\n")
        ))
    }
}

#[cfg(test)]
#[allow(clippy::print_stderr)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_verify_npm_installation_with_which() {
        // This test assumes `node` is installed (which is likely if npm is installed)
        // Note: This is a brittle test - depends on system state
        let result = verify_npm_installation("node", true).await;
        // We can't assert it's Some because CI might not have node
        // Just ensure it doesn't panic
        eprintln!("node path: {:?}", result);
    }

    #[test]
    fn test_install_result_display() {
        let success = InstallResult::Success(PathBuf::from("/usr/bin/test"));
        assert!(matches!(success, InstallResult::Success(_)));

        let failed = InstallResult::Failed("test error".to_string());
        assert!(matches!(failed, InstallResult::Failed(_)));
    }
}
