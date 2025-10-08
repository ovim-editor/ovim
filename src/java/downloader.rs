//! Auto-downloader for Eclipse JDT.LS
//!
//! Downloads and extracts jdtls from Eclipse releases
//! Caches installation for fast startup

use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

// Use latest milestone release (more stable than snapshots)
const JDTLS_VERSION: &str = "1.38.0";
const JDTLS_MILESTONE_DATE: &str = "202408011337";

/// Auto-downloader for jdtls
pub struct JdtlsDownloader {
    install_dir: PathBuf,
}

impl JdtlsDownloader {
    /// Create a new downloader
    pub fn new(install_dir: PathBuf) -> Self {
        Self { install_dir }
    }

    /// Check if jdtls is installed (async)
    pub async fn is_installed(&self) -> bool {
        let launcher = self.launcher_jar();
        // Use async metadata check instead of blocking exists() and is_file()
        match tokio::fs::metadata(&launcher).await {
            Ok(metadata) => metadata.is_file(),
            Err(_) => false,
        }
    }

    /// Get the path to the launcher JAR
    pub fn launcher_jar(&self) -> PathBuf {
        self.install_dir
            .join("plugins")
            .join(format!(
                "org.eclipse.equinox.launcher_*.jar"
            ))
    }

    /// Find the actual launcher JAR (glob pattern)
    pub async fn find_launcher_jar(&self) -> Result<PathBuf> {
        let plugins_dir = self.install_dir.join("plugins");

        let mut entries = tokio::fs::read_dir(&plugins_dir)
            .await
            .context("Failed to read plugins directory")?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("org.eclipse.equinox.launcher_")
                    && name.ends_with(".jar")
                    && !name.contains("source")
                {
                    return Ok(path);
                }
            }
        }

        anyhow::bail!("Launcher JAR not found in {}", plugins_dir.display())
    }

    /// Download and install jdtls
    pub async fn download(&self, progress_callback: impl Fn(String)) -> Result<()> {
        progress_callback("Downloading jdtls...".to_string());

        // Create install directory
        tokio::fs::create_dir_all(&self.install_dir)
            .await
            .context("Failed to create install directory")?;

        // Try multiple download URLs in order of preference
        let urls = vec![
            // Latest snapshot (may not exist)
            "https://download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz".to_string(),
            // Milestone release (stable)
            format!(
                "https://download.eclipse.org/jdtls/milestones/{}/jdt-language-server-{}.tar.gz",
                JDTLS_VERSION, JDTLS_MILESTONE_DATE
            ),
            // Alternative: specific milestone
            "https://download.eclipse.org/jdtls/milestones/1.38.0/jdt-language-server-1.38.0-202408011337.tar.gz".to_string(),
        ];

        let mut last_error = None;

        for (attempt, url) in urls.iter().enumerate() {
            progress_callback(format!("Attempt {}/{}: {}", attempt + 1, urls.len(), url));

            // Try to download from this URL
            let response = match reqwest::get(url).await {
                Ok(resp) => resp,
                Err(e) => {
                    last_error = Some(format!("Connection failed: {}", e));
                    continue;
                }
            };

            if !response.status().is_success() {
                last_error = Some(format!("HTTP {}", response.status()));
                continue;
            }

            let bytes = match response.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    last_error = Some(format!("Failed to read response: {}", e));
                    continue;
                }
            };

            progress_callback(format!("Downloaded {} bytes", bytes.len()));

            // Save to temp file
            let temp_path = self.install_dir.join("jdtls.tar.gz");
            let mut file = match tokio::fs::File::create(&temp_path).await {
                Ok(f) => f,
                Err(e) => {
                    last_error = Some(format!("Failed to create temp file: {}", e));
                    continue;
                }
            };

            if let Err(e) = file.write_all(&bytes).await {
                last_error = Some(format!("Failed to write temp file: {}", e));
                continue;
            }

            progress_callback("Extracting jdtls...".to_string());

            // Extract using tar command (async)
            // Note: This can take 10-30 seconds for ~98MB archive
            // Spawn tar process
            let mut child = match tokio::process::Command::new("tar")
                .arg("xzf")
                .arg(&temp_path)
                .arg("-C")
                .arg(&self.install_dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(e) => {
                    last_error = Some(format!("Failed to spawn tar: {}", e));
                    progress_callback(format!("Extraction error: {}", e));
                    continue;
                }
            };

            // Wait for extraction with periodic progress updates and timeout (10 seconds)
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
            let mut dots = 1;
            const EXTRACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

            let extract_result = tokio::time::timeout(EXTRACT_TIMEOUT, async {
                loop {
                    tokio::select! {
                        result = child.wait() => {
                            return result;
                        }
                        _ = interval.tick() => {
                            let dot_str = ".".repeat(dots);
                            progress_callback(format!("Extracting jdtls{}", dot_str));
                            dots = (dots % 3) + 1; // Cycle through 1, 2, 3 dots
                        }
                    }
                }
            }).await;

            let extract_result = match extract_result {
                Ok(result) => result,
                Err(_) => {
                    // Timeout - kill the tar process
                    let _ = child.kill().await;
                    last_error = Some("Tar extraction timed out after 10 seconds".to_string());
                    progress_callback("Extraction timeout - server may be slow or file corrupt".to_string());
                    continue;
                }
            };

            let status = match extract_result {
                Ok(status) => status,
                Err(e) => {
                    last_error = Some(format!("Failed to run tar: {}", e));
                    progress_callback(format!("Extraction error: {}", e));
                    continue;
                }
            };

            if !status.success() {
                // Try to read stderr if available
                let stderr_msg = if let Some(mut stderr) = child.stderr.take() {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    if stderr.read_to_end(&mut buf).await.is_ok() {
                        String::from_utf8_lossy(&buf).to_string()
                    } else {
                        "Unknown error".to_string()
                    }
                } else {
                    "Unknown error".to_string()
                };
                last_error = Some(format!("Extraction failed: {}", stderr_msg));
                progress_callback(format!("Extraction failed: {}", stderr_msg));
                continue;
            }

            progress_callback("Extraction complete, verifying...".to_string());

            // Verify extraction succeeded by checking for launcher JAR
            if let Err(e) = self.find_launcher_jar().await {
                last_error = Some(format!("Launcher JAR not found after extraction: {}", e));
                progress_callback(format!("Verification failed: {}", e));
                continue;
            }

            // Clean up temp file
            let _ = tokio::fs::remove_file(&temp_path).await;

            progress_callback("jdtls installed successfully!".to_string());

            return Ok(());
        }

        // All URLs failed
        anyhow::bail!(
            "Failed to download jdtls from all sources. Last error: {}",
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        )
    }

    /// Ensure jdtls is installed (download if needed)
    pub async fn ensure_installed(
        &self,
        progress_callback: impl Fn(String),
    ) -> Result<()> {
        if self.is_installed().await {
            return Ok(());
        }

        self.download(progress_callback).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downloader_creation() {
        let dir = PathBuf::from("/tmp/test_jdtls");
        let downloader = JdtlsDownloader::new(dir.clone());
        assert_eq!(downloader.install_dir, dir);
    }
}
