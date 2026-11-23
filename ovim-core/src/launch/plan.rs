//! Launch plans: the one shape every way of starting a program is
//! normalised into, whether it came from Hyperion's `hyperion.resolveLaunch`
//! (see the launch contract), from `.ovim/debug.toml`, or from
//! `hyperion.runConfigurations`.
//!
//! A plan is pure data. The editor turns it into build / run / debug steps.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::process::CommandSpec;
use crate::debug_config::{DebugRunConfig, DebugRunKind};

/// Run without a debugger, or under the debugger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Run,
    Debug,
}

impl LaunchMode {
    pub fn verb(self) -> &'static str {
        match self {
            LaunchMode::Run => "Run",
            LaunchMode::Debug => "Debug",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    /// A `main` entry point started directly with `java`.
    Main,
    /// Tests run through the build tool (`test` details present).
    Test,
    /// An arbitrary build-tool task (Gradle config from `debug.toml`).
    Task,
    /// Attach to a JVM that is already listening.
    Attach,
}

/// A `java` invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct JavaLaunch {
    pub main_class: String,
    pub classpath: String,
    pub args: Vec<String>,
    pub jvm_args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl JavaLaunch {
    /// The command that runs this program without a debugger.
    pub fn run_command(&self) -> CommandSpec {
        self.command(false)
    }

    /// The command that starts the JVM suspended, waiting for a debugger on a
    /// free port it announces with `Listening for transport dt_socket at
    /// address: N`. ovim owns the process (stdin, output, process group) and
    /// the adapter attaches to it.
    pub fn debug_command(&self) -> CommandSpec {
        self.command(true)
    }

    fn command(&self, debug: bool) -> CommandSpec {
        let mut argv = vec![java_program()];
        argv.extend(self.jvm_args.iter().cloned());
        if debug {
            argv.push(
                "-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=127.0.0.1:0"
                    .to_string(),
            );
        }
        if !self.classpath.is_empty() {
            argv.push("-cp".to_string());
            argv.push(self.classpath.clone());
        }
        argv.push(self.main_class.clone());
        argv.extend(self.args.iter().cloned());
        CommandSpec {
            argv,
            cwd: self.cwd.clone(),
            env: self.env.clone(),
        }
    }
}

/// Tests (or another build-tool task) started through the build tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPlan {
    pub argv: Vec<String>,
    /// argv that makes the test JVM wait for a debugger and print
    /// `Listening for transport dt_socket at address: N`.
    pub debug_argv: Option<Vec<String>>,
    pub cwd: PathBuf,
    pub class_name: Option<String>,
    pub method_name: Option<String>,
    /// Directory with JUnit XML results, parsed after the run.
    pub reports_dir: Option<PathBuf>,
    /// Set when the task is a command line run through the shell instead of
    /// a build-tool invocation (see [`LaunchPlan::shell`]).
    pub shell: Option<ShellRun>,
}

/// A command line run through the platform shell: non-JVM test runs
/// (`<Space>t*`, plan kind [`PlanKind::Test`], results parsed from the output
/// into the test panel) and `:make` (plan kind [`PlanKind::Task`],
/// diagnostics parsed from the output into the quickfix list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRun {
    pub command: String,
    /// "nearest" / "file" / "suite" / "re-run" for tests, "make" for `:make`.
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachTarget {
    pub host: String,
    pub port: u16,
    pub project_root: PathBuf,
}

/// Everything needed to build, run, or debug one thing.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPlan {
    pub name: String,
    pub kind: PlanKind,
    /// `java`, `kotlin`, ... Used to pick the debug adapter.
    pub language: Option<String>,
    pub project_root: PathBuf,
    pub module_dir: Option<PathBuf>,
    pub build_tool: String,
    pub build: Option<CommandSpec>,
    pub launch: Option<JavaLaunch>,
    pub task: Option<TaskPlan>,
    pub attach: Option<AttachTarget>,
    pub warnings: Vec<String>,
}

