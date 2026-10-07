//! Jumpable locations in run-console output.
//!
//! Two shapes are recognised:
//!
//! - JVM stack frames: `at com.foo.Bar$Inner.baz(Bar.java:42)`, including the
//!   module prefix (`java.base/java.util.Objects.requireNonNull(Objects.java:233)`)
//!   and Kotlin frames (`at com.foo.MainKt.main(Main.kt:5)`).
//! - Compiler diagnostics that carry a file path: `Foo.java:12: error: ...`,
//!   `e: file:///abs/Foo.kt:12:5 ...`, `[ERROR] /abs/Foo.java:[12,5] ...`.
//!
//! Frames name a class and a bare file name, not a path, so they are resolved
//! against the project by looking for `<package path>/<file>` under the
//! project root (honouring `.gitignore`, which skips `build/` and `target/`).
//! A [`FrameResolver`] answers any number of frames with one walk of the tree.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

static FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*at\s+(?:[\w.$\-@]+/)*(?P<class>[\w.$]+)\.(?P<method><?[\w$\-]+>?)\((?P<file>[^():]+\.(?:java|kt|kts|scala|groovy)):(?P<line>\d+)\)",
    )
    .unwrap()
});

/// A location a console line points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleLocation {
    /// A JVM stack frame; needs source-root resolution.
    Frame {
        class: String,
        method: String,
        file: String,
        line: usize,
    },
    /// A path (possibly relative to the run's working directory).
    File {
        path: PathBuf,
        line: usize,
        column: usize,
    },
}

/// Parses a console line into a jumpable location, if it has one.
pub fn parse_console_location(line: &str) -> Option<ConsoleLocation> {
    let cleaned = super::diagnostics::strip_ansi(line);
    let line = cleaned.as_ref();
    if let Some(caps) = FRAME.captures(line) {
        return Some(ConsoleLocation::Frame {
            class: caps["class"].to_string(),
            method: caps["method"].to_string(),
            file: caps["file"].to_string(),
            line: caps["line"].parse().ok()?,
        });
    }
    let parsed = super::diagnostics::parse_jvm_diagnostics(line, None);
    let entry = parsed.entries.first()?;
    let path = entry.filename.clone()?;
    if entry.lnum == 0 {
        return None;
    }
    Some(ConsoleLocation::File {
        path,
        line: entry.lnum,
        column: entry.col,
    })
}

/// The package directory for a class name (`com.foo.Bar$Inner` -> `com/foo`).
fn package_dir(class: &str) -> PathBuf {
    let mut parts: Vec<&str> = class.split('.').collect();
    parts.pop(); // simple (possibly nested) class name
    parts.iter().collect()
}

/// Source directories checked before searching a tree.
const CONVENTIONAL_SOURCE_DIRS: &[&str] = &[
    "src/main/java",
    "src/main/kotlin",
    "src/test/java",
    "src/test/kotlin",
    "src",
    "",
];

/// File extensions a stack frame can name.
const SOURCE_EXTENSIONS: &[&str] = &["java", "kt", "kts", "scala", "groovy"];

/// Packages of the JDK, language runtimes and test/build frameworks. Frames
/// in them are never project files, so no search of the tree is worth it.
const LIBRARY_PACKAGES: &[&str] = &[
    "java.",
    "javax.",
    "jdk.",
    "sun.",
    "com.sun.",
    "kotlin.",
    "kotlinx.",
    "scala.",
    "groovy.",
    "org.codehaus.groovy.",
    "org.junit.",
    "org.opentest4j.",
    "org.testng.",
    "org.gradle.",
    "worker.org.gradle.",
    "org.apache.maven.",
];

/// Whether `class` belongs to the JDK or a well-known framework.
fn is_library_class(class: &str) -> bool {
    LIBRARY_PACKAGES.iter().any(|p| class.starts_with(p))
}

/// Source files under some roots by file name, from one walk per root
/// (honouring `.gitignore`, skipping hidden files).
struct SourceIndex {
    /// Per file name: (index of its root, path), in walk order.
    by_name: HashMap<OsString, Vec<(usize, PathBuf)>>,
    walks: usize,
}

