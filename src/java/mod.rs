//! Java IDE toolchain management
//!
//! Provides automatic setup and configuration for Java development:
//! - Auto-downloads and installs jdtls
//! - Detects Java version from build.gradle/pom.xml
//! - Finds appropriate JVM (17, 21, etc.)
//! - Launches jdtls with optimal configuration
//! - Fully async and non-blocking

pub mod downloader;
pub mod launcher;
pub mod parser;

pub use downloader::JdtlsDownloader;
pub use launcher::{JdtlsConfig, JdtlsLauncher};
pub use parser::{JavaVersion, ProjectConfig};

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Get the cache directory for ovim Java tools (async version)
pub async fn cache_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("Could not determine home directory")?;

    let cache = PathBuf::from(home).join(".cache").join("ovim").join("java");
    tokio::fs::create_dir_all(&cache)
        .await
        .context("Failed to create cache directory")?;

    Ok(cache)
}

/// Get the jdtls installation directory (async version)
pub async fn jdtls_dir() -> Result<PathBuf> {
    Ok(cache_dir().await?.join("jdtls"))
}

/// Get the workspace data directory for a project (async version)
pub async fn workspace_dir(project_root: &Path) -> Result<PathBuf> {
    let project_name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let workspace = cache_dir().await?.join("workspaces").join(project_name);
    tokio::fs::create_dir_all(&workspace)
        .await
        .context("Failed to create workspace directory")?;

    Ok(workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_dir() {
        let dir = cache_dir().await.unwrap();
        assert!(dir.ends_with(".cache/ovim/java"));
    }

    #[tokio::test]
    async fn test_jdtls_dir() {
        let dir = jdtls_dir().await.unwrap();
        assert!(dir.ends_with(".cache/ovim/java/jdtls"));
    }
}
