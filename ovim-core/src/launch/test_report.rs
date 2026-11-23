//! What a finished JUnit test run reports, prepared away from the editor
//! thread: reading and parsing the report files, putting parameterized
//! method names back from the test sources, and locating the project files of
//! the failures' stack frames. Large projects make all of that slow.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::junit::{self, CaseStatus, TestCaseResult};
use super::stacktrace::{self, FrameResolver};

/// A failed or errored test with the project locations of its stack, in
/// stack order, as (file, 1-based line, 1-based column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedTest {
    /// `Class.method`
    pub label: String,
    pub message: String,
    pub frames: Vec<(PathBuf, usize, usize)>,
}

#[derive(Debug, Clone, Default)]
pub struct TestReport {
    pub cases: Vec<TestCaseResult>,
    /// `12 passed, 1 failed`; `None` when nothing was reported.
    pub summary: Option<String>,
    pub failures: Vec<FailedTest>,
}

/// Reads the reports in `dir` written at or after `since`. `roots` are the
/// source roots of the run; `method_hint` is the test method the run was
/// limited to.
pub fn build(
    dir: &Path,
    since: SystemTime,
    roots: &[PathBuf],
    method_hint: Option<&str>,
) -> TestReport {
    let mut cases = junit::read_reports_dir(dir, since);
    let mut resolver = FrameResolver::new(roots.to_vec());
    junit::restore_parameterized_names(&mut cases, method_hint, |class| {
        parameterized_methods_in_source(class, &mut resolver)
    });
    let summary = junit::summary_text(&cases);
    let failures = if summary.is_some() {
        cases
            .iter()
            .filter(|c| matches!(c.status, CaseStatus::Failed | CaseStatus::Errored))
            .map(|case| FailedTest {
                label: format!("{}.{}", case.class_name, case.name),
                message: case
                    .message
                    .clone()
                    .unwrap_or_else(|| "test failed".to_string()),
                frames: case
                    .details
                    .as_deref()
                    .into_iter()
                    .flat_map(str::lines)
                    .filter_map(stacktrace::parse_console_location)
                    .filter_map(|location| resolver.resolve_location(&location, dir))
                    .collect(),
            })
            .collect()
    } else {
        Vec::new()
    };
    TestReport {
        cases,
        summary,
        failures,
    }
}

/// The parameterized test methods of `class` (binary name, `$` for nested
/// classes) as found in its source file.
fn parameterized_methods_in_source(class: &str, resolver: &mut FrameResolver) -> Vec<String> {
    use crate::editor::test_runner::nearest::{discover_tests, TestFlavor};
    let (package_class, chain) = match class.split_once('$') {
        Some((outer, nested)) => (
            outer,
            std::iter::once(outer.rsplit('.').next().unwrap_or(outer))
                .chain(nested.split('$'))
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ),
        None => (
            class,
            vec![class.rsplit('.').next().unwrap_or(class).to_string()],
        ),
    };
    let simple = package_class.rsplit('.').next().unwrap_or(package_class);
    for (ext, language) in [
        ("java", crate::syntax::Language::Java),
        ("kt", crate::syntax::Language::Kotlin),
    ] {
        let file = format!("{simple}.{ext}");
        let Some(path) = resolver.resolve_frame(package_class, &file) else {
            continue;
        };
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        return discover_tests(language, &source)
            .into_iter()
            .filter(|t| t.flavor == TestFlavor::Parameterized && t.namespaces == chain)
            .map(|t| t.name)
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterized_methods_are_read_from_the_class_source_including_nested_classes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let pkg = root.join("src/test/java/p");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("OrdersTest.java"),
            "package p;\nclass OrdersTest {\n  @Test void plain() {}\n  @ParameterizedTest @ValueSource(ints = {1}) void totals(int n) {}\n  @Nested class Empty {\n    @ParameterizedTest @ValueSource(ints = {1}) void zero(int n) {}\n  }\n}\n",
        )
        .unwrap();
        let mut resolver = FrameResolver::new(vec![root]);
        assert_eq!(
            parameterized_methods_in_source("p.OrdersTest", &mut resolver),
            vec!["totals"]
        );
        assert_eq!(
            parameterized_methods_in_source("p.OrdersTest$Empty", &mut resolver),
            vec!["zero"]
        );
        assert!(parameterized_methods_in_source("p.Missing", &mut resolver).is_empty());
    }

    #[test]
    fn failures_carry_the_project_frames_of_their_stack_and_skip_library_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let source = root.join("odd/layout/com/example/FooTest.java");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "class FooTest {}\n").unwrap();
        let reports = root.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        let failing = |name: &str| {
            format!(
                "<testcase name=\"{name}()\" classname=\"com.example.FooTest\" time=\"0.1\">\
                 <failure message=\"boom\" type=\"AssertionError\">AssertionError\n\
                 \tat org.junit.Assert.fail(Assert.java:87)\n\
                 \tat com.vendor.Thing.run(Thing.java:3)\n\
                 \tat com.example.FooTest.{name}(FooTest.java:12)\n\
                 </failure></testcase>"
            )
        };
        std::fs::write(
            reports.join("TEST-com.example.FooTest.xml"),
            format!(
                "<testsuite name=\"com.example.FooTest\">\
                 <testcase name=\"ok()\" classname=\"com.example.FooTest\" time=\"0.1\"/>{}{}\
                 </testsuite>",
                failing("a"),
                failing("b")
            ),
        )
        .unwrap();

        let report = build(
            &reports,
            SystemTime::UNIX_EPOCH,
            std::slice::from_ref(&root),
            None,
        );
        assert_eq!(report.summary.as_deref(), Some("1 passed, 2 failed"));
        assert_eq!(report.cases.len(), 3);
        assert_eq!(report.failures.len(), 2);
        assert_eq!(report.failures[0].label, "com.example.FooTest.a()");
        assert_eq!(report.failures[0].message, "AssertionError: boom");
        assert_eq!(report.failures[1].frames, vec![(source, 12, 1)]);
    }
}
