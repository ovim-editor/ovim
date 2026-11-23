//! Local launch plans for Java / Kotlin tests, used when the language server
//! cannot say how to run a test (`hyperion.resolveLaunch` missing, still
//! indexing, or returning nothing).
//!
//! The plan is composed from the file alone: tree-sitter finds the
//! `@Test` / `@ParameterizedTest` methods and their (nested) classes, the
//! nearest `build.gradle(.kts)` / `pom.xml` decides the build tool and module,
//! and the build tool's own test filter selects what to run
//! (Gradle `--tests`, Maven `-Dtest=`). It has exactly the shape of a plan
//! from the server, so everything after it (console, JUnit results, debug
//! attach) is shared.

use std::path::{Path, PathBuf};

use super::nearest::{discover_tests, jvm_package, nearest_test, DiscoveredTest};
use super::runners::TestScope;
use crate::launch::plan::{
    gradle_program, maven_program, with_clean_test, LaunchPlan, PlanKind, TaskPlan,
};
use crate::syntax::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildTool {
    Gradle,
    Maven,
}

struct Module {
    tool: BuildTool,
    /// Directory of the module that owns the file.
    dir: PathBuf,
    /// Workspace root (holds `settings.gradle(.kts)` / the top-level pom).
    root: PathBuf,
}

fn has_gradle_build(dir: &Path) -> bool {
    ["build.gradle", "build.gradle.kts"]
        .iter()
        .any(|f| dir.join(f).is_file())
}

/// The module that owns `file`: its nearest ancestor with a Gradle build
/// file or a pom. The workspace `root` is given, never rediscovered here: it
/// is the one the language server and the launch flow use
/// (`project_root::find_project_root_with_outermost`).
fn find_module(file: &Path, root: &Path) -> Option<Module> {
    let mut dir = file.parent()?;
    loop {
        let gradle = has_gradle_build(dir);
        if gradle || dir.join("pom.xml").is_file() {
            return Some(Module {
                tool: if gradle {
                    BuildTool::Gradle
                } else {
                    BuildTool::Maven
                },
                dir: dir.to_path_buf(),
                root: root.to_path_buf(),
            });
        }
        dir = dir.parent()?;
    }
}

/// Binary class name (`com.example.Outer$Inner`) of a discovered test's class.
fn binary_class(package: Option<&str>, classes: &[String]) -> String {
    let nested = classes.join("$");
    match package {
        Some(p) => format!("{p}.{nested}"),
        None => nested,
    }
}

/// Test selection for the build tool's filter.
struct Selection {
    /// `(class, method)`; a `None` method selects the whole class.
    filters: Vec<(String, Option<String>)>,
    label: String,
    class_name: Option<String>,
    method_name: Option<String>,
}

fn select(
    scope: TestScope,
    tests: &[DiscoveredTest],
    package: Option<&str>,
    cursor_line: usize,
    file: &Path,
    source: &str,
) -> Result<Selection, String> {
    match scope {
        TestScope::Nearest if class_declared_at(source, tests, cursor_line).is_some() => {
            // On a class declaration the nearest thing to run is that class
            // (with its nested classes), not the first test method below it.
            let (chain, _) = class_declared_at(source, tests, cursor_line).unwrap();
            let mut chains: Vec<&Vec<String>> = Vec::new();
            for test in tests {
                if test.namespaces.starts_with(&chain) && !chains.contains(&&test.namespaces) {
                    chains.push(&test.namespaces);
                }
            }
            let filters: Vec<_> = chains
                .iter()
                .map(|c| (binary_class(package, c), None))
                .collect();
            Ok(Selection {
                class_name: Some(binary_class(package, &chain)),
                filters,
                label: chain.last().cloned().unwrap_or_default(),
                method_name: None,
            })
        }
        TestScope::Nearest => {
            let test = nearest_test(tests, cursor_line)
                .ok_or_else(|| "No test found near cursor".to_string())?;
            let class = binary_class(package, &test.namespaces);
            let simple = test.namespaces.last().cloned().unwrap_or_default();
            // Parameterized invocations are reported as `name(int)` etc., so
            // the method filter is a plain name either way; the flavor only
            // matters for how results are matched afterwards.
            Ok(Selection {
                filters: vec![(class.clone(), Some(test.name.clone()))],
                label: format!("{simple}.{}", test.name),
                class_name: Some(class),
                method_name: Some(test.name.clone()),
            })
        }
        TestScope::File => {
            let mut chains: Vec<Vec<String>> = Vec::new();
            for test in tests {
                if !chains.contains(&test.namespaces) {
                    chains.push(test.namespaces.clone());
                }
            }
            if chains.is_empty() {
                return Err("No tests found in this file".to_string());
            }
            let filters: Vec<_> = chains
                .iter()
                .map(|c| (binary_class(package, c), None))
                .collect();
            let name = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(Selection {
                class_name: (filters.len() == 1).then(|| filters[0].0.clone()),
                filters,
                label: name,
                method_name: None,
            })
        }
        TestScope::Suite => Ok(Selection {
            filters: Vec::new(),
            label: "all tests".to_string(),
            class_name: None,
            method_name: None,
        }),
    }
}