impl SourceIndex {
    fn build(roots: &[PathBuf]) -> Self {
        let mut by_name: HashMap<OsString, Vec<(usize, PathBuf)>> = HashMap::new();
        for (root_index, root) in roots.iter().enumerate() {
            let walker = ignore::WalkBuilder::new(root)
                .max_depth(Some(10))
                .hidden(true)
                .build();
            for entry in walker.flatten() {
                let path = entry.path();
                let is_source = path
                    .extension()
                    .and_then(OsStr::to_str)
                    .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&ext));
                if !is_source || entry.file_type().is_some_and(|t| t.is_dir()) {
                    continue;
                }
                if let Some(name) = path.file_name() {
                    by_name
                        .entry(name.to_os_string())
                        .or_default()
                        .push((root_index, path.to_path_buf()));
                }
            }
        }
        Self {
            by_name,
            walks: roots.len(),
        }
    }

    /// The `file` whose path ends in `relative`: from the earliest root, and
    /// within it the one nearest to the top.
    fn find(&self, file: &str, relative: &Path) -> Option<PathBuf> {
        self.by_name
            .get(OsStr::new(file))?
            .iter()
            .filter(|(_, path)| path.ends_with(relative))
            .min_by_key(|(root, path)| (*root, path.components().count()))
            .map(|(_, path)| path.clone())
    }
}

/// Finds the source files of stack frames under `roots`, for any number of
/// frames: the conventional source directories first (cheap, exact), then an
/// index of the tree built on the first frame that needs it. Answers are
/// remembered, and frames from the JDK and well-known frameworks never
/// cause a search.
pub struct FrameResolver {
    roots: Vec<PathBuf>,
    index: Option<SourceIndex>,
    /// Answers by `<package path>/<file>`.
    found: HashMap<PathBuf, Option<PathBuf>>,
}

impl FrameResolver {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            index: None,
            found: HashMap::new(),
        }
    }

    /// The source file for a stack frame of `class` in `file`.
    pub fn resolve_frame(&mut self, class: &str, file: &str) -> Option<PathBuf> {
        let relative = package_dir(class).join(file);
        if let Some(known) = self.found.get(&relative) {
            return known.clone();
        }
        let path = self.lookup(class, file, &relative);
        self.found.insert(relative, path.clone());
        path
    }

    fn lookup(&mut self, class: &str, file: &str, relative: &Path) -> Option<PathBuf> {
        for root in &self.roots {
            for dir in CONVENTIONAL_SOURCE_DIRS {
                let candidate = root.join(dir).join(relative);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        if is_library_class(class) {
            return None;
        }
        let roots = &self.roots;
        self.index
            .get_or_insert_with(|| SourceIndex::build(roots))
            .find(file, relative)
    }

    /// Resolves a location to an absolute path plus 1-based line/column.
    pub fn resolve_location(
        &mut self,
        location: &ConsoleLocation,
        cwd: &Path,
    ) -> Option<(PathBuf, usize, usize)> {
        match location {
            ConsoleLocation::Frame {
                class, file, line, ..
            } => self.resolve_frame(class, file).map(|p| (p, *line, 1)),
            ConsoleLocation::File { path, line, column } => {
                let abs = if path.is_absolute() {
                    path.clone()
                } else {
                    cwd.join(path)
                };
                abs.is_file().then_some((abs, *line, (*column).max(1)))
            }
        }
    }

    /// Directory trees walked so far (each root at most once).
    pub fn walks(&self) -> usize {
        self.index.as_ref().map_or(0, |i| i.walks)
    }
}

/// Finds the source file for a stack frame under `roots`.
pub fn resolve_frame_source(class: &str, file: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    FrameResolver::new(roots.to_vec()).resolve_frame(class, file)
}

/// Finds the source file of `class` through the language server's
/// `workspace/symbol` (classes in other modules, libraries with attached
/// sources, JDK sources the server materialised).
pub async fn lookup_frame_source(
    manager: &crate::lsp::LspManager,
    language_id: &str,
    class: &str,
    file: &str,
) -> Result<PathBuf, String> {
    // `com.foo.Outer$Inner` -> query `Outer`, wanted package `com.foo`.
    let mut parts: Vec<&str> = class.split('.').collect();
    let simple = parts.pop().unwrap_or(class);
    let outer = simple.split('$').next().unwrap_or(simple).to_string();
    let package = parts.join(".");
    let symbols = manager
        .workspace_symbols(language_id, outer.clone())
        .await
        // No server for the language, or the request failed: the source is
        // simply not findable.
        .map_err(|_| format!("Source not found for {class} ({file})"))?;
    pick_frame_source(&symbols, &outer, &package, class, file)
        .ok_or_else(|| format!("Source not found for {class} ({file})"))
}

/// Chooses the symbol that is `class`: a path ending in `<package>/<file>`
/// beats a matching container package, which beats a same-named file.
#[allow(deprecated)] // SymbolInformation::container_name
fn pick_frame_source(
    symbols: &[lsp_types::SymbolInformation],
    outer: &str,
    package: &str,
    class: &str,
    file: &str,
) -> Option<PathBuf> {
    let wanted_tail = package_dir(class).join(file);
    let mut best: Option<(u8, PathBuf)> = None;
    for symbol in symbols {
        if symbol.name != outer {
            continue;
        }
        let Some(path) = crate::lsp::uri_to_file_path(&symbol.location.uri) else {
            continue;
        };
        let score = if path.ends_with(&wanted_tail) {
            3
        } else if !package.is_empty()
            && symbol
                .container_name
                .as_deref()
                .is_some_and(|c| c == package || c.starts_with(&format!("{package}.")))
        {
            2
        } else if path.file_name().is_some_and(|n| n == file) {
            1
        } else {
            continue;
        };
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, path));
        }
    }
    best.map(|(_, path)| path)
}

