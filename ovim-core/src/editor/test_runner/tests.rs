//! Tests for tree-sitter test discovery and command construction.
//!
//! Expected commands are derived from the actual CLI contracts of cargo
//! test, vitest, jest, pytest and go test (see runners.rs doc comments for
//! the dialect rules), cross-checked against vim-test's runner sources and
//! its issue tracker's known bugs — several cases below are regression
//! tests for bugs vim-test still has.

use super::nearest::{discover_tests, nearest_test, TestFlavor};
use super::runners::{build_test_command, TestContext, TestScope};
use crate::editor::Editor;
use crate::language_config::TestConfig;
use crate::syntax::Language;
use std::fs;
use std::path::{Path, PathBuf};

fn ctx<'a>(
    file: &'a Path,
    source: &'a str,
    cursor_line: usize,
    language: Language,
) -> TestContext<'a> {
    TestContext {
        file,
        source,
        cursor_line,
        language: Some(language),
        config: None,
    }
}

// ---------------------------------------------------------------------------
// Rust discovery
// ---------------------------------------------------------------------------

const RUST_SRC: &str = r#"
fn helper() {}

#[test]
fn top_level_test() {
    assert!(true);
}

mod outer {
    mod tests {
        #[test]
        fn nested_test() {}

        #[tokio::test]
        async fn async_test() {}

        #[rstest]
        #[case(1)]
        fn param_test(#[case] n: u32) {}
    }
}
"#;

#[test]
fn rust_discovers_tests_with_module_paths() {
    let tests = discover_tests(Language::Rust, RUST_SRC);
    let names: Vec<(String, Vec<String>)> = tests
        .iter()
        .map(|t| (t.name.clone(), t.namespaces.clone()))
        .collect();
    assert!(names.contains(&("top_level_test".into(), vec![])));
    assert!(names.contains(&("nested_test".into(), vec!["outer".into(), "tests".into()])));
    assert!(names.contains(&("async_test".into(), vec!["outer".into(), "tests".into()])));
}

#[test]
fn rust_rstest_is_parameterized() {
    let tests = discover_tests(Language::Rust, RUST_SRC);
    let param = tests.iter().find(|t| t.name == "param_test").unwrap();
    assert_eq!(param.flavor, TestFlavor::Parameterized);
    let plain = tests.iter().find(|t| t.name == "top_level_test").unwrap();
    assert_eq!(plain.flavor, TestFlavor::Exact);
}

#[test]
fn rust_ignores_non_test_functions() {
    let tests = discover_tests(Language::Rust, RUST_SRC);
    assert!(!tests.iter().any(|t| t.name == "helper"));
}

#[test]
fn rust_attribute_with_comment_between() {
    let src = "#[test]\n// a comment\nfn commented_test() {}\n";
    let tests = discover_tests(Language::Rust, src);
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].name, "commented_test");
}

// ---------------------------------------------------------------------------
// Nearest selection
// ---------------------------------------------------------------------------

#[test]
fn nearest_prefers_containing_then_above_then_below() {
    let tests = discover_tests(Language::Rust, RUST_SRC);
    // Cursor inside top_level_test's body (line 5 of the literal, 0-indexed 4).
    let inside = nearest_test(&tests, 4).unwrap();
    assert_eq!(inside.name, "top_level_test");
    // Cursor after top_level_test but before the mod: picks the test above.
    let above = nearest_test(&tests, 7).unwrap();
    assert_eq!(above.name, "top_level_test");
    // Cursor at the top of the file: falls forward to the first test.
    let below = nearest_test(&tests, 0).unwrap();
    assert_eq!(below.name, "top_level_test");
}

// ---------------------------------------------------------------------------
// Rust command construction
// ---------------------------------------------------------------------------

struct RustProject {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn rust_package() -> RustProject {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("src/editor")).unwrap();
    fs::create_dir_all(root.join("src/bin")).unwrap();
    fs::create_dir_all(root.join("tests")).unwrap();
    RustProject { _dir: dir, root }
}

#[test]
fn rust_nearest_uses_full_module_path_and_exact() {
    let p = rust_package();
    let file = p.root.join("src/editor/input.rs");
    fs::write(&file, "").unwrap();
    let src = "mod tests {\n    #[test]\n    fn handles_keys() {}\n}\n";
    let inv = build_test_command(TestScope::Nearest, &ctx(&file, src, 2, Language::Rust)).unwrap();
    assert_eq!(
        inv.command,
        "cargo test 'editor::input::tests::handles_keys' -- --exact"
    );
    assert_eq!(inv.cwd, p.root);
}