/// A locally composed plan plus where in the file to ask the server about it.
pub struct LocalTest {
    pub plan: LaunchPlan,
    /// LSP position (line, UTF-16 column) inside the selected test / class.
    pub anchor: (usize, usize),
    /// The cursor is already inside the selected test (use it as is).
    pub cursor_inside: bool,
}

/// Line and indentation of the first line matching `class|object <name>`.
fn class_anchor(source: &str, name: &str) -> Option<(usize, usize)> {
    source.lines().enumerate().find_map(|(i, line)| {
        let pos = line.find(name)?;
        let before = line[..pos].trim_end();
        let is_decl = before.ends_with("class") || before.ends_with("object");
        let after = line[pos + name.len()..].chars().next();
        let boundary = after.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        (is_decl && boundary).then(|| (i, line.len() - line.trim_start().len()))
    })
}

/// The test class (namespace chain, outermost first) whose declaration line
/// (or the annotation lines right above it) holds `cursor_line`, with the
/// class name's position.
fn class_declared_at(
    source: &str,
    tests: &[DiscoveredTest],
    cursor_line: usize,
) -> Option<(Vec<String>, (usize, usize))> {
    let lines: Vec<&str> = source.lines().collect();
    let mut chains: Vec<&Vec<String>> = Vec::new();
    for test in tests {
        for end in 1..=test.namespaces.len() {
            let prefix = &test.namespaces[..end];
            if !chains.iter().any(|c| c.as_slice() == prefix) {
                chains.push(&test.namespaces);
            }
        }
    }
    // Innermost declaration wins when annotation ranges could overlap.
    let mut found: Option<(Vec<String>, (usize, usize))> = None;
    for chain in chains {
        for end in 1..=chain.len() {
            let prefix = &chain[..end];
            let Some(anchor) = class_anchor(source, &prefix[end - 1]) else {
                continue;
            };
            let mut top = anchor.0;
            while top > 0
                && lines
                    .get(top - 1)
                    .is_some_and(|l| l.trim_start().starts_with('@'))
            {
                top -= 1;
            }
            if (top..=anchor.0).contains(&cursor_line)
                && found.as_ref().is_none_or(|(c, _)| c.len() < prefix.len())
            {
                found = Some((prefix.to_vec(), anchor));
            }
        }
    }
    found
}