/// Resolves a location to an absolute path plus 1-based line/column.
pub fn resolve_location(
    location: &ConsoleLocation,
    cwd: &Path,
    source_roots: &[PathBuf],
) -> Option<(PathBuf, usize, usize)> {
    FrameResolver::new(source_roots.to_vec()).resolve_location(location, cwd)
}

#[cfg(test)]
mod tests {
    fn symbol(name: &str, container: Option<&str>, path: &str) -> lsp_types::SymbolInformation {
        #[allow(deprecated)]
        lsp_types::SymbolInformation {
            name: name.to_string(),
            kind: lsp_types::SymbolKind::CLASS,
            tags: None,
            deprecated: None,
            location: lsp_types::Location {
                uri: crate::lsp::uri_from_file_path(std::path::Path::new(path)).unwrap(),
                range: Default::default(),
            },
            container_name: container.map(str::to_string),
        }
    }

    #[test]
    fn frame_lookup_prefers_the_file_in_the_right_package() {
        let symbols = vec![
            symbol("List", Some("java.awt"), "/jdk/java/awt/List.java"),
            symbol("List", Some("java.util"), "/jdk/java/util/List.java"),
            symbol("Other", Some("java.util"), "/jdk/java/util/Other.java"),
        ];
        assert_eq!(
            pick_frame_source(&symbols, "List", "java.util", "java.util.List", "List.java"),
            Some(PathBuf::from("/jdk/java/util/List.java"))
        );
        // No package match by path: fall back to the container.
        let symbols = vec![symbol("Foo", Some("com.acme"), "/x/generated/Foo.kt")];
        assert_eq!(
            pick_frame_source(
                &symbols,
                "Foo",
                "com.acme",
                "com.acme.Foo$Inner",
                "Foo.java"
            ),
            Some(PathBuf::from("/x/generated/Foo.kt"))
        );
        assert_eq!(
            pick_frame_source(&symbols, "Foo", "org.other", "org.other.Foo", "Bar.java"),
            None
        );
    }

    use super::*;

    #[test]
    fn parses_plain_nested_and_module_frames() {
        assert_eq!(
            parse_console_location("\tat com.foo.Bar$Inner.baz(Bar.java:42)"),
            Some(ConsoleLocation::Frame {
                class: "com.foo.Bar$Inner".into(),
                method: "baz".into(),
                file: "Bar.java".into(),
                line: 42
            })
        );
        assert!(matches!(
            parse_console_location("\tat java.base/java.util.Objects.requireNonNull(Objects.java:233)"),
            Some(ConsoleLocation::Frame { class, line: 233, .. }) if class == "java.util.Objects"
        ));
        assert!(matches!(
            parse_console_location("\tat com.x.MainKt.main(Main.kt:5)"),
            Some(ConsoleLocation::Frame { line: 5, .. })
        ));
        assert!(matches!(
            parse_console_location("\tat com.x.Foo.<init>(Foo.java:9)"),
            Some(ConsoleLocation::Frame { method, .. }) if method == "<init>"
        ));
        assert!(matches!(
            parse_console_location("\tat com.x.Foo.lambda$main$0(Foo.java:9)"),
            Some(ConsoleLocation::Frame { method, .. }) if method == "lambda$main$0"
        ));
    }