#[test]
fn rust_file_scope_module_filter_survives_mod_in_name() {
    // Regression: the old implementation ran `.replace("mod", "")` on the
    // path, so src/editor/mode.rs produced the filter `editor::e`.
    let p = rust_package();
    let file = p.root.join("src/editor/mode.rs");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::File, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test 'editor::mode::'");
}

#[test]
fn rust_mod_rs_maps_to_parent_module() {
    let p = rust_package();
    let file = p.root.join("src/editor/mod.rs");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::File, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test 'editor::'");
}

#[test]
fn rust_integration_test_uses_test_target() {
    // vim-test never emits --test for integration targets (open TODO there).
    let p = rust_package();
    let file = p.root.join("tests/api_test.rs");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::File, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test --test 'api_test'");

    let src = "#[test]\nfn hits_endpoint() {}\n";
    let inv = build_test_command(TestScope::Nearest, &ctx(&file, src, 1, Language::Rust)).unwrap();
    assert_eq!(
        inv.command,
        "cargo test --test 'api_test' 'hits_endpoint' -- --exact"
    );
}

#[test]
fn rust_bin_target() {
    let p = rust_package();
    let file = p.root.join("src/bin/tool.rs");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::File, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test --bin 'tool'");
}

#[test]
fn rust_lib_root_runs_whole_package() {
    let p = rust_package();
    let file = p.root.join("src/lib.rs");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::File, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test");
}

