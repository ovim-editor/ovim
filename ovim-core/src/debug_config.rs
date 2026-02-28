//! Debug run configuration loading.
//!
//! Reads `.ovim/debug.toml` for project-local debug launch/attach configs.
//! Optionally imports IntelliJ run configurations from `.idea/runConfigurations/`
//! and `.run/` directories.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A single debug run configuration.
#[derive(Debug, Clone)]
pub struct DebugRunConfig {
    /// Display name shown in the picker.
    pub name: String,
    /// What kind of debug session to start.
    pub kind: DebugRunKind,
}

/// The kind of debug session.
#[derive(Debug, Clone)]
pub enum DebugRunKind {
    /// Run a Gradle task with `--debug-jvm`, then attach.
    Gradle {
        task: String,
        args: Vec<String>,
        project_root: Option<String>,
    },
    /// Attach to an already-running JVM.
    Attach {
        host: String,
        port: u16,
        project_root: Option<String>,
    },
    /// Launch a JVM directly with a main class.
    Launch {
        main_class: String,
        classpath: Option<String>,
        args: Vec<String>,
        jvm_args: Vec<String>,
        cwd: Option<String>,
        project_root: Option<String>,
    },
}

// ---- TOML deserialization types ----

#[derive(Debug, Deserialize)]
struct DebugToml {
    #[serde(default)]
    config: Vec<RawConfig>,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    name: String,
    #[serde(rename = "type")]
    config_type: String,
    // Gradle fields
    #[serde(default)]
    task: Option<String>,
    // Attach fields
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    // Launch fields
    #[serde(default)]
    main_class: Option<String>,
    #[serde(default)]
    classpath: Option<String>,
    // Shared fields
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    jvm_args: Option<Vec<String>>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    project_root: Option<String>,
}

impl RawConfig {
    fn into_run_config(self) -> Option<DebugRunConfig> {
        let kind = match self.config_type.as_str() {
            "gradle" => DebugRunKind::Gradle {
                task: self.task?,
                args: self.args.unwrap_or_default(),
                project_root: self.project_root,
            },
            "attach" => DebugRunKind::Attach {
                host: self.host.unwrap_or_else(|| "127.0.0.1".to_owned()),
                port: self.port.unwrap_or(5005),
                project_root: self.project_root,
            },
            "launch" => DebugRunKind::Launch {
                main_class: self.main_class?,
                classpath: self.classpath,
                args: self.args.unwrap_or_default(),
                jvm_args: self.jvm_args.unwrap_or_default(),
                cwd: self.cwd,
                project_root: self.project_root,
            },
            _ => return None,
        };
        Some(DebugRunConfig {
            name: self.name,
            kind,
        })
    }
}

/// Load debug configurations from `.ovim/debug.toml` relative to `project_root`.
pub fn load_debug_configs(project_root: &Path) -> Vec<DebugRunConfig> {
    let config_path = project_root.join(".ovim").join("debug.toml");
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let toml: DebugToml = match toml::from_str(&content) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warning: failed to parse {}: {e}", config_path.display());
            return Vec::new();
        }
    };
    toml.config
        .into_iter()
        .filter_map(|c| c.into_run_config())
        .collect()
}

/// Import IntelliJ `GradleRunConfiguration` entries from `.idea/runConfigurations/` and `.run/`.
pub fn load_intellij_configs(project_root: &Path) -> Vec<DebugRunConfig> {
    let mut configs = Vec::new();
    let dirs = [
        project_root.join(".idea").join("runConfigurations"),
        project_root.join(".run"),
    ];
    for dir in &dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "xml") {
                continue;
            }
            if let Some(config) = parse_intellij_run_config(&path) {
                configs.push(config);
            }
        }
    }
    configs
}

/// Parse a single IntelliJ run configuration XML file.
/// Handles `GradleRunConfiguration` type only.
fn parse_intellij_run_config(path: &PathBuf) -> Option<DebugRunConfig> {
    let content = std::fs::read_to_string(path).ok()?;

    // Simple XML extraction — no full parser dependency.
    if !content.contains("GradleRunConfiguration") {
        return None;
    }

    let name = extract_xml_attr(&content, "configuration", "name")?;
    let task = extract_option_value(&content, "mainClass")
        .or_else(|| extract_option_value(&content, "taskNames"))?;

    Some(DebugRunConfig {
        name,
        kind: DebugRunKind::Gradle {
            task,
            args: Vec::new(),
            project_root: None,
        },
    })
}

/// Extract an attribute value from an XML element.
fn extract_xml_attr(xml: &str, element: &str, attr: &str) -> Option<String> {
    let pattern = format!("<{element} ");
    let start = xml.find(&pattern)?;
    let rest = &xml[start..];
    let attr_pattern = format!("{attr}=\"");
    let attr_start = rest.find(&attr_pattern)? + attr_pattern.len();
    let attr_end = rest[attr_start..].find('"')? + attr_start;
    Some(rest[attr_start..attr_end].to_owned())
}

/// Extract the value attribute from `<option name="key" value="val" />`.
fn extract_option_value(xml: &str, name: &str) -> Option<String> {
    let pattern = format!("name=\"{name}\"");
    let pos = xml.find(&pattern)?;
    let rest = &xml[pos..];
    let val_start = rest.find("value=\"")? + 7;
    let val_end = rest[val_start..].find('"')? + val_start;
    Some(rest[val_start..val_end].to_owned())
}
