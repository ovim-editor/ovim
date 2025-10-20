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
    /// Path to Lombok JAR (optional)
    pub lombok_jar: Option<PathBuf>,
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

    /// Find Java executable for jdtls (requires Java 21+)
    /// Note: jdtls itself requires Java 21+ to run, even if the project uses an older version
    pub async fn find_java(&self) -> Result<PathBuf> {
        // jdtls requires Java 21+ minimum (as of the latest versions downloaded)
        // We MUST find Java 21+, ignoring JAVA_HOME/PATH which might be older versions
        let java_cmd = if cfg!(windows) { "java.exe" } else { "java" };

        // Try common installation locations
        let common_paths = if cfg!(target_os = "macos") {
            vec![
                "/Library/Java/JavaVirtualMachines/",
                "/System/Library/Java/JavaVirtualMachines/",
            ]
        } else if cfg!(target_os = "linux") {
            vec!["/usr/lib/jvm/", "/usr/java/", "/opt/java/"]
        } else {
            vec![] // Windows uses registry
        };

        // Search common paths for Java 21+
        for base_path in common_paths {
            if let Ok(mut entries) = tokio::fs::read_dir(base_path).await {
                let mut candidates = vec![];
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let java_bin = entry.path().join("Contents/Home/bin").join(java_cmd);
                    // Use async metadata check instead of blocking exists()
                    if tokio::fs::metadata(&java_bin).await.is_ok() {
                        candidates.push(java_bin);
                    }
                }

                // Sort candidates by version (prefer higher versions)
                // Try Java 24, then 21, then others
                for version in [24, 21, 23, 22] {
                    for candidate in &candidates {
                        if let Ok(ver) = self.check_java_version_number(candidate).await {
                            if ver >= 21 && ver == version {
                                return Ok(candidate.clone());
                            }
                        }
                    }
                }

                // If no exact match, return any Java 21+
                for candidate in &candidates {
                    if let Ok(ver) = self.check_java_version_number(candidate).await {
                        if ver >= 21 {
                            return Ok(candidate.clone());
                        }
                    }
                }
            }
        }

        anyhow::bail!(
            "Could not find Java 21 or higher (required for jdtls). Please install Java 21+ and set JAVA_HOME."
        )
    }

    /// Get Java version number from executable
    async fn check_java_version_number(&self, java_path: &Path) -> Result<u32> {
        let output = Command::new(java_path)
            .arg("-version")
            .stderr(Stdio::piped())
            .output()
            .await?;

        let version_output = String::from_utf8_lossy(&output.stderr);

        // Parse version from output like: openjdk version "21.0.2"
        for line in version_output.lines() {
            if line.contains("version") {
                if let Some(start) = line.find('"') {
                    if let Some(end) = line[start + 1..].find('"') {
                        let version_str = &line[start + 1..start + 1 + end];
                        // Version format: "21.0.2" or "1.8.0_292"
                        let major = if version_str.starts_with("1.") {
                            // Old format: 1.8.x means Java 8
                            version_str.split('.').nth(1)
                        } else {
                            // New format: 21.x.x means Java 21
                            version_str.split('.').next()
                        };

                        if let Some(major_str) = major {
                            if let Ok(major_num) = major_str.parse::<u32>() {
                                return Ok(major_num);
                            }
                        }
                    }
                }
            }
        }

        anyhow::bail!("Could not parse Java version")
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
            java_bin.to_string_lossy().to_string(),
            // Memory settings (optimized for speed)
            "-Xms256m".to_string(),
            "-Xmx2G".to_string(),
            // Performance flags
            "-XX:+UseG1GC".to_string(),
            "-XX:+UseStringDeduplication".to_string(),
        ];

        // Add Lombok javaagent if available
        if let Some(lombok_jar) = &self.config.lombok_jar {
            args.push(format!("-javaagent:{}", lombok_jar.to_string_lossy()));
        }

        // JDT.LS JAR
        args.extend(vec![
            "-jar".to_string(),
            jdtls_launcher.to_string_lossy().to_string(),
            // Configuration
            "-configuration".to_string(),
            config_dir.to_string_lossy().to_string(),
            // Workspace data
            "-data".to_string(),
            self.config.workspace_dir.to_string_lossy().to_string(),
        ]);

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
        lombok_jar: Option<PathBuf>,
    ) -> Self {
        let config = JdtlsConfig {
            jdtls_home,
            project_root: project_config.root,
            workspace_dir,
            java_version: project_config.java_version,
            java_home: None,
            lombok_jar,
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
            lombok_jar: None,
        };

        let launcher = JdtlsLauncher::new(config);
        assert_eq!(launcher.config.java_version, JavaVersion::Java17);
    }
}