    #[test]
    fn native_and_unknown_frames_are_not_jumpable() {
        assert_eq!(
            parse_console_location("\tat java.base/jdk.internal.reflect.NativeMethodAccessorImpl.invoke0(Native Method)"),
            None
        );
        assert_eq!(
            parse_console_location(
                "Exception in thread \"main\" java.lang.IllegalStateException: x"
            ),
            None
        );
        assert_eq!(
            parse_console_location("Caused by: java.io.IOException: nope"),
            None
        );
    }

    #[test]
    fn compiler_lines_are_jumpable() {
        assert_eq!(
            parse_console_location("/p/A.java:12: error: boom"),
            Some(ConsoleLocation::File {
                path: "/p/A.java".into(),
                line: 12,
                column: 0
            })
        );
        assert_eq!(
            parse_console_location("e: file:///p/A.kt:3:9 nope"),
            Some(ConsoleLocation::File {
                path: "/p/A.kt".into(),
                line: 3,
                column: 9
            })
        );
    }

    #[test]
    fn frames_resolve_through_source_roots() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("app/src/main/java/com/foo");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("Bar.java"), "class Bar {}").unwrap();
        let odd = dir.path().join("weird/layout/com/foo");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("Baz.java"), "class Baz {}").unwrap();

        let roots = vec![dir.path().join("app"), dir.path().to_path_buf()];
        let bar = resolve_frame_source("com.foo.Bar$Inner", "Bar.java", &roots).unwrap();
        assert!(bar.ends_with("app/src/main/java/com/foo/Bar.java"));
        let baz = resolve_frame_source("com.foo.Baz", "Baz.java", &roots).unwrap();
        assert!(baz.ends_with("weird/layout/com/foo/Baz.java"));
        assert!(resolve_frame_source("java.util.Objects", "Objects.java", &roots).is_none());
    }

    #[test]
    fn many_frames_cost_one_walk_and_library_frames_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let odd = root.join("weird/layout/com/foo");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("Baz.java"), "class Baz {}").unwrap();
        for i in 0..50 {
            let pkg = root.join(format!("modules/m{i}/p{i}"));
            std::fs::create_dir_all(&pkg).unwrap();
            std::fs::write(pkg.join("Other.java"), "class Other {}").unwrap();
        }

        let mut resolver = FrameResolver::new(vec![root.clone()]);
        for class in ["java.util.Objects", "org.junit.Assert", "kotlin.Unit"] {
            assert_eq!(resolver.resolve_frame(class, "X.java"), None);
        }
        assert_eq!(
            resolver.walks(),
            0,
            "library frames must not search the tree"
        );

        // Frames of code that is not in the project, 500 distinct classes
        // twice over, plus project frames among them.
        for round in 0..2 {
            for i in 0..500 {
                let class = format!("com.vendor.pkg{i}.Thing");
                assert_eq!(
                    resolver.resolve_frame(&class, "Thing.java"),
                    None,
                    "{round}"
                );
            }
            let baz = resolver.resolve_frame("com.foo.Baz", "Baz.java").unwrap();
            assert!(baz.ends_with("weird/layout/com/foo/Baz.java"));
            let other = resolver.resolve_frame("p7.Other", "Other.java").unwrap();
            assert!(other.ends_with("modules/m7/p7/Other.java"));
        }
        assert_eq!(resolver.walks(), 1);
    }

    #[test]
    fn the_nearest_match_in_the_earliest_root_wins() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        for path in [
            a.join("x/deep/er/com/foo"),
            a.join("y/com/foo"),
            b.join("z/com/foo"),
        ] {
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("Bar.java"), "class Bar {}").unwrap();
        }
        let mut resolver = FrameResolver::new(vec![a.clone(), b]);
        assert_eq!(
            resolver.resolve_frame("com.foo.Bar", "Bar.java"),
            Some(a.join("y/com/foo/Bar.java")),
            "earliest root first, then the shortest path"
        );
    }
}