impl LaunchPlan {
    /// A plan that runs `command` in `cwd` through the platform shell.
    pub fn shell(kind: PlanKind, label: &'static str, command: &str, cwd: PathBuf) -> Self {
        let argv: &[&str] = if cfg!(windows) {
            &["cmd", "/C"]
        } else {
            &["sh", "-c"]
        };
        Self {
            name: command.to_string(),
            kind,
            language: None,
            project_root: cwd.clone(),
            module_dir: None,
            build_tool: "shell".to_string(),
            build: None,
            launch: None,
            task: Some(TaskPlan {
                argv: argv
                    .iter()
                    .map(|a| a.to_string())
                    .chain([command.to_string()])
                    .collect(),
                debug_argv: None,
                cwd,
                class_name: None,
                method_name: None,
                reports_dir: None,
                shell: Some(ShellRun {
                    command: command.to_string(),
                    label,
                }),
            }),
            attach: None,
            warnings: Vec::new(),
        }
    }

    /// Directories that may contain the sources behind stack-trace frames.
    pub fn source_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(module) = &self.module_dir {
            roots.push(module.clone());
        }
        if !roots.contains(&self.project_root) {
            roots.push(self.project_root.clone());
        }
        roots
    }

    /// Why this plan cannot be started in `mode`, if it cannot.
    pub fn unsupported_reason(&self, mode: LaunchMode) -> Option<String> {
        match (self.kind, mode) {
            (PlanKind::Attach, LaunchMode::Run) => Some(format!(
                "'{}' attaches to a running JVM, so it can only be debugged",
                self.name
            )),
            (PlanKind::Test | PlanKind::Task, LaunchMode::Debug)
                if self.task.as_ref().is_none_or(|t| t.debug_argv.is_none()) =>
            {
                Some(format!(
                    "'{}' has no debug command (missing debugArgv)",
                    self.name
                ))
            }
            (PlanKind::Main, _) if self.launch.is_none() => {
                Some(format!("'{}' has no launch information", self.name))
            }
            (PlanKind::Test | PlanKind::Task, _) if self.task.is_none() => {
                Some(format!("'{}' has no task information", self.name))
            }
            _ => None,
        }
    }
}

/// `java` executable: `$JAVA_HOME/bin/java` when it exists, else `java`.
pub fn java_program() -> String {
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let candidate =
            Path::new(&home)
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" });
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    "java".to_string()
}

/// `./gradlew` only when the wrapper is actually usable (its jar exists),
/// otherwise `gradle` from PATH.
pub fn gradle_program(root: &Path) -> String {
    let script = root.join(if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    });
    if root.join("gradle/wrapper/gradle-wrapper.jar").is_file() && script.is_file() {
        script.to_string_lossy().into_owned()
    } else {
        "gradle".to_string()
    }
}

/// `./mvnw` only when `.mvn/wrapper` exists, otherwise `mvn` from PATH.
pub fn maven_program(root: &Path) -> String {
    let script = root.join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" });
    if root.join(".mvn/wrapper").is_dir() && script.is_file() {
        script.to_string_lossy().into_owned()
    } else {
        "mvn".to_string()
    }
}