/// Composes a test plan for the file, or explains why it cannot. `root` is
/// the workspace root the build tool runs in.
pub fn local_test_plan(
    scope: TestScope,
    file: &Path,
    source: &str,
    cursor_line: usize,
    lang: Language,
    root: &Path,
) -> Result<LocalTest, String> {
    let tests = discover_tests(lang, source);
    let package = jvm_package(source);
    let module = find_module(file, root).ok_or_else(|| {
        "No Gradle or Maven project found for this file (looked for build.gradle(.kts) / pom.xml)"
            .to_string()
    })?;
    let selection = select(scope, &tests, package.as_deref(), cursor_line, file, source)?;
    let class_anchor_at_cursor = match scope {
        TestScope::Nearest => class_declared_at(source, &tests, cursor_line).map(|(_, a)| a),
        _ => None,
    };
    let anchor = match scope {
        TestScope::Nearest if class_anchor_at_cursor.is_some() => class_anchor_at_cursor,
        TestScope::Nearest => nearest_test(&tests, cursor_line).map(|t| {
            let indent = source
                .lines()
                .nth(t.line)
                .map(|l| l.len() - l.trim_start().len())
                .unwrap_or(0);
            (t.line, indent)
        }),
        _ => tests
            .first()
            .and_then(|t| t.namespaces.first())
            .and_then(|name| class_anchor(source, name)),
    }
    .unwrap_or((cursor_line, 0));
    let cursor_inside = class_anchor_at_cursor.is_none()
        && nearest_test(&tests, cursor_line)
            .is_some_and(|t| t.line <= cursor_line && cursor_line <= t.end_line);
    let rel = module
        .dir
        .strip_prefix(&module.root)
        .unwrap_or(Path::new(""));

    let (argv, debug_argv, cwd, reports_dir) = match module.tool {
        BuildTool::Gradle => {
            let project_path: String = rel
                .components()
                .map(|c| format!(":{}", c.as_os_str().to_string_lossy()))
                .collect();
            let task = format!("{project_path}:test");
            let mut argv = vec![gradle_program(&module.root), task.clone()];
            for (class, method) in &selection.filters {
                argv.push("--tests".to_string());
                argv.push(match method {
                    Some(m) => format!("{class}.{m}"),
                    None => class.clone(),
                });
            }
            argv.push("--console=plain".to_string());
            // `cleanTest` first: an unchanged rerun would otherwise be
            // UP-TO-DATE and write no new reports.
            let argv = with_clean_test(argv, Some(&task));
            let mut debug = argv.clone();
            debug.push("--debug-jvm".to_string());
            (
                argv,
                debug,
                module.root.clone(),
                module.dir.join("build/test-results/test"),
            )
        }
        BuildTool::Maven => {
            let mut argv = vec![maven_program(&module.root)];
            if !selection.filters.is_empty() {
                let spec = selection
                    .filters
                    .iter()
                    .map(|(class, method)| match method {
                        Some(m) => format!("{class}#{m}"),
                        None => class.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                argv.push(format!("-Dtest={spec}"));
                argv.push("-Dsurefire.failIfNoSpecifiedTests=false".to_string());
                argv.push("-DfailIfNoTests=false".to_string());
            }
            if !rel.as_os_str().is_empty() {
                argv.push("-pl".to_string());
                argv.push(rel.to_string_lossy().into_owned());
                argv.push("-am".to_string());
            }
            argv.push("test".to_string());
            let mut debug = argv.clone();
            debug.insert(1, "-Dmaven.surefire.debug".to_string());
            (
                argv,
                debug,
                module.root.clone(),
                module.dir.join("target/surefire-reports"),
            )
        }
    };

    let plan = LaunchPlan {
        name: format!("{} (tests)", selection.label),
        kind: PlanKind::Test,
        language: Some(
            match lang {
                Language::Kotlin => "kotlin",
                _ => "java",
            }
            .to_string(),
        ),
        project_root: module.root.clone(),
        module_dir: Some(module.dir.clone()),
        build_tool: match module.tool {
            BuildTool::Gradle => "gradle",
            BuildTool::Maven => "maven",
        }
        .to_string(),
        build: None,
        launch: None,
        task: Some(TaskPlan {
            argv,
            debug_argv: Some(debug_argv),
            cwd,
            class_name: selection.class_name,
            method_name: selection.method_name,
            reports_dir: Some(reports_dir),
            shell: None,
        }),
        attach: None,
        warnings: Vec::new(),
    };
    Ok(LocalTest {
        plan,
        anchor,
        cursor_inside,
    })
}