#[test]
fn rust_suite_uses_workspace_root() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().canonicalize().unwrap();
    fs::write(ws.join("Cargo.toml"), "[workspace]\nmembers = [\"demo\"]\n").unwrap();
    fs::create_dir_all(ws.join("demo/src")).unwrap();
    fs::write(
        ws.join("demo/Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let file = ws.join("demo/src/lib.rs");
    fs::write(&file, "").unwrap();

    let inv = build_test_command(TestScope::Suite, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test --workspace");
    assert_eq!(inv.cwd, ws);

    // File scope still runs in the package, not the workspace root.
    let inv = build_test_command(TestScope::File, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.cwd, ws.join("demo"));
}

#[test]
fn rust_parameterized_nearest_drops_exact() {
    let p = rust_package();
    let file = p.root.join("src/lib.rs");
    fs::write(&file, "").unwrap();
    let src = "#[rstest]\n#[case(1)]\nfn with_cases(#[case] n: u32) {}\n";
    let inv = build_test_command(TestScope::Nearest, &ctx(&file, src, 2, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test 'with_cases'");
}

// ---------------------------------------------------------------------------
// JavaScript / TypeScript
// ---------------------------------------------------------------------------

const JS_SRC: &str = r#"
describe('math utils', () => {
  it('adds numbers', () => {
    expect(1 + 1).toBe(2);
  });

  describe('edge cases', () => {
    it("doesn't overflow (hopefully?)", () => {});
  });

  it.each([1, 2])('handles %d items', (n) => {});
});
"#;

#[test]
fn js_discovers_nested_describes() {
    let tests = discover_tests(Language::TypeScript, JS_SRC);
    let nested = tests
        .iter()
        .find(|t| t.name.starts_with("doesn't"))
        .unwrap();
    assert_eq!(
        nested.namespaces,
        vec!["math utils".to_string(), "edge cases".to_string()]
    );
    let each = tests.iter().find(|t| t.name.contains("%d")).unwrap();
    assert_eq!(each.flavor, TestFlavor::Parameterized);
}

struct JsProject {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn js_package(package_json: &str) -> JsProject {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("package.json"), package_json).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    JsProject { _dir: dir, root }
}

#[test]
fn js_vitest_nearest_is_anchored_and_escaped() {
    // Regression class: vim-test#625 — quotes/regex chars in test names must
    // survive both regex escaping and shell quoting.
    let p = js_package(r#"{"devDependencies": {"vitest": "^2.0.0"}}"#);
    let file = p.root.join("src/math.test.ts");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, JS_SRC, 7, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(
        inv.command,
        r#"npx vitest run -t '^math utils edge cases doesn'\''t overflow \(hopefully\?\)$' 'src/math.test.ts'"#
    );
}

#[test]
fn js_each_printf_name_is_truncated_and_unanchored() {
    let p = js_package(r#"{"devDependencies": {"vitest": "^2.0.0"}}"#);
    let file = p.root.join("src/math.test.ts");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, JS_SRC, 10, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(
        inv.command,
        "npx vitest run -t 'math utils handles' 'src/math.test.ts'"
    );
}

#[test]
fn js_jest_detected_from_config_file() {
    let p = js_package(r#"{"name": "demo"}"#);
    fs::write(p.root.join("jest.config.js"), "module.exports = {}").unwrap();
    let file = p.root.join("src/math.test.ts");
    fs::write(&file, "").unwrap();
    let inv =
        build_test_command(TestScope::File, &ctx(&file, "", 0, Language::TypeScript)).unwrap();
    assert_eq!(inv.command, "npx jest --runTestsByPath 'src/math.test.ts'");
}

#[test]
fn js_monorepo_finds_hoisted_runner_and_package_cwd() {
    // vim-test#272/#490: detection reads vim's cwd, so nested packages fail.
    // Here the runner lives in the workspace root and the file in a package.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().canonicalize().unwrap();
    fs::write(
        ws.join("package.json"),
        r#"{"devDependencies": {"vitest": "1.0.0"}, "workspaces": ["packages/*"]}"#,
    )
    .unwrap();
    fs::create_dir_all(ws.join("packages/app/src")).unwrap();
    fs::write(ws.join("packages/app/package.json"), r#"{"name": "app"}"#).unwrap();
    let file = ws.join("packages/app/src/app.test.ts");
    fs::write(&file, "").unwrap();

    let inv =
        build_test_command(TestScope::File, &ctx(&file, "", 0, Language::TypeScript)).unwrap();
    // cwd is the package, not the workspace; runner found by walking up.
    assert_eq!(inv.cwd, ws.join("packages/app"));
    assert_eq!(inv.command, "npx vitest run 'src/app.test.ts'");
}

#[test]
fn js_bun_detected_from_lockfile() {
    let p = js_package(r#"{"name": "demo"}"#);
    fs::write(p.root.join("bun.lock"), "").unwrap();
    let file = p.root.join("src/x.test.ts");
    fs::write(&file, "").unwrap();
    let inv =
        build_test_command(TestScope::File, &ctx(&file, "", 0, Language::TypeScript)).unwrap();
    assert_eq!(inv.command, "bun test 'src/x.test.ts'");
}

#[test]
fn js_node_test_nearest_runs_directly_without_a_test_script() {
    let p = js_package(r#"{"name": "demo"}"#);
    let file = p.root.join("src/clock.test.ts");
    fs::write(&file, "").unwrap();
    let src = r#"
import { describe, it } from 'node:test';
describe('FakeClock', () => {
  it('fires exactly', () => {});
});
"#;

    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, src, 3, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(
        inv.command,
        "node --test --test-name-pattern '^FakeClock fires exactly$' 'src/clock.test.ts'"
    );

    let file_scope =
        build_test_command(TestScope::File, &ctx(&file, src, 3, Language::TypeScript)).unwrap();
    assert_eq!(file_scope.command, "node --test 'src/clock.test.ts'");

    let suite =
        build_test_command(TestScope::Suite, &ctx(&file, src, 3, Language::TypeScript)).unwrap();
    assert_eq!(suite.command, "node --test");
}

#[test]
fn js_node_test_typescript_prefers_declared_tsx_runner() {
    let p = js_package(r#"{"devDependencies": {"tsx": "^4.0.0"}}"#);
    let file = p.root.join("src/clock.test.ts");
    fs::write(&file, "").unwrap();
    let src = "import { it } from 'node:test';\nit('ticks', () => {});\n";

    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, src, 1, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(
        inv.command,
        "npx tsx --test --test-name-pattern '^ticks$' 'src/clock.test.ts'"
    );
}

#[test]
fn js_node_test_preserves_the_package_test_script() {
    let p = js_package(r#"{"name": "demo", "scripts": {"test": "node --import tsx --test"}}"#);
    let file = p.root.join("src/clock.test.ts");
    fs::write(&file, "").unwrap();
    let src = "import { it } from 'node:test';\nit('ticks', () => {});\n";

    let nearest = build_test_command(
        TestScope::Nearest,
        &ctx(&file, src, 1, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(
        nearest.command,
        "npm test -- --test-name-pattern '^ticks$' 'src/clock.test.ts'"
    );

    let file_scope =
        build_test_command(TestScope::File, &ctx(&file, src, 1, Language::TypeScript)).unwrap();
    assert_eq!(file_scope.command, "npm test -- 'src/clock.test.ts'");

    let suite =
        build_test_command(TestScope::Suite, &ctx(&file, src, 1, Language::TypeScript)).unwrap();
    assert_eq!(suite.command, "npm test");
}

#[test]
fn js_node_test_does_not_forward_args_to_an_unrelated_test_script() {
    let p = js_package(r#"{"name": "demo", "scripts": {"test": "./custom-runner.sh"}}"#);
    let file = p.root.join("src/clock.test.ts");
    fs::write(&file, "").unwrap();
    let src = "import { it } from 'node:test';\nit('ticks', () => {});\n";

    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, src, 1, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(
        inv.command,
        "node --test --test-name-pattern '^ticks$' 'src/clock.test.ts'"
    );
}

#[test]
fn js_node_test_is_not_selected_from_a_comment() {
    let p = js_package(r#"{"devDependencies": {"vitest": "^2.0.0"}}"#);
    let file = p.root.join("src/x.test.ts");
    fs::write(&file, "").unwrap();
    let src = "// node:test is used elsewhere\nit('works', () => {});\n";

    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, src, 1, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(inv.command, "npx vitest run -t '^works$' 'src/x.test.ts'");
}

#[test]
fn js_npm_fallback_when_no_runner_found() {
    let p = js_package(r#"{"name": "demo", "scripts": {"test": "./custom-runner.sh"}}"#);
    let file = p.root.join("src/x.test.ts");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, JS_SRC, 2, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(inv.command, "npm test");
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

const PY_SRC: &str = r#"
import pytest

class TestMath:
    def test_addition(self):
        assert 1 + 1 == 2

    class TestNested:
        def test_deep(self):
            pass

@pytest.mark.parametrize("n", [1, 2])
def test_param(n):
    assert n > 0
"#;

#[test]
fn python_nearest_builds_node_id() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    fs::create_dir_all(root.join("tests")).unwrap();
    let file = root.join("tests/test_math.py");
    fs::write(&file, "").unwrap();

    let inv =
        build_test_command(TestScope::Nearest, &ctx(&file, PY_SRC, 8, Language::Python)).unwrap();
    assert_eq!(
        inv.command,
        "python3 -m pytest 'tests/test_math.py::TestMath::TestNested::test_deep'"
    );
    assert_eq!(inv.cwd, root);
}

#[test]
fn python_decorated_test_selected_from_decorator_line() {
    let tests = discover_tests(Language::Python, PY_SRC);
    // Cursor on the @pytest.mark.parametrize line (0-indexed 11).
    let t = nearest_test(&tests, 11).unwrap();
    assert_eq!(t.name, "test_param");
}

#[test]
fn python_uv_lock_prefixes_runner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("pyproject.toml"), "").unwrap();
    fs::write(root.join("uv.lock"), "").unwrap();
    let file = root.join("test_x.py");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::Suite, &ctx(&file, "", 0, Language::Python)).unwrap();
    assert_eq!(inv.command, "uv run pytest");
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

const GO_SRC: &str = r#"package math

import "testing"

func helper() {}

func TestAdd(t *testing.T) {
    t.Run("with negative numbers", func(t *testing.T) {
        // ...
    })
}

func TestTable(t *testing.T) {
    cases := []struct {
        name string
        n    int
    }{
        {name: "adds two numbers", n: 2},
        {name: "handles (parens)", n: 3},
    }
    for _, tc := range cases {
        t.Run(tc.name, func(t *testing.T) {})
    }
}
"#;

struct GoProject {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn go_module() -> GoProject {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("go.mod"), "module example.com/demo\n").unwrap();
    fs::create_dir_all(root.join("math")).unwrap();
    GoProject { _dir: dir, root }
}

#[test]
fn go_subtest_run_pattern_with_underscores() {
    let p = go_module();
    let file = p.root.join("math/add_test.go");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::Nearest, &ctx(&file, GO_SRC, 8, Language::Go)).unwrap();
    assert_eq!(
        inv.command,
        "go test -run '^TestAdd$/^with_negative_numbers$' './math'"
    );
    assert_eq!(inv.cwd, p.root);
}

#[test]
fn go_table_entry_is_unanchored_and_escaped() {
    // vim-test's escape set misses braces/backslash; ours escapes parens too.
    let p = go_module();
    let file = p.root.join("math/add_test.go");
    fs::write(&file, "").unwrap();
    let inv =
        build_test_command(TestScope::Nearest, &ctx(&file, GO_SRC, 18, Language::Go)).unwrap();
    assert_eq!(
        inv.command,
        r"go test -run '^TestTable$/handles_\(parens\)' './math'"
    );
}

#[test]
fn go_file_scope_unions_top_level_tests() {
    // Tighter than vim-test, which runs the whole package for file scope.
    let p = go_module();
    let file = p.root.join("math/add_test.go");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::File, &ctx(&file, GO_SRC, 0, Language::Go)).unwrap();
    assert_eq!(inv.command, "go test -run '^(TestAdd|TestTable)$' './math'");
}

#[test]
fn go_suite_runs_all_packages() {
    let p = go_module();
    let file = p.root.join("math/add_test.go");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::Suite, &ctx(&file, "", 0, Language::Go)).unwrap();
    assert_eq!(inv.command, "go test ./...");
}

// ---------------------------------------------------------------------------
// Config templates
// ---------------------------------------------------------------------------

#[test]
fn config_template_substitutes_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("mix.exs"), "").unwrap();
    fs::create_dir_all(root.join("test")).unwrap();
    let file = root.join("test/demo_test.exs");
    fs::write(&file, "").unwrap();

    let cfg = TestConfig {
        suite_command: Some("mix test".into()),
        file_command: Some("mix test {file}".into()),
        nearest_command: Some("mix test {file}:{line}".into()),
        root_markers: vec!["mix.exs".into()],
    };
    let ctx = TestContext {
        file: &file,
        source: "",
        cursor_line: 41,
        language: None,
        config: Some(&cfg),
    };
    let inv = build_test_command(TestScope::Nearest, &ctx).unwrap();
    assert_eq!(inv.command, "mix test 'test/demo_test.exs':42");
    assert_eq!(inv.cwd, root);
}

#[test]
fn no_runner_yields_helpful_error() {
    let file = PathBuf::from("/tmp/nonexistent.xyz");
    let ctx = TestContext {
        file: &file,
        source: "",
        cursor_line: 0,
        language: None,
        config: None,
    };
    let err = build_test_command(TestScope::File, &ctx).unwrap_err();
    assert!(err.contains("[language.test]"));
}

// ---------------------------------------------------------------------------
// Review regression tests
// ---------------------------------------------------------------------------

#[test]
fn js_plain_test_with_printf_lookalike_stays_exact() {
    // A literal %d in a plain (non-.each) title is part of the runtime name;
    // it must stay anchored, not get truncated as if parameterized.
    let src = "it('processes %d items successfully', () => {});\n";
    let p = js_package(r#"{"devDependencies": {"vitest": "^2.0.0"}}"#);
    let file = p.root.join("src/x.test.ts");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, src, 0, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(
        inv.command,
        "npx vitest run -t '^processes %d items successfully$' 'src/x.test.ts'"
    );
}

#[test]
fn js_describe_each_makes_nested_tests_parameterized() {
    // Tests under describe.each('group %s') have runtime names like
    // "group a does something" — an anchored filter containing the literal
    // %s would match zero tests.
    let src =
        "describe.each(['a', 'b'])('group %s', (l) => {\n  it('does something', () => {});\n});\n";
    let p = js_package(r#"{"devDependencies": {"vitest": "^2.0.0"}}"#);
    let file = p.root.join("src/x.test.ts");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(
        TestScope::Nearest,
        &ctx(&file, src, 1, Language::TypeScript),
    )
    .unwrap();
    assert_eq!(inv.command, "npx vitest run -t 'group' 'src/x.test.ts'");
}

#[test]
fn go_struct_literal_name_field_is_not_a_test() {
    // `Person{name: "Alice"}` inside a test without table-driven t.Run
    // usage must not become a phantom subtest.
    let src = "package x\n\nimport \"testing\"\n\nfunc TestFoo(t *testing.T) {\n    p := Person{name: \"Alice\"}\n    _ = p\n}\n";
    let tests = discover_tests(Language::Go, src);
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].name, "TestFoo");
}

#[test]
fn go_package_dir_with_space_is_quoted() {
    let p = go_module();
    fs::create_dir_all(p.root.join("my pkg")).unwrap();
    let file = p.root.join("my pkg/add_test.go");
    fs::write(&file, "").unwrap();
    let src = "package x\n\nimport \"testing\"\n\nfunc TestAdd(t *testing.T) {}\n";
    let inv = build_test_command(TestScope::File, &ctx(&file, src, 0, Language::Go)).unwrap();
    assert_eq!(inv.command, "go test -run '^(TestAdd)$' './my pkg'");
}

#[test]
fn rust_workspace_detected_from_dotted_table_header() {
    // A workspace root manifest that only writes [workspace.package] (valid
    // TOML, implicit parent table) still marks a workspace.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().canonicalize().unwrap();
    fs::write(
        ws.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(ws.join("demo/src")).unwrap();
    fs::write(
        ws.join("demo/Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let file = ws.join("demo/src/lib.rs");
    fs::write(&file, "").unwrap();
    let inv = build_test_command(TestScope::Suite, &ctx(&file, "", 0, Language::Rust)).unwrap();
    assert_eq!(inv.command, "cargo test --workspace");
    assert_eq!(inv.cwd, ws);
}

// ---------------------------------------------------------------------------
// Java / Kotlin
// ---------------------------------------------------------------------------

const JAVA_SRC: &str = r#"package com.example.app;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Nested;
import org.junit.jupiter.params.ParameterizedTest;

class CalcTest {
    @Test
    void adds() {
        assertEquals(2, 1 + 1);
    }

    @ParameterizedTest
    @ValueSource(ints = {1, 2})
    void squares(int n) {}

    void helper() {}

    @Nested
    class Inner {
        @org.junit.jupiter.api.Test
        void nestedCase() {}
    }
}
"#;

const KOTLIN_SRC: &str = r#"package com.example.app

import org.junit.jupiter.api.Test

class KCalcTest {
    @Test
    fun `adds two numbers`() {
        check(1 + 1 == 2)
    }

    @Test fun plain() {}

    fun helper() {}
}
"#;

#[test]
fn java_discovery_finds_test_methods_and_nested_classes() {
    let tests = discover_tests(Language::Java, JAVA_SRC);
    let found: Vec<(String, Vec<String>, TestFlavor)> = tests
        .iter()
        .map(|t| (t.name.clone(), t.namespaces.clone(), t.flavor))
        .collect();
    assert_eq!(
        found,
        vec![
            ("adds".into(), vec!["CalcTest".into()], TestFlavor::Exact),
            (
                "squares".into(),
                vec!["CalcTest".into()],
                TestFlavor::Parameterized
            ),
            (
                "nestedCase".into(),
                vec!["CalcTest".into(), "Inner".into()],
                TestFlavor::Exact
            ),
        ]
    );
}

#[test]
fn kotlin_discovery_finds_backticked_and_inline_annotated_tests() {
    let tests = discover_tests(Language::Kotlin, KOTLIN_SRC);
    let names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["adds two numbers", "plain"]);
    assert_eq!(tests[0].namespaces, vec!["KCalcTest".to_string()]);
}

fn gradle_project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("settings.gradle.kts"), "include(\"app\")\n").unwrap();
    fs::create_dir_all(root.join("app/src/test/java/com/example/app")).unwrap();
    fs::write(root.join("app/build.gradle.kts"), "").unwrap();
    let file = root.join("app/src/test/java/com/example/app/CalcTest.java");
    fs::write(&file, JAVA_SRC).unwrap();
    (dir, file)
}

#[test]
fn jvm_local_plan_filters_gradle_by_nearest_method() {
    let (dir, file) = gradle_project();
    let root = dir.path().canonicalize().unwrap();
    // Cursor inside `adds`.
    let local = local_plan(TestScope::Nearest, &file, JAVA_SRC, 9, Language::Java).unwrap();
    let task = local.plan.task.as_ref().unwrap();
    assert_eq!(
        task.argv,
        vec![
            "gradle",
            ":app:cleanTest",
            ":app:test",
            "--tests",
            "com.example.app.CalcTest.adds",
            "--console=plain"
        ]
    );
    assert_eq!(
        task.debug_argv.as_ref().unwrap().last().map(String::as_str),
        Some("--debug-jvm")
    );
    assert_eq!(task.cwd, root);
    assert_eq!(
        task.reports_dir,
        Some(root.join("app/build/test-results/test"))
    );
    assert!(local.cursor_inside);
    assert_eq!(local.anchor.0, 7, "anchored on the @Test line of `adds`");

    // Nested class: binary name with `$`.
    let local = local_plan(TestScope::Nearest, &file, JAVA_SRC, 20, Language::Java).unwrap();
    let argv = local.plan.task.unwrap().argv;
    assert!(argv.contains(&"com.example.app.CalcTest$Inner.nestedCase".to_string()));
}

/// OV-00448: on a class declaration line the nearest test is the class.
#[test]
fn jvm_nearest_on_a_class_declaration_runs_the_whole_class() {
    let (_dir, file) = gradle_project();
    let filters = |line: usize| -> (Vec<String>, (usize, usize), bool) {
        let local = local_plan(TestScope::Nearest, &file, JAVA_SRC, line, Language::Java).unwrap();
        let task = local.plan.task.unwrap();
        let filters = task
            .argv
            .iter()
            .skip_while(|a| *a != "--tests")
            .filter(|a| !a.starts_with("--"))
            .cloned()
            .collect();
        (filters, local.anchor, local.cursor_inside)
    };
    let class_line = JAVA_SRC
        .lines()
        .position(|l| l.contains("class CalcTest"))
        .unwrap();
    let (all, anchor, inside) = filters(class_line);
    assert_eq!(
        all,
        vec!["com.example.app.CalcTest", "com.example.app.CalcTest$Inner"],
        "the class and its nested classes, not the first method"
    );
    assert_eq!(anchor.0, class_line, "the server is asked about the class");
    assert!(!inside);

    // The nested class line runs just the nested class.
    let inner_line = JAVA_SRC
        .lines()
        .position(|l| l.contains("class Inner"))
        .unwrap();
    let (nested, _, _) = filters(inner_line);
    assert_eq!(nested, vec!["com.example.app.CalcTest$Inner"]);

    // A method line still runs that method.
    let (method, _, inside) = filters(9);
    assert_eq!(method, vec!["com.example.app.CalcTest.adds"]);
    assert!(inside);
}

#[test]
fn jvm_local_plan_file_scope_selects_every_test_class() {
    let (_dir, file) = gradle_project();
    let local = local_plan(TestScope::File, &file, JAVA_SRC, 0, Language::Java).unwrap();
    let task = local.plan.task.unwrap();
    let filters: Vec<&String> = task
        .argv
        .iter()
        .skip_while(|a| *a != "--tests")
        .filter(|a| !a.starts_with("--"))
        .collect();
    assert_eq!(
        filters,
        vec!["com.example.app.CalcTest", "com.example.app.CalcTest$Inner"]
    );
    assert_eq!(local.anchor, (6, 0), "server is asked about the class line");
}

#[test]
fn jvm_local_plan_uses_maven_with_surefire_filter_and_debug_flag() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("pom.xml"),
        "<project><modules><module>core</module></modules></project>",
    )
    .unwrap();
    fs::create_dir_all(root.join("core/src/test/java")).unwrap();
    fs::write(root.join("core/pom.xml"), "<project/>").unwrap();
    let file = root.join("core/src/test/java/CalcTest.java");
    let src = "package p;\nclass CalcTest {\n  @Test\n  void adds() {}\n}\n";
    fs::write(&file, src).unwrap();
    let local = local_plan(TestScope::Nearest, &file, src, 3, Language::Java).unwrap();
    let task = local.plan.task.unwrap();
    assert_eq!(
        task.argv,
        vec![
            "mvn",
            "-Dtest=p.CalcTest#adds",
            "-Dsurefire.failIfNoSpecifiedTests=false",
            "-DfailIfNoTests=false",
            "-pl",
            "core",
            "-am",
            "test"
        ]
    );
    assert_eq!(task.debug_argv.unwrap()[1], "-Dmaven.surefire.debug");
    assert_eq!(
        task.reports_dir,
        Some(root.join("core/target/surefire-reports"))
    );
}

#[test]
fn jvm_local_plan_reports_missing_project_and_missing_tests() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Lone.java");
    let err = local_plan(TestScope::Nearest, &file, JAVA_SRC, 0, Language::Java)
        .err()
        .unwrap();
    assert!(err.contains("No Gradle or Maven project"), "{err}");
    let (_d, file) = gradle_project();
    let err = local_plan(TestScope::Nearest, &file, "class A {}", 0, Language::Java)
        .err()
        .unwrap();
    assert_eq!(err, "No test found near cursor");
}

// ---------------------------------------------------------------------------
// JVM roots: one rule for the language server, launches and local test plans
// ---------------------------------------------------------------------------

/// A local test plan rooted where a launch would be (`launch_project_root`).
fn local_plan(
    scope: TestScope,
    file: &Path,
    source: &str,
    cursor_line: usize,
    lang: Language,
) -> Result<super::jvm::LocalTest, String> {
    let root = Editor::with_content("").launch_project_root(Some(file));
    super::jvm::local_test_plan(scope, file, source, cursor_line, lang, &root)
}

/// The root the language server is started with for `file` (Java's
/// `root_markers` / `outermost_root_markers` from languages.toml).
fn server_root(file: &Path) -> PathBuf {
    let markers: Vec<String> = [
        "settings.gradle",
        "settings.gradle.kts",
        "pom.xml",
        "build.gradle",
    ]
    .map(String::from)
    .into();
    let outermost: Vec<String> = ["settings.gradle", "settings.gradle.kts", "pom.xml"]
        .map(String::from)
        .into();
    crate::project_root::find_project_root_with_outermost(file, &markers, &outermost)
}

/// Lays out `files` (relative path, content) under a temp dir and returns
/// the dir plus the test file's path.
fn jvm_layout(files: &[(&str, &str)], test_file: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (path, content) in files {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }
    (dir, root.join(test_file))
}

const ONE_TEST: &str = "package p;\nclass CalcTest {\n  @Test\n  void adds() {}\n}\n";

/// The language server, a launch and the local test plan agree on the root
/// for every layout; the plan addresses the module relative to it.
/// Was: the plan walked up on its own, and disagreed for a Maven parent that
/// does not list the module and for a Gradle build with its own settings
/// inside another (OV-00481).
#[test]
fn jvm_root_layouts_agree_between_server_launch_and_local_plan() {
    struct Row {
        name: &'static str,
        files: Vec<(&'static str, &'static str)>,
        test_file: &'static str,
        /// Directory (relative to the layout) every one of them is rooted at.
        root: &'static str,
        /// Substrings the plan's command line must contain / not contain.
        argv_has: &'static str,
        argv_lacks: &'static str,
    }
    let test_file = "app/src/test/java/CalcTest.java";
    let rows = vec![
        Row {
            name: "single gradle project",
            files: vec![("build.gradle", "")],
            test_file: "src/test/java/CalcTest.java",
            root: "",
            argv_has: " :cleanTest :test ",
            argv_lacks: "-pl",
        },
        Row {
            name: "gradle multi-module with settings",
            files: vec![("settings.gradle", ""), ("app/build.gradle", "")],
            test_file,
            root: "",
            argv_has: " :app:cleanTest :app:test ",
            argv_lacks: "-pl",
        },
        Row {
            name: "gradle build with its own settings inside another: the outer one wins",
            files: vec![
                ("settings.gradle", ""),
                ("app/settings.gradle", ""),
                ("app/build.gradle", ""),
            ],
            test_file,
            root: "",
            argv_has: " :app:cleanTest :app:test ",
            argv_lacks: "-pl",
        },
        Row {
            name: "maven aggregator lists the module",
            files: vec![
                (
                    "pom.xml",
                    "<project><modules><module>app</module></modules></project>",
                ),
                ("app/pom.xml", "<project/>"),
            ],
            test_file,
            root: "",
            argv_has: "-pl app -am",
            argv_lacks: "zzz",
        },
        Row {
            name: "maven parent does not list the module: it is not the reactor",
            files: vec![
                (
                    "pom.xml",
                    "<project><modules><module>other</module></modules></project>",
                ),
                ("app/pom.xml", "<project/>"),
            ],
            test_file,
            root: "app",
            argv_has: "mvn",
            argv_lacks: "-pl",
        },
        // Known gap (OV-00481): the reactor rule climbs one directory at a
        // time, so an aggregator listing `mid/app` is not found.
        Row {
            name: "maven aggregator lists a nested module path",
            files: vec![
                (
                    "pom.xml",
                    "<project><modules><module>mid/app</module></modules></project>",
                ),
                ("mid/app/pom.xml", "<project/>"),
            ],
            test_file: "mid/app/src/test/java/CalcTest.java",
            root: "mid/app",
            argv_has: "mvn",
            argv_lacks: "-pl",
        },
    ];
    for row in rows {
        let mut files = row.files.clone();
        files.push((row.test_file, ONE_TEST));
        let (dir, file) = jvm_layout(&files, row.test_file);
        let base = dir.path().canonicalize().unwrap();
        let expected = if row.root.is_empty() {
            base.clone()
        } else {
            base.join(row.root)
        };
        assert_eq!(server_root(&file), expected, "server root: {}", row.name);
        assert_eq!(
            Editor::with_content("").launch_project_root(Some(&file)),
            expected,
            "launch root: {}",
            row.name
        );
        let plan = local_plan(TestScope::Nearest, &file, ONE_TEST, 3, Language::Java)
            .unwrap_or_else(|e| panic!("{}: {e}", row.name))
            .plan;
        assert_eq!(plan.project_root, expected, "plan root: {}", row.name);
        let argv = format!("{} ", plan.task.unwrap().argv.join(" "));
        assert!(argv.contains(row.argv_has), "{}: {argv}", row.name);
        assert!(!argv.contains(row.argv_lacks), "{}: {argv}", row.name);
    }
}