// ---- hyperion.resolveLaunch ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawResolved {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    project_root: Option<String>,
    #[serde(default)]
    module_dir: Option<String>,
    #[serde(default)]
    build_tool: Option<String>,
    #[serde(default)]
    build: Option<RawCommand>,
    #[serde(default)]
    launch: Option<Value>,
    #[serde(default)]
    test: Option<RawTest>,
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Deserialize)]
struct RawCommand {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTest {
    #[serde(default)]
    class_name: Option<String>,
    #[serde(default)]
    method_name: Option<String>,
    #[serde(default)]
    argv: Vec<String>,
    #[serde(default)]
    debug_argv: Option<Vec<String>>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    reports_dir: Option<String>,
    #[serde(default)]
    gradle_task: Option<String>,
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn env_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

fn java_launch_from_json(raw: &Value, fallback_cwd: &Path) -> Result<JavaLaunch, String> {
    let main_class = raw
        .get("mainClass")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("launch information has no mainClass")?
        .to_owned();
    let cwd = raw
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback_cwd.to_path_buf());
    Ok(JavaLaunch {
        main_class,
        classpath: raw
            .get("classpath")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        args: string_list(raw.get("args")),
        jvm_args: string_list(raw.get("jvmArgs")),
        cwd,
        env: env_map(raw.get("env")),
    })
}

/// Parses the result of `hyperion.resolveLaunch`. `Ok(None)` for a JSON
/// `null` (nothing runnable at that position).
pub fn plan_from_resolved(value: &Value) -> Result<Option<LaunchPlan>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let raw: RawResolved = serde_json::from_value(value.clone())
        .map_err(|e| format!("unexpected resolveLaunch result: {e}"))?;
    let project_root = PathBuf::from(
        raw.project_root
            .clone()
            .or_else(|| {
                raw.launch
                    .as_ref()
                    .and_then(|l| l.get("projectRoot"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .ok_or("resolveLaunch result has no projectRoot")?,
    );
    let module_dir = raw.module_dir.map(PathBuf::from);
    let name = raw.name.unwrap_or_else(|| "Application".to_string());
    let build = raw.build.map(|b| CommandSpec {
        argv: b.argv,
        cwd: b
            .cwd
            .map(PathBuf::from)
            .unwrap_or_else(|| project_root.clone()),
        env: BTreeMap::new(),
    });
    let build = build.filter(|b| !b.argv.is_empty());

    let kind_name = raw
        .kind
        .as_deref()
        .unwrap_or(if raw.test.is_some() { "test" } else { "main" });
    let mut plan = LaunchPlan {
        name,
        kind: PlanKind::Main,
        language: raw.language,
        project_root: project_root.clone(),
        module_dir: module_dir.clone(),
        build_tool: raw.build_tool.unwrap_or_else(|| "none".to_string()),
        build,
        launch: None,
        task: None,
        attach: None,
        warnings: raw.warnings,
    };
    match kind_name {
        "test" => {
            let test = raw
                .test
                .ok_or("resolveLaunch kind is 'test' but has no test block")?;
            plan.kind = PlanKind::Test;
            // Tests go through the build tool, which compiles for itself.
            plan.build = None;
            let cwd = test
                .cwd
                .map(PathBuf::from)
                .unwrap_or_else(|| project_root.clone());
            let clean = |argv: Vec<String>| with_clean_test(argv, test.gradle_task.as_deref());
            plan.task = Some(TaskPlan {
                argv: clean(test.argv),
                debug_argv: test.debug_argv.filter(|a| !a.is_empty()).map(clean),
                cwd,
                class_name: test.class_name,
                method_name: test.method_name,
                reports_dir: test.reports_dir.map(PathBuf::from),
                shell: None,
            });
        }
        _ => {
            let launch = raw
                .launch
                .ok_or("resolveLaunch result has no launch block")?;
            let fallback = module_dir.as_deref().unwrap_or(&project_root);
            plan.launch = Some(java_launch_from_json(&launch, fallback)?);
        }
    }
    Ok(Some(plan))
}

/// Gradle skips an unchanged test task as UP-TO-DATE and writes no new
/// reports, so rerunning a test would show nothing. Runs `cleanTest` (in the
/// same project) first. Other argv (Maven, custom) is returned as is.
pub fn with_clean_test(mut argv: Vec<String>, gradle_task: Option<&str>) -> Vec<String> {
    let Some(task) = gradle_task.filter(|t| t.ends_with("test") || t.ends_with("Test")) else {
        return argv;
    };
    let Some(pos) = argv.iter().position(|a| a == task) else {
        return argv;
    };
    let clean = match task.rsplit_once(':') {
        Some((project, name)) => format!("{project}:clean{}", capitalize(name)),
        None => format!("clean{}", capitalize(task)),
    };
    if !argv.contains(&clean) {
        argv.insert(pos, clean);
    }
    argv
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---- .ovim/debug.toml and hyperion.runConfigurations ----

fn resolve_against(root: &Path, value: Option<&str>) -> Option<PathBuf> {
    value.map(|v| {
        let p = PathBuf::from(v);
        if p.is_absolute() {
            p
        } else {
            root.join(p)
        }
    })
}

/// Turns a configured run configuration into a plan. `default_root` is the
/// LSP workspace root of the current file (not the process working directory).
pub fn plan_from_config(config: &DebugRunConfig, default_root: &Path) -> LaunchPlan {
    let base = LaunchPlan {
        name: config.name.clone(),
        kind: PlanKind::Main,
        language: Some("java".to_string()),
        project_root: default_root.to_path_buf(),
        module_dir: None,
        build_tool: "none".to_string(),
        build: None,
        launch: None,
        task: None,
        attach: None,
        warnings: Vec::new(),
    };
    match &config.kind {
        DebugRunKind::Gradle {
            task,
            args,
            project_root,
        } => {
            let root = resolve_against(default_root, project_root.as_deref())
                .unwrap_or_else(|| default_root.to_path_buf());
            let mut argv = vec![gradle_program(&root), task.clone()];
            argv.extend(args.iter().cloned());
            if !argv.iter().any(|a| a.starts_with("--console")) {
                argv.push("--console=plain".to_string());
            }
            let mut debug_argv = argv.clone();
            if !debug_argv.iter().any(|a| a == "--debug-jvm") {
                debug_argv.push("--debug-jvm".to_string());
            }
            LaunchPlan {
                kind: PlanKind::Task,
                project_root: root.clone(),
                build_tool: "gradle".to_string(),
                task: Some(TaskPlan {
                    argv,
                    debug_argv: Some(debug_argv),
                    cwd: root,
                    class_name: None,
                    method_name: None,
                    reports_dir: None,
                    shell: None,
                }),
                ..base
            }
        }
        DebugRunKind::Attach {
            host,
            port,
            project_root,
        } => {
            let root = resolve_against(default_root, project_root.as_deref())
                .unwrap_or_else(|| default_root.to_path_buf());
            LaunchPlan {
                kind: PlanKind::Attach,
                project_root: root.clone(),
                attach: Some(AttachTarget {
                    host: host.clone(),
                    port: *port,
                    project_root: root,
                }),
                ..base
            }
        }
        DebugRunKind::Launch {
            main_class,
            classpath,
            args,
            jvm_args,
            cwd,
            project_root,
            build,
        } => {
            let root = resolve_against(default_root, project_root.as_deref())
                .unwrap_or_else(|| default_root.to_path_buf());
            let cwd = resolve_against(&root, cwd.as_deref()).unwrap_or_else(|| root.clone());
            let launch = JavaLaunch {
                main_class: main_class.clone(),
                classpath: classpath.clone().unwrap_or_default(),
                args: args.clone(),
                jvm_args: jvm_args.clone(),
                cwd,
                env: BTreeMap::new(),
            };
            LaunchPlan {
                project_root: root.clone(),
                build: build
                    .as_ref()
                    .filter(|argv| !argv.is_empty())
                    .map(|argv| CommandSpec {
                        argv: argv.clone(),
                        cwd: root.clone(),
                        env: BTreeMap::new(),
                    }),
                launch: Some(launch),
                ..base
            }
        }
    }
}

/// Extracts the port from a JDWP "Listening for transport dt_socket at
/// address: 5005" line, wherever it appears on the line.
/// Surefire forks the test JVM with its output captured into the forked-VM
/// protocol channel, so the `Listening for transport dt_socket` line of a
/// `-Dmaven.surefire.debug` run never reaches Maven's output. Give the JVM an
/// address of our choosing instead (also avoiding the hardcoded 5005), and
/// watch that port. Returns the rewritten argv and the port, or `None` when
/// `argv` has no bare `-Dmaven.surefire.debug`.
pub fn pin_surefire_debug_port(argv: &[String], port: u16) -> Option<Vec<String>> {
    if !argv.iter().any(|a| a == "-Dmaven.surefire.debug") {
        return None;
    }
    Some(
        argv.iter()
            .map(|a| {
                if a == "-Dmaven.surefire.debug" {
                    format!(
                        "-Dmaven.surefire.debug=-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=127.0.0.1:{port}"
                    )
                } else {
                    a.clone()
                }
            })
            .collect(),
    )
}

/// The port of an explicit `-Dmaven.surefire.debug=...address=host:PORT`.
pub fn surefire_debug_port(argv: &[String]) -> Option<u16> {
    let value = argv
        .iter()
        .find_map(|a| a.strip_prefix("-Dmaven.surefire.debug="))?;
    let address = value.split("address=").nth(1)?;
    let address = address.split(',').next()?;
    address.rsplit(':').next()?.parse().ok()
}

pub fn parse_listening_port(line: &str) -> Option<u16> {
    let idx = line.find("Listening for transport dt_socket at address:")?;
    let rest = &line[idx + "Listening for transport dt_socket at address:".len()..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn surefire_debug_gets_a_pinned_port_that_can_be_read_back() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let plain = argv(&["mvn", "-Dtest=A#b", "-Dmaven.surefire.debug", "test"]);
        let pinned = pin_surefire_debug_port(&plain, 4711).unwrap();
        assert_eq!(surefire_debug_port(&pinned), Some(4711));
        assert_eq!(pinned[1], "-Dtest=A#b");
        assert!(pin_surefire_debug_port(&argv(&["gradle", "test"]), 1).is_none());
        // Someone else's explicit address is respected, not rewritten.
        let own = argv(&["mvn", "-Dmaven.surefire.debug=-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=localhost:6006"]);
        assert!(pin_surefire_debug_port(&own, 1).is_none());
        assert_eq!(surefire_debug_port(&own), Some(6006));
    }

    #[test]
    fn gradle_test_argv_gets_a_clean_task_so_reruns_are_not_up_to_date() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            with_clean_test(
                argv(&["gradlew", ":app:test", "--tests", "A"]),
                Some(":app:test")
            ),
            argv(&["gradlew", ":app:cleanTest", ":app:test", "--tests", "A"])
        );
        assert_eq!(
            with_clean_test(argv(&["gradle", "test"]), Some("test")),
            argv(&["gradle", "cleanTest", "test"])
        );
        // Maven (no gradleTask) and already-clean argv are untouched.
        let mvn = argv(&["mvn", "-Dtest=A", "test"]);
        assert_eq!(with_clean_test(mvn.clone(), None), mvn);
        let done = argv(&["gradle", ":a:cleanTest", ":a:test"]);
        assert_eq!(with_clean_test(done.clone(), Some(":a:test")), done);
    }

    use super::*;
    use serde_json::json;

    fn sample_main() -> Value {
        json!({
            "name": "Main (app)",
            "kind": "main",
            "language": "java",
            "projectRoot": "/r",
            "moduleDir": "/r/app",
            "buildTool": "gradle",
            "build": {"argv": ["/r/gradlew", ":app:classes", "--console=plain"], "cwd": "/r"},
            "launch": {
                "mainClass": "com.example.Main",
                "classpath": "/r/app/build/classes/java/main:/j.jar",
                "args": ["a b", "c"], "jvmArgs": ["-Xmx64m"],
                "cwd": "/r/app", "projectRoot": "/r", "env": {"K": "V"}
            },
            "warnings": ["classpath incomplete"]
        })
    }

    #[test]
    fn resolves_a_main_plan_from_the_contract_shape() {
        let plan = plan_from_resolved(&sample_main()).unwrap().unwrap();
        assert_eq!(plan.kind, PlanKind::Main);
        assert_eq!(plan.project_root, PathBuf::from("/r"));
        assert_eq!(plan.module_dir, Some(PathBuf::from("/r/app")));
        assert_eq!(plan.build.as_ref().unwrap().argv[1], ":app:classes");
        assert_eq!(plan.warnings, vec!["classpath incomplete".to_string()]);
        let launch = plan.launch.as_ref().unwrap();
        assert_eq!(launch.main_class, "com.example.Main");
        assert_eq!(launch.env.get("K").map(String::as_str), Some("V"));
        let cmd = launch.run_command();
        assert_eq!(
            &cmd.argv[1..],
            &[
                "-Xmx64m",
                "-cp",
                "/r/app/build/classes/java/main:/j.jar",
                "com.example.Main",
                "a b",
                "c"
            ]
        );
        assert_eq!(cmd.cwd, PathBuf::from("/r/app"));
    }

    #[test]
    fn null_result_means_nothing_to_run() {
        assert_eq!(plan_from_resolved(&Value::Null), Ok(None));
    }

    #[test]
    fn resolves_a_test_plan_and_drops_the_separate_build_step() {
        let plan = plan_from_resolved(&json!({
            "name": "FooTest.handlesEmpty", "kind": "test", "language": "java",
            "projectRoot": "/r", "moduleDir": "/r/app", "buildTool": "gradle",
            "build": {"argv": ["gradle", ":app:testClasses"], "cwd": "/r"},
            "launch": {"mainClass": "x", "projectRoot": "/r"},
            "test": {
                "className": "com.example.FooTest", "methodName": "handlesEmpty",
                "gradleTask": ":app:test",
                "argv": ["/r/gradlew", ":app:test", "--tests", "com.example.FooTest.handlesEmpty"],
                "debugArgv": ["/r/gradlew", ":app:test", "--tests", "com.example.FooTest.handlesEmpty", "--debug-jvm"],
                "cwd": "/r", "reportsDir": "/r/app/build/test-results/test"
            }
        }))
        .unwrap()
        .unwrap();
        assert_eq!(plan.kind, PlanKind::Test);
        assert!(plan.build.is_none());
        let task = plan.task.unwrap();
        assert_eq!(task.method_name.as_deref(), Some("handlesEmpty"));
        assert!(task
            .debug_argv
            .unwrap()
            .contains(&"--debug-jvm".to_string()));
        assert_eq!(
            task.reports_dir,
            Some(PathBuf::from("/r/app/build/test-results/test"))
        );
    }

    #[test]
    fn malformed_results_explain_what_is_missing() {
        assert!(plan_from_resolved(&json!({"kind": "main"}))
            .unwrap_err()
            .contains("projectRoot"));
        assert!(plan_from_resolved(&json!({"projectRoot": "/r"}))
            .unwrap_err()
            .contains("launch"));
        assert!(plan_from_resolved(&json!("nope")).is_err());
    }

    #[test]
    fn listening_line_port_is_parsed_from_either_stream_shape() {
        assert_eq!(
            parse_listening_port("Listening for transport dt_socket at address: 5005"),
            Some(5005)
        );
        assert_eq!(
            parse_listening_port(
                "> Task :test\nListening for transport dt_socket at address: 41977"
            ),
            Some(41977)
        );
        assert_eq!(
            parse_listening_port("Listening for transport dt_socket at address: "),
            None
        );
        assert_eq!(parse_listening_port("nothing here"), None);
    }

    #[test]
    fn gradle_config_uses_wrapper_only_when_its_jar_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gradlew"), "#!/bin/sh\n").unwrap();
        assert_eq!(
            gradle_program(dir.path()),
            "gradle",
            "wrapper script without jar is unusable"
        );
        std::fs::create_dir_all(dir.path().join("gradle/wrapper")).unwrap();
        std::fs::write(dir.path().join("gradle/wrapper/gradle-wrapper.jar"), "").unwrap();
        assert!(gradle_program(dir.path()).ends_with("gradlew"));

        let config = DebugRunConfig {
            name: "t".into(),
            kind: DebugRunKind::Gradle {
                task: ":app:test".into(),
                args: vec!["--tests".into(), "X".into()],
                project_root: None,
            },
        };
        let plan = plan_from_config(&config, dir.path());
        let task = plan.task.unwrap();
        assert_eq!(task.argv[1], ":app:test");
        assert!(task.argv.contains(&"--console=plain".to_string()));
        assert_eq!(task.debug_argv.unwrap().last().unwrap(), "--debug-jvm");
    }

    #[test]
    fn launch_config_resolves_relative_paths_against_the_project_root() {
        let config = DebugRunConfig {
            name: "l".into(),
            kind: DebugRunKind::Launch {
                main_class: "a.B".into(),
                classpath: Some("out".into()),
                args: vec![],
                jvm_args: vec![],
                cwd: Some("work".into()),
                project_root: None,
                build: Some(vec!["gradle".into(), "classes".into()]),
            },
        };
        let plan = plan_from_config(&config, Path::new("/proj"));
        assert_eq!(
            plan.launch.as_ref().unwrap().cwd,
            PathBuf::from("/proj/work")
        );
        assert_eq!(plan.build.as_ref().unwrap().cwd, PathBuf::from("/proj"));
    }

    #[test]
    fn attach_plans_cannot_be_run_without_a_debugger() {
        let config = DebugRunConfig {
            name: "a".into(),
            kind: DebugRunKind::Attach {
                host: "127.0.0.1".into(),
                port: 5005,
                project_root: None,
            },
        };
        let plan = plan_from_config(&config, Path::new("/proj"));
        assert!(plan
            .unsupported_reason(LaunchMode::Run)
            .unwrap()
            .contains("attaches"));
        assert!(plan.unsupported_reason(LaunchMode::Debug).is_none());
    }
}
