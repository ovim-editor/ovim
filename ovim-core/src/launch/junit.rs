//! JUnit XML results (`build/test-results/test/*.xml`, Surefire's
//! `target/surefire-reports/*.xml`), read after a test run.
//!
//! Only what the test panel and quickfix need: one entry per `<testcase>`
//! with its outcome, failure message and stack text. The scanner is a small
//! purpose-built tag walker rather than a general XML parser: these files are
//! machine-written and flat.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseStatus {
    Passed,
    Failed,
    Errored,
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestCaseResult {
    pub class_name: String,
    pub name: String,
    pub status: CaseStatus,
    /// `message` attribute of `<failure>` / `<error>`.
    pub message: Option<String>,
    /// Body of `<failure>` / `<error>` (usually the stack trace).
    pub details: Option<String>,
    pub seconds: f64,
}

/// Totals over a set of cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JunitTotals {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

pub fn totals(cases: &[TestCaseResult]) -> JunitTotals {
    let mut t = JunitTotals::default();
    for case in cases {
        match case.status {
            CaseStatus::Passed => t.passed += 1,
            CaseStatus::Failed | CaseStatus::Errored => t.failed += 1,
            CaseStatus::Skipped => t.skipped += 1,
        }
    }
    t
}

/// `12 passed, 1 failed, 2 skipped`.
pub fn summary_text(cases: &[TestCaseResult]) -> Option<String> {
    if cases.is_empty() {
        return None;
    }
    let t = totals(cases);
    let mut parts = vec![format!("{} passed", t.passed)];
    if t.failed > 0 {
        parts.push(format!("{} failed", t.failed));
    }
    if t.skipped > 0 {
        parts.push(format!("{} skipped", t.skipped));
    }
    Some(parts.join(", "))
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else { break };
        let entity = &rest[1..semi];
        let replacement = match entity {
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "amp" => Some("&".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" => Some("'".to_string()),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .map(String::from),
            e if e.starts_with('#') => e[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(String::from),
            _ => None,
        };
        match replacement {
            Some(r) => {
                out.push_str(&r);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Attributes of one tag's inside text, e.g. ` name="a" time='1.2'`.
fn attributes(tag_body: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut rest = tag_body;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq]
            .trim()
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or("");
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let Some(end) = after[1..].find(quote) else {
            break;
        };
        attrs.push((key.to_string(), unescape(&after[1..1 + end])));
        rest = &after[end + 2..];
    }
    attrs
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Parses one JUnit XML document.
pub fn parse_junit_xml(xml: &str) -> Vec<TestCaseResult> {
    let mut cases = Vec::new();
    let mut current: Option<TestCaseResult> = None;
    // Which failure-ish element's text we are collecting.
    let mut collecting: Option<CaseStatus> = None;
    let mut text = String::new();
    let mut pos = 0;

    while pos < xml.len() {
        let Some(lt) = xml[pos..].find('<') else {
            break;
        };
        if collecting.is_some() {
            text.push_str(&unescape(&xml[pos..pos + lt]));
        }
        pos += lt;
        let rest = &xml[pos..];

        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>").unwrap_or(after.len());
            if collecting.is_some() {
                text.push_str(&after[..end]);
            }
            pos += "<![CDATA[".len() + end + 3;
            continue;
        }
        if rest.starts_with("<!--") {
            pos += rest.find("-->").map(|i| i + 3).unwrap_or(rest.len());
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        pos += gt + 1;
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let inner = tag.trim_start_matches('/').trim_end_matches('/');
        let name = inner.split_whitespace().next().unwrap_or("");
        let attrs = attributes(&inner[name.len()..]);

        match (name, closing) {
            ("testcase", false) => {
                let case = TestCaseResult {
                    class_name: attr(&attrs, "classname").unwrap_or_default().to_string(),
                    name: attr(&attrs, "name").unwrap_or_default().to_string(),
                    status: CaseStatus::Passed,
                    message: None,
                    details: None,
                    seconds: attr(&attrs, "time")
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(0.0),
                };
                if self_closing {
                    cases.push(case);
                } else {
                    current = Some(case);
                }
            }
            ("testcase", true) => {
                if let Some(case) = current.take() {
                    cases.push(case);
                }
            }
            ("failure" | "error", false) => {
                let status = if name == "failure" {
                    CaseStatus::Failed
                } else {
                    CaseStatus::Errored
                };
                if let Some(case) = current.as_mut() {
                    case.status = status;
                    let ty = attr(&attrs, "type").unwrap_or_default();
                    let msg = attr(&attrs, "message").unwrap_or_default();
                    // Gradle already writes `message="<type>: <text>"`.
                    let msg_has_type = !ty.is_empty() && msg.starts_with(ty);
                    case.message = Some(match (ty.is_empty() || msg_has_type, msg.is_empty()) {
                        (false, false) => format!("{ty}: {msg}"),
                        (false, true) => ty.to_string(),
                        _ => msg.to_string(),
                    })
                    .filter(|m| !m.is_empty());
                }
                if !self_closing {
                    collecting = Some(status);
                    text.clear();
                }
            }
            ("failure" | "error", true) => {
                if let (Some(case), Some(_)) = (current.as_mut(), collecting.take()) {
                    let body = text.trim().to_string();
                    if !body.is_empty() {
                        case.details = Some(body);
                    }
                }
            }
            ("skipped", false) => {
                if let Some(case) = current.as_mut() {
                    case.status = CaseStatus::Skipped;
                    case.message = attr(&attrs, "message").map(str::to_string);
                }
            }
            _ => {}
        }
    }
    if let Some(case) = current.take() {
        cases.push(case);
    }
    cases
}

/// Reads every `*.xml` in `dir` modified at or after `since` (results from
/// earlier runs are stale) and returns all test cases, ordered by file name.
/// JUnit 5 reports a parameterized invocation by its display name
/// (`[1] 5`), which drops the method name. Put it back: from the method the
/// run was limited to, else from a stack frame of the failure that names one
/// of the class's parameterized methods, else when the class has exactly one.
/// `parameterized_methods(class)` lists the class's parameterized test
/// methods as found in its source; it is asked once per class.
pub fn restore_parameterized_names(
    cases: &mut [TestCaseResult],
    method_hint: Option<&str>,
    mut parameterized_methods: impl FnMut(&str) -> Vec<String>,
) {
    let mut by_class: HashMap<String, Vec<String>> = HashMap::new();
    for case in cases.iter_mut().filter(|c| c.name.starts_with('[')) {
        let candidates = by_class
            .entry(case.class_name.clone())
            .or_insert_with_key(|class| parameterized_methods(class));
        let from_frame = case.details.as_deref().and_then(|details| {
            let prefix = format!("{}.", case.class_name);
            details.lines().find_map(|line| {
                let frame = line.trim().strip_prefix("at ")?.strip_prefix(&prefix)?;
                let method = frame.split('(').next()?;
                candidates
                    .iter()
                    .any(|c| c == method)
                    .then(|| method.to_string())
            })
        });
        let method = method_hint
            .map(str::to_string)
            .or(from_frame)
            .or_else(|| (candidates.len() == 1).then(|| candidates[0].clone()));
        if let Some(method) = method {
            case.name = format!("{method} {}", case.name);
        }
    }
}

pub fn read_reports_dir(dir: &Path, since: SystemTime) -> Vec<TestCaseResult> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xml"))
        .filter(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .is_ok_and(|modified| modified >= since)
        })
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|xml| parse_junit_xml(&xml))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(class: &str, name: &str, details: Option<&str>) -> TestCaseResult {
        TestCaseResult {
            class_name: class.into(),
            name: name.into(),
            status: CaseStatus::Passed,
            message: None,
            details: details.map(str::to_string),
            seconds: 0.0,
        }
    }

    /// OV-00449: parameterized invocations keep their method name.
    #[test]
    fn parameterized_invocations_get_their_method_name_back() {
        let methods = |_: &str| vec!["totals".to_string(), "sizes".to_string()];
        let mut cases = vec![
            case("p.OrdersTest", "[1] 1", None),
            case("p.OrdersTest", "plain()", None),
            case(
                "p.OrdersTest",
                "[2] 5",
                Some("AssertionError\n\tat p.OrdersTest.helper(OrdersTest.java:5)\n\tat p.OrdersTest.sizes(OrdersTest.java:9)"),
            ),
        ];
        // Two parameterized methods and no other hint: only the stack tells.
        restore_parameterized_names(&mut cases, None, methods);
        assert_eq!(cases[0].name, "[1] 1", "ambiguous stays as reported");
        assert_eq!(cases[1].name, "plain()");
        assert_eq!(cases[2].name, "sizes [2] 5");

        // A single candidate, or the method the run was limited to, is enough.
        let mut cases = vec![case("p.T", "[1] a", None)];
        restore_parameterized_names(&mut cases, None, |_| vec!["only".to_string()]);
        assert_eq!(cases[0].name, "only [1] a");
        let mut cases = vec![case("p.T", "[1] a", None)];
        restore_parameterized_names(&mut cases, Some("picked"), |_| vec![]);
        assert_eq!(cases[0].name, "picked [1] a");
    }

    #[test]
    fn a_class_source_is_consulted_once_however_many_cases_it_has() {
        let mut cases: Vec<TestCaseResult> = (0..50)
            .map(|i| case("p.T", &format!("[{i}] x"), None))
            .collect();
        cases.push(case("p.U", "[1] x", None));
        let mut asked = Vec::new();
        restore_parameterized_names(&mut cases, None, |class| {
            asked.push(class.to_string());
            vec!["only".to_string()]
        });
        assert_eq!(asked, vec!["p.T", "p.U"]);
        assert_eq!(cases[49].name, "only [49] x");
    }

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="com.example.FooTest" tests="4" skipped="1" failures="1" errors="0">
  <properties/>
  <testcase name="passes()" classname="com.example.FooTest" time="0.012"/>
  <testcase name="fails()" classname="com.example.FooTest" time="0.5">
    <failure message="expected: &lt;2&gt; but was: &lt;3&gt;" type="org.opentest4j.AssertionFailedError">org.opentest4j.AssertionFailedError: expected: &lt;2&gt; but was: &lt;3&gt;
	at com.example.FooTest.fails(FooTest.java:14)
</failure>
  </testcase>
  <testcase name="skips()" classname="com.example.FooTest" time="0"><skipped/></testcase>
  <testcase name="boom()" classname="com.example.FooTest" time="0.1"><error message="npe" type="java.lang.NullPointerException"><![CDATA[java.lang.NullPointerException
	at com.example.Foo.run(Foo.java:3)]]></error></testcase>
  <system-out><![CDATA[noise <not xml>]]></system-out>
</testsuite>"#;

    #[test]
    fn parses_outcomes_messages_and_stack_text() {
        let cases = parse_junit_xml(SAMPLE);
        assert_eq!(cases.len(), 4);
        assert_eq!(cases[0].status, CaseStatus::Passed);
        assert_eq!(cases[0].name, "passes()");
        assert_eq!(cases[1].status, CaseStatus::Failed);
        assert_eq!(
            cases[1].message.as_deref(),
            Some("org.opentest4j.AssertionFailedError: expected: <2> but was: <3>")
        );
        assert!(cases[1]
            .details
            .as_ref()
            .unwrap()
            .contains("FooTest.java:14"));
        assert_eq!(cases[2].status, CaseStatus::Skipped);
        assert_eq!(cases[3].status, CaseStatus::Errored);
        assert!(cases[3].details.as_ref().unwrap().contains("Foo.java:3"));
        assert!((cases[1].seconds - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_message_that_already_names_its_type_is_not_prefixed_twice() {
        let xml = r#"<testsuite><testcase name="t()" classname="C" time="0">
<failure message="org.opentest4j.AssertionFailedError: expected: &lt;3&gt; but was: &lt;2&gt;" type="org.opentest4j.AssertionFailedError">stack</failure></testcase></testsuite>"#;
        let cases = parse_junit_xml(xml);
        assert_eq!(
            cases[0].message.as_deref(),
            Some("org.opentest4j.AssertionFailedError: expected: <3> but was: <2>")
        );
    }

    #[test]
    fn summary_counts_errors_as_failures() {
        let cases = parse_junit_xml(SAMPLE);
        assert_eq!(
            summary_text(&cases).as_deref(),
            Some("1 passed, 2 failed, 1 skipped")
        );
        assert_eq!(summary_text(&[]), None);
    }

    #[test]
    fn reads_only_reports_written_since_the_run_started() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("TEST-old.xml"), SAMPLE).unwrap();
        let cutoff = SystemTime::now() + std::time::Duration::from_secs(1);
        assert!(read_reports_dir(dir.path(), cutoff).is_empty());
        let epoch = SystemTime::UNIX_EPOCH;
        assert_eq!(read_reports_dir(dir.path(), epoch).len(), 4);
        assert!(read_reports_dir(&dir.path().join("missing"), epoch).is_empty());
    }
}
