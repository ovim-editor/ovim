//! Build file parser for Java version detection
//!
//! Parses build.gradle, build.gradle.kts, and pom.xml to detect:
//! - Java source compatibility version
//! - Java target compatibility version
//! - Toolchain version

use anyhow::{Context, Result};
use regex::Regex;
use std::path::Path;

/// Java version detected from project configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JavaVersion {
    Java8,
    Java11,
    Java17,
    Java21,
    Java24,
}

impl JavaVersion {
    /// Get the version number as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            JavaVersion::Java8 => "8",
            JavaVersion::Java11 => "11",
            JavaVersion::Java17 => "17",
            JavaVersion::Java21 => "21",
            JavaVersion::Java24 => "24",
        }
    }

    /// Parse from version number
    pub fn from_number(version: u32) -> Option<Self> {
        match version {
            8 | 1_8 => Some(JavaVersion::Java8),
            11 | 1_11 => Some(JavaVersion::Java11),
            17 => Some(JavaVersion::Java17),
            21 => Some(JavaVersion::Java21),
            24 => Some(JavaVersion::Java24),
            _ => None,
        }
    }

    /// Get minimum JVM version required to run this code
    pub fn min_jvm_version(&self) -> &'static str {
        match self {
            JavaVersion::Java8 => "1.8",
            JavaVersion::Java11 => "11",
            JavaVersion::Java17 => "17",
            JavaVersion::Java21 => "21",
            JavaVersion::Java24 => "21", // Use Java 21 JVM for Java 24 code
        }
    }
}

/// Project configuration extracted from build files
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// Java version for source code
    pub java_version: JavaVersion,
    /// Project root directory
    pub root: std::path::PathBuf,
    /// Build system (gradle or maven)
    pub build_system: BuildSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSystem {
    Gradle,
    Maven,
    Unknown,
}

/// Detect Java version from project root
pub async fn detect_java_version(project_root: &Path) -> Result<ProjectConfig> {
    // Try Gradle first (build.gradle or build.gradle.kts)
    if let Ok(version) = parse_gradle(project_root).await {
        return Ok(ProjectConfig {
            java_version: version,
            root: project_root.to_path_buf(),
            build_system: BuildSystem::Gradle,
        });
    }

    // Try Maven (pom.xml)
    if let Ok(version) = parse_maven(project_root).await {
        return Ok(ProjectConfig {
            java_version: version,
            root: project_root.to_path_buf(),
            build_system: BuildSystem::Maven,
        });
    }

    // Default to Java 17 (LTS)
    Ok(ProjectConfig {
        java_version: JavaVersion::Java17,
        root: project_root.to_path_buf(),
        build_system: BuildSystem::Unknown,
    })
}

/// Parse build.gradle or build.gradle.kts for Java version
async fn parse_gradle(project_root: &Path) -> Result<JavaVersion> {
    // Try build.gradle.kts first (Kotlin DSL)
    let gradle_kts = project_root.join("build.gradle.kts");
    if tokio::fs::try_exists(&gradle_kts).await.unwrap_or(false) {
        if let Ok(version) = parse_gradle_file(&gradle_kts).await {
            return Ok(version);
        }
    }

    // Try build.gradle (Groovy DSL)
    let gradle_groovy = project_root.join("build.gradle");
    if tokio::fs::try_exists(&gradle_groovy).await.unwrap_or(false) {
        if let Ok(version) = parse_gradle_file(&gradle_groovy).await {
            return Ok(version);
        }
    }

    anyhow::bail!("No build.gradle file found")
}

/// Parse a Gradle build file
async fn parse_gradle_file(path: &Path) -> Result<JavaVersion> {
    let content = tokio::fs::read_to_string(path)
        .await
        .context("Failed to read build.gradle")?;

    // Patterns to match Java version in Gradle files
    let patterns = vec![
        // toolchain { languageVersion = JavaLanguageVersion.of(17) }
        Regex::new(r"languageVersion\s*=\s*JavaLanguageVersion\.of\((\d+)\)").unwrap(),
        // toolchain.languageVersion.set(JavaLanguageVersion.of(17))
        Regex::new(r"toolchain\.languageVersion\.set\(JavaLanguageVersion\.of\((\d+)\)\)").unwrap(),
        // sourceCompatibility = '17'
        Regex::new(r#"sourceCompatibility\s*=\s*['"]?(\d+)['"]?"#).unwrap(),
        // targetCompatibility = '17'
        Regex::new(r#"targetCompatibility\s*=\s*['"]?(\d+)['"]?"#).unwrap(),
        // sourceCompatibility = JavaVersion.VERSION_17
        Regex::new(r"sourceCompatibility\s*=\s*JavaVersion\.VERSION_(\d+)").unwrap(),
        // jvmTarget = "17"
        Regex::new(r#"jvmTarget\s*=\s*"(\d+)""#).unwrap(),
    ];

    for pattern in patterns {
        if let Some(captures) = pattern.captures(&content) {
            if let Some(version_str) = captures.get(1) {
                if let Ok(version_num) = version_str.as_str().parse::<u32>() {
                    if let Some(version) = JavaVersion::from_number(version_num) {
                        return Ok(version);
                    }
                }
            }
        }
    }

    // Default to Java 17 if not found
    Ok(JavaVersion::Java17)
}

/// Parse pom.xml for Java version
async fn parse_maven(project_root: &Path) -> Result<JavaVersion> {
    let pom_path = project_root.join("pom.xml");
    if !tokio::fs::try_exists(&pom_path).await.unwrap_or(false) {
        anyhow::bail!("No pom.xml found");
    }

    let content = tokio::fs::read_to_string(&pom_path)
        .await
        .context("Failed to read pom.xml")?;

    // Patterns to match Java version in pom.xml
    let patterns = vec![
        // <maven.compiler.source>17</maven.compiler.source>
        Regex::new(r"<maven\.compiler\.source>(\d+)</maven\.compiler\.source>").unwrap(),
        // <maven.compiler.target>17</maven.compiler.target>
        Regex::new(r"<maven\.compiler\.target>(\d+)</maven\.compiler\.target>").unwrap(),
        // <java.version>17</java.version>
        Regex::new(r"<java\.version>(\d+)</java\.version>").unwrap(),
        // <release>17</release> (in maven-compiler-plugin)
        Regex::new(r"<release>(\d+)</release>").unwrap(),
    ];

    for pattern in patterns {
        if let Some(captures) = pattern.captures(&content) {
            if let Some(version_str) = captures.get(1) {
                if let Ok(version_num) = version_str.as_str().parse::<u32>() {
                    if let Some(version) = JavaVersion::from_number(version_num) {
                        return Ok(version);
                    }
                }
            }
        }
    }

    // Default to Java 17 if not found
    Ok(JavaVersion::Java17)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_java_version_from_number() {
        assert_eq!(JavaVersion::from_number(8), Some(JavaVersion::Java8));
        assert_eq!(JavaVersion::from_number(11), Some(JavaVersion::Java11));
        assert_eq!(JavaVersion::from_number(17), Some(JavaVersion::Java17));
        assert_eq!(JavaVersion::from_number(21), Some(JavaVersion::Java21));
        assert_eq!(JavaVersion::from_number(99), None);
    }

    #[test]
    fn test_java_version_min_jvm() {
        assert_eq!(JavaVersion::Java8.min_jvm_version(), "1.8");
        assert_eq!(JavaVersion::Java17.min_jvm_version(), "17");
        assert_eq!(JavaVersion::Java21.min_jvm_version(), "21");
    }
}
