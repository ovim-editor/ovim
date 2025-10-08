//! jdtls launcher with dynamic JVM detection and configuration
//!
//! Finds the appropriate JVM for the project's Java version
//! Launches jdtls with optimal memory and performance settings

use super::parser::{JavaVersion, ProjectConfig};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// Configuration for launching jdtls
#[derive(Debug, Clone)]
pub struct JdtlsConfig {
    /// Path to jdtls installation
    pub jdtls_home: PathBuf,
    /// Project root directory
    pub project_root: PathBuf,
    /// Workspace data directory
    pub workspace_dir: PathBuf,
    /// Java version for the project
    pub java_version: JavaVersion,
    /// Path to Java executable
    pub java_home: Option<PathBuf>,
}

/// jdtls launcher
pub struct JdtlsLauncher {
    config: JdtlsConfig,
}

impl JdtlsLauncher {
    /// Create a new launcher
    pub fn new(config: JdtlsConfig) -> Self {
        Self { config }
    }

    /// Find Java executable for the required version
    pub async fn find_java(&self) -> Result<PathBuf> {
        // First, check JAVA_HOME
        if let Some(java_home) = &self.config.java_home {
            return Ok(java_home.join("bin").join("java"));
        }

        // Try environment JAVA_HOME
        if let Ok(java_home) = std::env::var("JAVA_HOME") {
            let java_bin = PathBuf::from(java_home).join("bin").join("java");
            // Use async metadata check instead of blocking exists()
            if tokio::fs::metadata(&java_bin).await.is_ok() {
                return Ok(java_bin);
            }
        }

        // Try to find java in PATH
        let java_cmd = if cfg!(windows) { "java.exe" } else { "java" };

        // Use `which` or `where` to find java
        let which_cmd = if cfg!(windows) { "where" } else { "which" };

        let output = Command::new(which_cmd)
            .arg(java_cmd)
            .output()
            .await
            .context("Failed to find java in PATH")?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let path = path.trim();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }

        // Try common installation locations
        let common_paths = if cfg!(target_os = "macos") {
            vec![
                "/Library/Java/JavaVirtualMachines/",
                "/System/Library/Java/JavaVirtualMachines/",
            ]
        } else if cfg!(target_os = "linux") {
            vec![
                "/usr/lib/jvm/",
                "/usr/java/",
                "/opt/java/",
            ]
        } else {
            vec![] // Windows uses registry
        };

        for base_path in common_paths {
            if let Ok(mut entries) = tokio::fs::read_dir(base_path).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let java_bin = entry.path().join("bin").join(java_cmd);
                    // Use async metadata check instead of blocking exists()
                    if tokio::fs::metadata(&java_bin).await.is_ok() {
                        // Check if this Java version is compatible
                        if self.check_java_version(&java_bin).await.is_ok() {
                            return Ok(java_bin);
                        }
                    }
                }
            }
        }

        anyhow::bail!(
            "Could not find Java {} or higher. Please install Java and set JAVA_HOME.",
            self.config.java_version.min_jvm_version()
        )
    }

    /// Check if a Java executable meets version requirements
    async fn check_java_version(&self, java_path: &Path) -> Result<()> {
        let output = Command::new(java_path)
            .arg("-version")
            .stderr(Stdio::piped())
            .output()
            .await?;

        let version_output = String::from_utf8_lossy(&output.stderr);

        // Parse version from output like: openjdk version "17.0.2"
        let required = self.config.java_version.min_jvm_version();

        if version_output.contains(required) || version_output.contains(&format!("\"{}.", required)) {
            Ok(())
        } else {
            anyhow::bail!("Java version mismatch")
        }
    }

    /// Launch jdtls and return command args
    pub async fn launch_command(&self) -> Result<Vec<String>> {
        let java_bin = self.find_java().await?;

        // Find launcher JAR
        let jdtls_launcher = self.find_launcher_jar().await?;

        // Configuration directory
        let config_dir = if cfg!(target_os = "macos") {
            self.config.jdtls_home.join("config_mac")
        } else if cfg!(target_os = "linux") {
            self.config.jdtls_home.join("config_linux")
        } else {
            self.config.jdtls_home.join("config_win")
        };

        // Build command
        let mut args = vec![
            java_bin.to_str().unwrap().to_string(),

            // Memory settings (optimized for speed)
            "-Xms256m".to_string(),
            "-Xmx2G".to_string(),

            // Performance flags
            "-XX:+UseG1GC".to_string(),
            "-XX:+UseStringDeduplication".to_string(),

            // JDT.LS JAR
            "-jar".to_string(),
            jdtls_launcher.to_str().unwrap().to_string(),

            // Configuration
            "-configuration".to_string(),
            config_dir.to_str().unwrap().to_string(),

            // Workspace data
            "-data".to_string(),
            self.config.workspace_dir.to_str().unwrap().to_string(),
        ];

        Ok(args)
    }

    /// Find the launcher JAR in jdtls plugins directory
    async fn find_launcher_jar(&self) -> Result<PathBuf> {
        let plugins_dir = self.config.jdtls_home.join("plugins");

        let mut entries = tokio::fs::read_dir(&plugins_dir)
            .await
            .context("Failed to read jdtls plugins directory")?;

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

        anyhow::bail!("jdtls launcher JAR not found")
    }

    /// Create launcher from project config
    pub fn from_project_config(
        project_config: ProjectConfig,
        jdtls_home: PathBuf,
        workspace_dir: PathBuf,
    ) -> Self {
        let config = JdtlsConfig {
            jdtls_home,
            project_root: project_config.root,
            workspace_dir,
            java_version: project_config.java_version,
            java_home: None,
        };

        Self::new(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_launcher_creation() {
        let config = JdtlsConfig {
            jdtls_home: PathBuf::from("/test/jdtls"),
            project_root: PathBuf::from("/test/project"),
            workspace_dir: PathBuf::from("/test/workspace"),
            java_version: JavaVersion::Java17,
            java_home: None,
        };

        let launcher = JdtlsLauncher::new(config);
        assert_eq!(launcher.config.java_version, JavaVersion::Java17);
    }
}
