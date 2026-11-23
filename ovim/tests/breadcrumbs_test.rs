//! Breadcrumbs (enclosing class › method) and the outline picker, from the
//! tree-sitter fallback (no language server involved).

mod helpers;

use helpers::EditorTest;
use ovim_core::Mode;
use std::fs;

const JAVA: &str = "package shapes;

public class Circle {
    private double radius;

    public Circle(double radius) {
        this.radius = radius;
    }

    public double area() {
        return Math.PI * radius * radius;
    }

    static class Builder {
        Circle build() {
            return new Circle(1.0);
        }
    }
}
";

async fn open(dir: &tempfile::TempDir, name: &str, content: &str) -> EditorTest {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    let mut test = EditorTest::new("");
    test.load_file(&path.to_string_lossy());
    test.editor.buffer_mut().enable_syntax_highlighting();
    test.editor.request_outline_if_needed().await;
    test
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn java_cursor_inside_a_method_shows_class_and_method() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = open(&dir, "Circle.java", JAVA).await;
    test.set_cursor(10, 8);
    assert_eq!(test.editor.breadcrumb_text(), "Circle › area");
    test.set_cursor(15, 8);
    assert_eq!(test.editor.breadcrumb_text(), "Circle › Builder › build");
    test.set_cursor(6, 8);
    assert_eq!(
        test.editor.breadcrumb_text(),
        "Circle › Circle",
        "the constructor"
    );
    test.set_cursor(3, 8);
    assert_eq!(
        test.editor.breadcrumb_text(),
        "Circle",
        "a field is only inside the class"
    );
    test.set_cursor(0, 0);
    assert_eq!(
        test.editor.breadcrumb_text(),
        "",
        "the package line is outside every symbol"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn breadcrumbs_follow_the_current_buffer_and_clear_for_unknown_languages() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = open(
        &dir,
        "lib.rs",
        "struct Point;\n\nimpl Point {\n    fn len(&self) -> f64 {\n        0.0\n    }\n}\n",
    )
    .await;
    test.set_cursor(4, 8);
    assert_eq!(test.editor.breadcrumb_text(), "Point › len");

    let other = dir.path().join("notes.txt");
    fs::write(&other, "hello\n").unwrap();
    test.load_file(&other.to_string_lossy());
    test.editor.request_outline_if_needed().await;
    assert_eq!(test.editor.breadcrumb_text(), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn outline_picker_lists_the_tree_with_indentation_and_jumps_to_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = open(&dir, "Circle.java", JAVA).await;
    test.keys(" o");
    test.assert_mode(Mode::Picker);
    let picker = test.editor.picker().unwrap();
    assert_eq!(picker.title(), Some("Outline"));
    let rows: Vec<String> = picker
        .collect_filtered_results(50)
        .into_iter()
        .map(|r| r.display.clone())
        .collect();
    assert_eq!(
        rows,
        vec![
            "Circle  class :3",
            "  Circle  constructor :6",
            "  area  method :10",
            "  Builder  class :14",
            "    build  method :15",
        ]
    );
    // Enter on "area" jumps to the method name.
    test.type_text("area");
    test.editor.apply_pending_picker_filter(0);
    let picker = test.editor.picker_mut().unwrap();
    picker.apply_pending_filter();
    test.press_enter();
    assert_eq!(test.cursor(), (9, 18));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn outline_picker_reports_files_without_symbols() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = open(&dir, "notes.txt", "just text\n").await;
    test.command("Outline");
    test.assert_mode(Mode::Normal);
    assert!(test.editor.status_message().contains("No symbols"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn kotlin_classes_objects_and_functions_are_recognised() {
    let dir = tempfile::tempdir().unwrap();
    let mut test = open(
        &dir,
        "Shapes.kt",
        "package shapes\n\nclass Square(val side: Double) {\n    fun area(): Double {\n        return side * side\n    }\n}\n\nobject Registry {\n    fun all() = 1\n}\n",
    )
    .await;
    test.set_cursor(4, 8);
    assert_eq!(test.editor.breadcrumb_text(), "Square › area");
    test.set_cursor(9, 4);
    assert_eq!(test.editor.breadcrumb_text(), "Registry › all");
}
