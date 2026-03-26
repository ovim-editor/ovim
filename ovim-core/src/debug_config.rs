//! Debug run configuration loading.
//!
//! Reads `.ovim/debug.toml` for project-local debug launch/attach configs.
//! Optionally imports IntelliJ run configurations from `.idea/runConfigurations/`
//! and `.run/` directories.

use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

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

/// Convert launch.json-shaped JSON values (from hyperion LSP `executeCommand`) into
/// `DebugRunConfig` entries.
///
/// Each value is expected to have `name`, `type`, and `request` fields. The `request`
/// field determines the `DebugRunKind`:
/// - `"attach"` → `DebugRunKind::Attach`
/// - `"launch"` → `DebugRunKind::Launch`
/// - `"gradle"` → `DebugRunKind::Gradle`
pub fn parse_lsp_run_configs(values: &[Value]) -> Vec<DebugRunConfig> {
    values.iter().filter_map(parse_single_lsp_config).collect()
}

fn parse_single_lsp_config(value: &Value) -> Option<DebugRunConfig> {
    let name = value.get("name")?.as_str()?.to_owned();
    let request = value.get("request")?.as_str()?;
    let kind = match request {
        "attach" => DebugRunKind::Attach {
            host: value
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_owned(),
            port: value.get("port").and_then(|v| v.as_u64()).unwrap_or(5005) as u16,
            project_root: value
                .get("projectRoot")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        "launch" => DebugRunKind::Launch {
            main_class: value.get("mainClass")?.as_str()?.to_owned(),
            classpath: value
                .get("classpath")
                .and_then(|v| v.as_str())
                .map(String::from),
            args: json_string_array(value.get("args")),
            jvm_args: json_string_array(value.get("jvmArgs")),
            cwd: value.get("cwd").and_then(|v| v.as_str()).map(String::from),
            project_root: value
                .get("projectRoot")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        "gradle" => DebugRunKind::Gradle {
            task: value
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned(),
            args: json_string_array(value.get("args")),
            project_root: value
                .get("projectPath")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        _ => return None,
    };
    Some(DebugRunConfig { name, kind })
}

/// Extract a `Vec<String>` from a JSON array of strings, defaulting to empty.
fn json_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_lsp_attach_config() {
        let configs = parse_lsp_run_configs(&[json!({
            "name": "Debug Attach",
            "type": "java",
            "request": "attach",
            "host": "192.168.1.1",
            "port": 8787,
        })]);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "Debug Attach");
        match &configs[0].kind {
            DebugRunKind::Attach { host, port, .. } => {
                assert_eq!(host, "192.168.1.1");
                assert_eq!(*port, 8787);
            }
            other => panic!("expected Attach, got {other:?}"),
        }
    }

    #[test]
    fn parse_lsp_attach_defaults() {
        let configs = parse_lsp_run_configs(&[json!({
            "name": "Default Attach",
            "type": "java",
            "request": "attach",
        })]);
        assert_eq!(configs.len(), 1);
        match &configs[0].kind {
            DebugRunKind::Attach { host, port, .. } => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(*port, 5005);
            }
            other => panic!("expected Attach, got {other:?}"),
        }
    }

    #[test]
    fn parse_lsp_launch_config() {
        let configs = parse_lsp_run_configs(&[json!({
            "name": "Run Main",
            "type": "java",
            "request": "launch",
            "mainClass": "com.example.Main",
            "jvmArgs": ["-Xmx512m", "-Dfoo=bar"],
            "args": ["--port", "8080"],
            "cwd": "/home/user/project",
        })]);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "Run Main");
        match &configs[0].kind {
            DebugRunKind::Launch {
                main_class,
                jvm_args,
                args,
                cwd,
                ..
            } => {
                assert_eq!(main_class, "com.example.Main");
                assert_eq!(jvm_args, &["-Xmx512m", "-Dfoo=bar"]);
                assert_eq!(args, &["--port", "8080"]);
                assert_eq!(cwd.as_deref(), Some("/home/user/project"));
            }
            other => panic!("expected Launch, got {other:?}"),
        }
    }

    #[test]
    fn parse_lsp_launch_missing_main_class() {
        let configs = parse_lsp_run_configs(&[json!({
            "name": "No Main",
            "type": "java",
            "request": "launch",
            "jvmArgs": ["-Xmx512m"],
        })]);
        assert!(configs.is_empty());
    }

    #[test]
    fn parse_lsp_gradle_config() {
        let configs = parse_lsp_run_configs(&[json!({
            "name": "Run Tests",
            "type": "java",
            "request": "gradle",
            "task": ":test",
            "args": ["--tests", "com.example.MyTest", "--info"],
            "projectPath": "/home/user/project",
        })]);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "Run Tests");
        match &configs[0].kind {
            DebugRunKind::Gradle {
                task,
                args,
                project_root,
            } => {
                assert_eq!(task, ":test");
                assert_eq!(args, &["--tests", "com.example.MyTest", "--info"]);
                assert_eq!(project_root.as_deref(), Some("/home/user/project"));
            }
            other => panic!("expected Gradle, got {other:?}"),
        }
    }

    #[test]
    fn parse_lsp_unknown_request_skipped() {
        let configs = parse_lsp_run_configs(&[json!({
            "name": "Docker",
            "type": "docker",
            "request": "docker-deploy",
        })]);
        assert!(configs.is_empty());
    }

    #[test]
    fn parse_lsp_mixed_configs() {
        let configs = parse_lsp_run_configs(&[
            json!({"name": "Attach", "type": "java", "request": "attach", "port": 5005}),
            json!({"name": "Bad", "type": "java", "request": "unknown"}),
            json!({"name": "Test", "type": "java", "request": "gradle", "task": ":test"}),
        ]);
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].name, "Attach");
        assert_eq!(configs[1].name, "Test");
    }
}
